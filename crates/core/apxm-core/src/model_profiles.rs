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

