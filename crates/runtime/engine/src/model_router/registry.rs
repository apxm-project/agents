//! Model registry — loads model definitions from `~/.apxm/models.toml`.
//!
//! ## Config format
//!
//! ```toml
//! # ~/.apxm/models.toml
//!
//! [defaults]
//! model = "production-model"
//! backend = "production-backend"
//!
//! [[models]]
//! name = "production-model"
//! backend = "production-backend"
//! cost_per_1k_input = 0.003
//! cost_per_1k_output = 0.015
//! context_window = 200000
//! tags = ["production", "smart"]
//! supports_vision = true
//! supports_tools = true
//! supports_json = true
//! quality_tier = 9
//!
//! [[models]]
//! name = "fast-model"
//! backend = "fast-backend"
//! cost_per_1k_input = 0.00025
//! cost_per_1k_output = 0.00125
//! context_window = 200000
//! tags = ["fast", "cheap"]
//! supports_json = true
//! quality_tier = 3
//!
//! [[models]]
//! name = "cheap-model"
//! backend = "cheap-backend"
//! cost_per_1k_input = 0.00015
//! cost_per_1k_output = 0.0006
//! context_window = 128000
//! tags = ["cheap", "fast"]
//! quality_tier = 1
//! local = true
//!
//! [routing]
//! # Optional global optimization target for the pool: "cost" | "latency"
//! # | "quality" | "balanced". When set (and no per-operation policy
//! # applies), the router ranks the price/capability table by this target.
//! # When omitted (or "balanced") the prefer_tags/fallback_tags
//! # behaviour is used.
//! target = "cost"
//! prefer_tags = ["production"]
//! fallback_tags = ["cheap"]
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use super::RoutingTarget;

/// A single model entry from the registry — one row of the price/capability
/// table loaded from `models.toml`.
///
/// `Default` is derived so new columns can be added without breaking the many
/// struct-literal construction sites in tests (`..Default::default()`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Model identifier (as sent to the API).
    pub name: String,
    /// Backend name (matches a registered LLMRegistry backend).
    pub backend: String,
    /// Cost per 1,000 input tokens (USD).
    #[serde(default)]
    pub cost_per_1k_input: f64,
    /// Cost per 1,000 output tokens (USD).
    #[serde(default)]
    pub cost_per_1k_output: f64,
    /// Context window size in tokens. `0` means "unknown / unconstrained" and
    /// disables the context-fit filter for this model.
    #[serde(default)]
    pub context_window: usize,
    /// Tags for routing policy (e.g. "fast", "cheap", "production").
    #[serde(default)]
    pub tags: Vec<String>,
    /// Whether this model supports extended thinking.
    #[serde(default)]
    pub supports_thinking: bool,
    /// Maximum output tokens supported.
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
    /// Whether this model accepts image inputs (multimodal vision).
    #[serde(default)]
    pub supports_vision: bool,
    /// Whether this model supports tool / function calling.
    #[serde(default)]
    pub supports_tools: bool,
    /// Whether this model supports structured (JSON / schema) output.
    #[serde(default)]
    pub supports_json: bool,
    /// Operator-supplied quality prior (higher = stronger). Used to rank for
    /// `RoutingTarget::Quality`. This is a static, hand-curated number, not a
    /// learned/measured score.
    #[serde(default)]
    pub quality_tier: u8,
    /// Whether this model runs locally / on-prem (data does not leave the
    /// host). Used to satisfy a privacy constraint on the request.
    #[serde(default)]
    pub local: bool,
}

impl ModelEntry {
    /// Returns true if the entry has all the given tags.
    pub fn has_tags(&self, tags: &[&str]) -> bool {
        tags.iter().all(|t| self.tags.iter().any(|mt| mt == t))
    }

    /// Estimate cost of a request.
    pub fn estimate_cost(&self, input_tokens: usize, output_tokens: usize) -> f64 {
        let input_cost = (input_tokens as f64 / 1000.0) * self.cost_per_1k_input;
        let output_cost = (output_tokens as f64 / 1000.0) * self.cost_per_1k_output;
        input_cost + output_cost
    }

    /// Returns true if this model can hold a request of the given estimated
    /// input + output token budget.
    ///
    /// A `context_window` of `0` is treated as "unknown" and always passes, so
    /// an unconfigured table never filters every candidate out. When
    /// `max_output_tokens` is set, the requested output must also fit.
    pub fn fits_context(&self, input_tokens: usize, output_tokens: usize) -> bool {
        if self.context_window != 0 && input_tokens + output_tokens > self.context_window {
            return false;
        }
        if let Some(max_out) = self.max_output_tokens {
            if output_tokens > max_out {
                return false;
            }
        }
        true
    }
}

/// Default settings from the config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefaultsConfig {
    /// Default model name.
    pub model: Option<String>,
    /// Default backend name.
    pub backend: Option<String>,
}

/// Routing policy from the config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Optional global optimization target for the pool. When set (and no
    /// per-operation policy applies), the router ranks the price/capability
    /// table by this target instead of using `prefer_tags`. `None` (or
    /// `Balanced`) keeps the tag-based behaviour.
    #[serde(default)]
    pub target: Option<RoutingTarget>,
    /// Tags to prefer when selecting a model (ordered by priority).
    #[serde(default)]
    pub prefer_tags: Vec<String>,
    /// Tags to use for fallback when preferred models are unavailable.
    #[serde(default)]
    pub fallback_tags: Vec<String>,
}

/// Top-level structure of `~/.apxm/models.toml`.
#[derive(Debug, Default, Deserialize)]
struct ModelsToml {
    #[serde(default)]
    defaults: DefaultsConfig,
    #[serde(default)]
    models: Vec<ModelEntry>,
    #[serde(default)]
    routing: RoutingConfig,
}

/// In-memory model registry, optionally loaded from disk.
pub struct ModelRegistry {
    models: Arc<RwLock<HashMap<String, ModelEntry>>>,
    defaults: Arc<RwLock<DefaultsConfig>>,
    routing: Arc<RwLock<RoutingConfig>>,
}

impl ModelRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        ModelRegistry {
            models: Arc::new(RwLock::new(HashMap::new())),
            defaults: Arc::new(RwLock::new(DefaultsConfig::default())),
            routing: Arc::new(RwLock::new(RoutingConfig::default())),
        }
    }

    /// Load from `~/.apxm/models.toml`, ignoring a missing file gracefully.
    pub fn load_from_default_path() -> Self {
        let registry = Self::new();
        if let Some(path) = default_models_path() {
            if path.exists() {
                if let Err(e) = registry.load_from_path(&path) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to load models.toml; using empty registry"
                    );
                }
            } else {
                tracing::debug!(
                    path = %path.display(),
                    "models.toml not found; using empty registry"
                );
            }
        }
        registry
    }

    /// Load from a specific TOML file path.
    pub fn load_from_path(
        &self,
        path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let content = std::fs::read_to_string(path)?;
        let parsed: ModelsToml = toml::from_str(&content)?;

        let mut models = self.models.write();
        models.clear();
        for entry in parsed.models {
            models.insert(entry.name.clone(), entry);
        }
        *self.defaults.write() = parsed.defaults;
        *self.routing.write() = parsed.routing;

        tracing::info!(
            path = %path.display(),
            model_count = models.len(),
            "Loaded model registry"
        );
        Ok(())
    }

    /// Register a model entry programmatically.
    pub fn register(&self, entry: ModelEntry) {
        self.models.write().insert(entry.name.clone(), entry);
    }

    /// Look up a model by name.
    pub fn get(&self, name: &str) -> Option<ModelEntry> {
        self.models.read().get(name).cloned()
    }

    /// List all registered models.
    pub fn list(&self) -> Vec<ModelEntry> {
        self.models.read().values().cloned().collect()
    }

    /// Returns all models matching the given tags.
    pub fn models_with_tags(&self, tags: &[&str]) -> Vec<ModelEntry> {
        self.models
            .read()
            .values()
            .filter(|m| m.has_tags(tags))
            .cloned()
            .collect()
    }

    /// Get the default model name (from config or first registered).
    pub fn default_model(&self) -> Option<String> {
        self.defaults
            .read()
            .model
            .clone()
            .or_else(|| self.models.read().values().next().map(|m| m.name.clone()))
    }

    /// Get the default backend name from config.
    pub fn default_backend(&self) -> Option<String> {
        self.defaults.read().backend.clone()
    }

    /// Get the routing policy.
    pub fn routing(&self) -> RoutingConfig {
        self.routing.read().clone()
    }

    /// Find the cheapest model (by output token cost).
    pub fn cheapest(&self) -> Option<ModelEntry> {
        self.models
            .read()
            .values()
            .min_by(|a, b| {
                a.cost_per_1k_output
                    .partial_cmp(&b.cost_per_1k_output)
                    .unwrap()
            })
            .cloned()
    }

    /// Find the model with the largest context window.
    pub fn largest_context(&self) -> Option<ModelEntry> {
        self.models
            .read()
            .values()
            .max_by_key(|m| m.context_window)
            .cloned()
    }

    /// Estimate cost for a named model.
    pub fn estimate_cost(
        &self,
        model_name: &str,
        input_tokens: usize,
        output_tokens: usize,
    ) -> Option<f64> {
        self.get(model_name)
            .map(|m| m.estimate_cost(input_tokens, output_tokens))
    }

    /// Returns true if the registry has no models loaded.
    pub fn is_empty(&self) -> bool {
        self.models.read().is_empty()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the default path for the models config file.
fn default_models_path() -> Option<PathBuf> {
    // Scope to `$APXM_HOME` (falling back to `~/.apxm`) so this registry lives
    // under the same root as the backend store (`config.toml`); a relocated
    // server home must not split its model roster across two directories.
    Some(apxm_core::env::apxm_home().join("models.toml"))
}
