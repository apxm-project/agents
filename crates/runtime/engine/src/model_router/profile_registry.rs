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
