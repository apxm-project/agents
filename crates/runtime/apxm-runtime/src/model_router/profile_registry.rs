//! Profile Registry — loads model profiles from `~/.apxm/model_profiles.toml`.
//!
//! ## Config format
//!
//! ```toml
//! # ~/.apxm/model_profiles.toml
//!
//! [[profile]]
//! name = "reasoning-tier"
//! description = "High-capability reasoning models for complex analysis"
//! tags = ["reasoning", "analysis", "research"]
//! min_context_window = 128000
//!
//!   [[profile.candidate]]
//!   model = "claude-opus-4-6"
//!   priority = 1
//!
//!   [[profile.candidate]]
//!   model = "gpt-4o"
//!   priority = 2
//!
//! [[profile]]
//! name = "fast-draft"
//! description = "Fast, cost-effective models for drafting and iteration"
//! tags = ["draft", "fast", "cheap"]
//! max_cost_per_1k_input = 0.0003
//!
//!   [[profile.candidate]]
//!   model = "claude-sonnet-4-5@20250929"
//!   priority = 1
//! ```

use apxm_core::model_profiles::{ModelProfile, ModelProfilesConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

/// In-memory registry of model profiles, loaded from disk.
pub struct ProfileRegistry {
    profiles: Arc<RwLock<HashMap<String, ModelProfile>>>,
}

impl ProfileRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        ProfileRegistry {
            profiles: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load from `~/.apxm/model_profiles.toml`, ignoring a missing file gracefully.
    pub fn load_from_default_path() -> Self {
        let registry = Self::new();
        if let Some(path) = default_profiles_path() {
            if path.exists() {
                if let Err(e) = registry.load_from_file(&path) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to load model_profiles.toml; using empty registry"
                    );
                }
            } else {
                tracing::debug!(
                    path = %path.display(),
                    "model_profiles.toml not found; using empty registry"
                );
            }
        }
        registry
    }

    /// Load from a specific TOML file path.
    pub fn load_from_file(
        &self,
        path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let content = std::fs::read_to_string(path)?;
        let parsed: ModelProfilesConfig = toml::from_str(&content)?;

        let mut profiles = self.profiles.write();
        profiles.clear();
        for profile in parsed.profile {
            profiles.insert(profile.name.clone(), profile);
        }

        tracing::info!(
            path = %path.display(),
            profile_count = profiles.len(),
            "Loaded model profile registry"
        );
        Ok(())
    }

    /// Look up a profile by name.
    pub fn get(&self, name: &str) -> Option<ModelProfile> {
        self.profiles.read().get(name).cloned()
    }

    /// List all registered profiles.
    pub fn list(&self) -> Vec<ModelProfile> {
        self.profiles.read().values().cloned().collect()
    }

    /// Register a profile programmatically (useful for testing).
    pub fn register(&self, profile: ModelProfile) {
        self.profiles.write().insert(profile.name.clone(), profile);
    }

    /// Returns true if the registry has no profiles loaded.
    pub fn is_empty(&self) -> bool {
        self.profiles.read().is_empty()
    }
}

impl Default for ProfileRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the default path for the model profiles config file.
fn default_profiles_path() -> Option<PathBuf> {
    // Scope to `$APXM_HOME` (falling back to `~/.apxm`), matching the backend
    // store and models registry so all per-home model config shares one root.
    Some(apxm_core::env::apxm_home().join("model_profiles.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::model_profiles::{ModelProfile, ProfileCandidate};
    use std::io::Write;

    #[test]
    fn test_empty_registry() {
        let reg = ProfileRegistry::new();
        assert!(reg.is_empty());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_register_and_get() {
        let reg = ProfileRegistry::new();
        let profile = ModelProfile {
            name: "test-profile".to_string(),
            description: "Test profile".to_string(),
            tags: vec!["test".to_string()],
            min_context_window: None,
            max_cost_per_1k_input: None,
            candidates: vec![ProfileCandidate {
                model: "test-model".to_string(),
                priority: 1,
            }],
        };

        reg.register(profile.clone());
        let retrieved = reg.get("test-profile").unwrap();
        assert_eq!(retrieved.name, "test-profile");
        assert_eq!(retrieved.candidates.len(), 1);
        assert!(!reg.is_empty());
    }

    #[test]
    fn test_list_profiles() {
        let reg = ProfileRegistry::new();
        reg.register(ModelProfile {
            name: "profile1".to_string(),
            description: "First".to_string(),
            tags: vec![],
            min_context_window: None,
            max_cost_per_1k_input: None,
            candidates: vec![],
        });
        reg.register(ModelProfile {
            name: "profile2".to_string(),
            description: "Second".to_string(),
            tags: vec![],
            min_context_window: None,
            max_cost_per_1k_input: None,
            candidates: vec![],
        });

        let list = reg.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_load_from_toml() {
        let toml_content = r#"
[[profile]]
name = "reasoning-tier"
description = "High-capability reasoning models"
tags = ["reasoning", "analysis"]
min_context_window = 128000

  [[profile.candidate]]
  model = "claude-opus-4-6"
  priority = 1

  [[profile.candidate]]
  model = "gpt-4o"
  priority = 2

[[profile]]
name = "fast-draft"
description = "Fast, cost-effective models"
tags = ["fast", "cheap"]
max_cost_per_1k_input = 0.0003

  [[profile.candidate]]
  model = "claude-sonnet-4-5@20250929"
  priority = 1
"#;

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(toml_content.as_bytes()).unwrap();

        let reg = ProfileRegistry::new();
        reg.load_from_file(tmp.path()).unwrap();

        assert_eq!(reg.list().len(), 2);

        let reasoning = reg.get("reasoning-tier").unwrap();
        assert_eq!(reasoning.description, "High-capability reasoning models");
        assert_eq!(reasoning.candidates.len(), 2);
        assert_eq!(reasoning.min_context_window, Some(128_000));

        let draft = reg.get("fast-draft").unwrap();
        assert_eq!(draft.candidates.len(), 1);
        assert_eq!(draft.max_cost_per_1k_input, Some(0.0003));
    }

    #[test]
    fn test_profiles_cleared_on_reload() {
        let reg = ProfileRegistry::new();
        reg.register(ModelProfile {
            name: "old-profile".to_string(),
            description: "Old".to_string(),
            tags: vec![],
            min_context_window: None,
            max_cost_per_1k_input: None,
            candidates: vec![],
        });

        assert!(reg.get("old-profile").is_some());

        let toml_content = r#"
[[profile]]
name = "new-profile"
description = "New"
"#;

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(toml_content.as_bytes()).unwrap();

        reg.load_from_file(tmp.path()).unwrap();

        // Old profile should be cleared
        assert!(reg.get("old-profile").is_none());
        // New profile should exist
        assert!(reg.get("new-profile").is_some());
    }
}
