//! Model Profile System (Plan B from model-profiles-design.md)
//!
//! Provides declarative capability abstractions for model selection without hardcoding
//! specific model names in graphs.
//!
//! ## Overview
//!
//! A `ModelProfile` defines:
//! - Semantic requirements (e.g., "reasoning-tier", "fast-draft")
//! - Prioritized candidate models
//! - Optional constraints (context window, cost ceiling)
//!
//! Runtime resolution happens via `ProfileRouter` which:
//! 1. Iterates candidates by priority (ascending: 1 → 2 → 3)
//! 2. Checks circuit-breaker health for each candidate
//! 3. Returns first healthy model
//!
//! ## Config Format
//!
//! `~/.apxm/model_profiles.toml`:
//! ```toml
//! [[profile]]
//! name = "reasoning-tier"
//! description = "High-capability reasoning models"
//! tags = ["reasoning", "analysis"]
//! min_context_window = 128000
//!
//!   [[profile.candidate]]
//!   model = "claude-opus-4-6"
//!   priority = 1
//!
//!   [[profile.candidate]]
//!   model = "gpt-4o"
//!   priority = 2
//! ```
//!
//! ## Graph Usage
//!
//! ```json
//! {
//!   "id": 1,
//!   "op": "ASK",
//!   "model_profile": "reasoning-tier",
//!   "attributes": {...}
//! }
//! ```

use serde::{Deserialize, Serialize};

/// A candidate model within a profile, ordered by priority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileCandidate {
    /// Model name (must exist in ModelRegistry at runtime).
    pub model: String,
    /// Priority (1 = highest priority, lower numbers tried first).
    pub priority: u32,
}

/// A model profile defining semantic requirements and prioritized candidates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelProfile {
    /// Profile identifier (e.g., "reasoning-tier", "fast-draft").
    pub name: String,
    /// Human-readable description of the profile's purpose.
    pub description: String,
    /// Searchable tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional minimum context window requirement (in tokens).
    #[serde(default)]
    pub min_context_window: Option<usize>,
    /// Optional maximum cost ceiling per 1K input tokens (USD).
    #[serde(default)]
    pub max_cost_per_1k_input: Option<f64>,
    /// Prioritized candidate models (runtime selects first healthy).
    #[serde(rename = "candidate", default)]
    pub candidates: Vec<ProfileCandidate>,
}

impl ModelProfile {
    /// Returns candidates sorted by priority (ascending: 1 → 2 → 3).
    pub fn candidates_by_priority(&self) -> Vec<&ProfileCandidate> {
        let mut sorted = self.candidates.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|c| c.priority);
        sorted
    }

    /// Returns true if the profile has at least one candidate.
    pub fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }
}

/// Top-level structure for `~/.apxm/model_profiles.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelProfilesConfig {
    /// List of profiles defined in the config.
    #[serde(default)]
    pub profile: Vec<ModelProfile>,
}

impl ModelProfilesConfig {
    /// Returns a reference to the profiles list.
    pub fn profiles(&self) -> &[ModelProfile] {
        &self.profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_candidate_serialization() {
        let candidate = ProfileCandidate {
            model: "claude-opus-4-6".to_string(),
            priority: 1,
        };
        let toml = toml::to_string(&candidate).unwrap();
        assert!(toml.contains("model = \"claude-opus-4-6\""));
        assert!(toml.contains("priority = 1"));
    }

    #[test]
    fn test_model_profile_serialization() {
        let profile = ModelProfile {
            name: "reasoning-tier".to_string(),
            description: "High-capability reasoning models".to_string(),
            tags: vec!["reasoning".to_string(), "analysis".to_string()],
            min_context_window: Some(128_000),
            max_cost_per_1k_input: None,
            candidates: vec![
                ProfileCandidate {
                    model: "claude-opus-4-6".to_string(),
                    priority: 1,
                },
                ProfileCandidate {
                    model: "gpt-4o".to_string(),
                    priority: 2,
                },
            ],
        };

        let toml = toml::to_string(&profile).unwrap();
        assert!(toml.contains("name = \"reasoning-tier\""));
        assert!(toml.contains("claude-opus-4-6"));
        assert!(toml.contains("gpt-4o"));
    }

    #[test]
    fn test_model_profiles_config_deserialization() {
        // TOML array-of-tables: use inline arrays for candidates within each [[profile]] block.
        // Nested [[profile.candidate]] tables can't span multiple [[profile]] blocks in TOML.
        let toml_content = r#"
[[profile]]
name = "reasoning-tier"
description = "High-capability reasoning models"
tags = ["reasoning", "analysis"]
min_context_window = 128000
candidate = [
  { model = "claude-opus-4-6", priority = 1 },
  { model = "gpt-4o", priority = 2 },
]

[[profile]]
name = "fast-draft"
description = "Fast, cost-effective models"
tags = ["fast", "cheap"]
max_cost_per_1k_input = 0.0003
candidate = [
  { model = "claude-sonnet-4-5@20250929", priority = 1 },
]
"#;

        let config: ModelProfilesConfig = toml::from_str(toml_content).unwrap();
        eprintln!("Parsed config: {:?}", config);
        eprintln!("Number of profiles: {}", config.profile.len());
        assert_eq!(config.profile.len(), 2);

        let reasoning = &config.profile[0];
        assert_eq!(reasoning.name, "reasoning-tier");
        assert_eq!(reasoning.candidates.len(), 2);
        assert_eq!(reasoning.min_context_window, Some(128_000));

        let draft = &config.profile[1];
        assert_eq!(draft.name, "fast-draft");
        assert_eq!(draft.max_cost_per_1k_input, Some(0.0003));
    }

    #[test]
    fn test_candidates_by_priority_sorting() {
        let profile = ModelProfile {
            name: "test".to_string(),
            description: "Test".to_string(),
            tags: vec![],
            min_context_window: None,
            max_cost_per_1k_input: None,
            candidates: vec![
                ProfileCandidate {
                    model: "c".to_string(),
                    priority: 3,
                },
                ProfileCandidate {
                    model: "a".to_string(),
                    priority: 1,
                },
                ProfileCandidate {
                    model: "b".to_string(),
                    priority: 2,
                },
            ],
        };

        let sorted = profile.candidates_by_priority();
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].model, "a");
        assert_eq!(sorted[1].model, "b");
        assert_eq!(sorted[2].model, "c");
    }

    #[test]
    fn test_has_candidates() {
        let empty_profile = ModelProfile {
            name: "empty".to_string(),
            description: "No candidates".to_string(),
            tags: vec![],
            min_context_window: None,
            max_cost_per_1k_input: None,
            candidates: vec![],
        };
        assert!(!empty_profile.has_candidates());

        let full_profile = ModelProfile {
            name: "full".to_string(),
            description: "Has candidates".to_string(),
            tags: vec![],
            min_context_window: None,
            max_cost_per_1k_input: None,
            candidates: vec![ProfileCandidate {
                model: "test".to_string(),
                priority: 1,
            }],
        };
        assert!(full_profile.has_candidates());
    }
}
