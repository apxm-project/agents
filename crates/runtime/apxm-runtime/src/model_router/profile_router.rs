//! Profile Router — selects healthy model from profile candidates using circuit breakers.
//!
//! The ProfileRouter bridges semantic profiles and runtime health checks:
//!
//! 1. User specifies `model_profile` in graph node (e.g., "reasoning-tier")
//! 2. ProfileRouter loads profile from ProfileRegistry
//! 3. Iterates candidates by priority (1 → 2 → 3)
//! 4. For each candidate, checks `ModelRouter.is_model_healthy()`
//! 5. Returns first healthy candidate's model name
//!
//! ## Example
//!
//! ```rust,ignore
//! use apxm_runtime::model_router::{ModelRouter, ProfileRegistry, ProfileRouter};
//! use std::sync::Arc;
//!
//! let model_router = Arc::new(ModelRouter::new(/* ... */));
//! let profile_registry = Arc::new(ProfileRegistry::load_from_default_path());
//! let router = ProfileRouter::new(&profile_registry, &model_router);
//!
//! let model_name = router.select_from_profile("reasoning-tier")?;
//! // Returns "claude-opus-4-6" if healthy, else "gpt-4o", etc.
//! # Ok::<(), anyhow::Error>(())
//! ```

use super::ModelRouter;
use super::ProfileRegistry;
use super::registry::ModelRegistry;
use anyhow::Result;

/// Router that selects healthy models from profiles based on runtime circuit-breaker state.
pub struct ProfileRouter<'a> {
    profile_registry: &'a ProfileRegistry,
    model_router: &'a ModelRouter,
    model_registry: &'a ModelRegistry,
}

impl<'a> ProfileRouter<'a> {
    /// Create a new profile router.
    ///
    /// # Arguments
    /// * `profile_registry` - Registry of model profiles
    /// * `model_router` - Model router with circuit-breaker state
    pub fn new(
        profile_registry: &'a ProfileRegistry,
        model_router: &'a ModelRouter,
        model_registry: &'a ModelRegistry,
    ) -> Self {
        ProfileRouter {
            profile_registry,
            model_router,
            model_registry,
        }
    }

    /// Select a healthy model from the given profile.
    ///
    /// Iterates candidates by priority (ascending: 1 → 2 → 3) and returns the first
    /// model name whose backend has a healthy circuit breaker.
    ///
    /// # Arguments
    /// * `profile_name` - Name of the profile to select from
    ///
    /// # Returns
    /// * `Ok(String)` - Model name of the first healthy candidate
    /// * `Err` - If profile not found or no candidates are healthy
    ///
    /// # Errors
    /// Returns an error if:
    /// - Profile does not exist in the registry
    /// - Profile has no candidates
    /// - All candidates' backends have open circuit breakers
    pub fn select_from_profile(&self, profile_name: &str) -> Result<String> {
        let profile = self
            .profile_registry
            .get(profile_name)
            .ok_or_else(|| anyhow::anyhow!("Profile '{}' not found in registry", profile_name))?;

        if !profile.has_candidates() {
            anyhow::bail!("Profile '{}' has no candidates configured", profile_name);
        }

        // Iterate candidates by priority (sorted ascending: 1 → 2 → 3)
        for candidate in profile.candidates_by_priority() {
            // Check if the model exists in the model registry
            if let Some(model_entry) = self.model_registry.get(&candidate.model) {
                // Check if the backend is healthy via circuit breaker
                if self.model_router.is_model_healthy(&candidate.model) {
                    tracing::debug!(
                        profile = %profile_name,
                        model = %candidate.model,
                        priority = candidate.priority,
                        backend = %model_entry.backend,
                        "Selected healthy candidate from profile"
                    );
                    return Ok(candidate.model.clone());
                } else {
                    tracing::debug!(
                        profile = %profile_name,
                        model = %candidate.model,
                        priority = candidate.priority,
                        backend = %model_entry.backend,
                        "Skipping unhealthy candidate"
                    );
                }
            } else {
                tracing::warn!(
                    profile = %profile_name,
                    model = %candidate.model,
                    priority = candidate.priority,
                    "Candidate model not found in model registry"
                );
            }
        }

        anyhow::bail!(
            "No healthy candidates available for profile '{}' (all {} candidates unavailable)",
            profile_name,
            profile.candidates.len()
        )
    }
}
