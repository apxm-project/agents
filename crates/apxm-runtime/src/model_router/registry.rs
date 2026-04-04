//! Model registry — loads model definitions from `~/.apxm/models.toml`.
//!
//! ## Config format
//!
//! ```toml
//! # ~/.apxm/models.toml
//!
//! [defaults]
//! model = "claude-sonnet-4-5"
//! backend = "anthropic"
//!
//! [[models]]
//! name = "claude-sonnet-4-5"
//! backend = "anthropic"
//! cost_per_1k_input = 0.003
//! cost_per_1k_output = 0.015
//! context_window = 200000
//! tags = ["production", "smart"]
//!
//! [[models]]
//! name = "claude-haiku-4-5"
//! backend = "anthropic"
//! cost_per_1k_input = 0.00025
//! cost_per_1k_output = 0.00125
//! context_window = 200000
//! tags = ["fast", "cheap"]
//!
//! [[models]]
//! name = "gpt-4o-mini"
//! backend = "openai"
//! cost_per_1k_input = 0.00015
//! cost_per_1k_output = 0.0006
//! context_window = 128000
//! tags = ["cheap", "fast"]
//!
//! [routing]
//! prefer_tags = ["production"]
//! fallback_tags = ["cheap"]
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

/// A single model entry from the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Context window size in tokens.
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
    dirs::home_dir().map(|home| home.join(".apxm").join("models.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_empty_registry() {
        let reg = ModelRegistry::new();
        assert!(reg.is_empty());
        assert!(reg.default_model().is_none());
    }

    #[test]
    fn test_register_and_get() {
        let reg = ModelRegistry::new();
        reg.register(ModelEntry {
            name: "test-model".to_string(),
            backend: "test-backend".to_string(),
            cost_per_1k_input: 0.001,
            cost_per_1k_output: 0.002,
            context_window: 128_000,
            tags: vec!["fast".to_string()],
            supports_thinking: false,
            max_output_tokens: None,
        });
        let entry = reg.get("test-model").unwrap();
        assert_eq!(entry.backend, "test-backend");
        assert!(!reg.is_empty());
    }

    #[test]
    fn test_tags_filter() {
        let reg = ModelRegistry::new();
        reg.register(ModelEntry {
            name: "cheap".to_string(),
            backend: "b".to_string(),
            cost_per_1k_input: 0.0001,
            cost_per_1k_output: 0.0002,
            context_window: 8_000,
            tags: vec!["cheap".to_string(), "fast".to_string()],
            supports_thinking: false,
            max_output_tokens: None,
        });
        reg.register(ModelEntry {
            name: "smart".to_string(),
            backend: "b".to_string(),
            cost_per_1k_input: 0.005,
            cost_per_1k_output: 0.015,
            context_window: 200_000,
            tags: vec!["production".to_string()],
            supports_thinking: true,
            max_output_tokens: None,
        });

        let fast = reg.models_with_tags(&["fast"]);
        assert_eq!(fast.len(), 1);
        assert_eq!(fast[0].name, "cheap");

        let production = reg.models_with_tags(&["production"]);
        assert_eq!(production.len(), 1);
        assert_eq!(production[0].name, "smart");
    }

    #[test]
    fn test_estimate_cost() {
        let reg = ModelRegistry::new();
        reg.register(ModelEntry {
            name: "m".to_string(),
            backend: "b".to_string(),
            cost_per_1k_input: 1.0,
            cost_per_1k_output: 2.0,
            context_window: 8_000,
            tags: vec![],
            supports_thinking: false,
            max_output_tokens: None,
        });
        let cost = reg.estimate_cost("m", 1000, 500).unwrap();
        assert!((cost - (1.0 + 1.0)).abs() < 1e-9); // 1.0 input + 1.0 output
    }

    #[test]
    fn test_load_from_toml() {
        let toml_content = r#"
[defaults]
model = "claude-sonnet-4-5"
backend = "anthropic"

[[models]]
name = "claude-sonnet-4-5"
backend = "anthropic"
cost_per_1k_input = 0.003
cost_per_1k_output = 0.015
context_window = 200000
tags = ["production", "smart"]

[[models]]
name = "gpt-4o-mini"
backend = "openai"
cost_per_1k_input = 0.00015
cost_per_1k_output = 0.0006
context_window = 128000
tags = ["cheap", "fast"]

[routing]
prefer_tags = ["production"]
fallback_tags = ["cheap"]
"#;

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(toml_content.as_bytes()).unwrap();

        let reg = ModelRegistry::new();
        reg.load_from_path(tmp.path()).unwrap();

        assert_eq!(reg.list().len(), 2);
        assert_eq!(reg.default_model().unwrap(), "claude-sonnet-4-5");
        assert_eq!(reg.default_backend().unwrap(), "anthropic");
        assert_eq!(reg.routing().prefer_tags, vec!["production"]);

        let cheap = reg.models_with_tags(&["cheap"]);
        assert_eq!(cheap.len(), 1);
        assert_eq!(cheap[0].name, "gpt-4o-mini");
    }

    #[test]
    fn test_cheapest() {
        let reg = ModelRegistry::new();
        reg.register(ModelEntry {
            name: "expensive".to_string(),
            backend: "b".to_string(),
            cost_per_1k_input: 0.01,
            cost_per_1k_output: 0.03,
            context_window: 200_000,
            tags: vec![],
            supports_thinking: false,
            max_output_tokens: None,
        });
        reg.register(ModelEntry {
            name: "cheap".to_string(),
            backend: "b".to_string(),
            cost_per_1k_input: 0.0001,
            cost_per_1k_output: 0.0002,
            context_window: 8_000,
            tags: vec![],
            supports_thinking: false,
            max_output_tokens: None,
        });
        let cheapest = reg.cheapest().unwrap();
        assert_eq!(cheapest.name, "cheap");
    }
}
