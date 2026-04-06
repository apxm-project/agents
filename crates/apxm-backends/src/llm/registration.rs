//! Typed registration and policy entities layered over `LLMRegistry`.
//!
//! These types keep registration dynamic while making the control plane
//! explicit enough for APXM runtimes and adapters.

use crate::llm::LLMRegistry;
use crate::llm::Provider;
use anyhow::Result;
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::constants::llm::config_keys;
use apxm_core::types::{AISOperationType, ModelInfo, ProviderProtocol};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue, json};
use std::collections::HashMap;

/// Model registration metadata attached to a backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRegistration {
    /// Canonical model identifier used at request time.
    pub id: String,
    /// Alternative names resolved to the canonical id.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Optional descriptive metadata for routing and inspection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<ModelInfo>,
}

/// Typed backend registration input for the LLM registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendRegistration {
    pub name: String,
    pub protocol: ProviderProtocol,
    pub api_key: String,
    #[serde(default, alias = "model")]
    pub default_model: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelRegistration>,
    pub endpoint: Option<String>,
    #[serde(default)]
    pub options: HashMap<String, String>,
    #[serde(default)]
    pub extra_headers: HashMap<String, String>,
}

impl BackendRegistration {
    fn primary_model_id(&self) -> Option<&str> {
        self.default_model
            .as_deref()
            .or_else(|| self.models.first().map(|model| model.id.as_str()))
    }

    fn backend_config_json(&self) -> Option<JsonValue> {
        let mut map = Map::new();

        if let Some(model) = self.primary_model_id() {
            map.insert(MODEL.to_string(), json!(model));
        }
        if let Some(endpoint) = &self.endpoint {
            map.insert(BASE_URL.to_string(), json!(endpoint));
        }
        for (key, value) in &self.options {
            map.insert(key.clone(), json!(value));
        }
        if !self.extra_headers.is_empty() {
            let headers = self
                .extra_headers
                .iter()
                .map(|(key, value)| (key.clone(), json!(value)))
                .collect::<Map<String, JsonValue>>();
            map.insert(config_keys::EXTRA_HEADERS.to_string(), JsonValue::Object(headers));
        }

        if map.is_empty() {
            None
        } else {
            Some(JsonValue::Object(map))
        }
    }

    /// Register this backend into the supplied registry.
    pub async fn register(&self, registry: &LLMRegistry) -> Result<()> {
        let provider =
            Provider::from_protocol(self.protocol, &self.api_key, self.backend_config_json())
                .await?;
        registry.register(self.name.clone(), provider)?;

        if let Some(model) = &self.default_model {
            registry.set_model_route(model.clone(), self.name.clone())?;
        }

        for model in &self.models {
            registry.set_model_route(model.id.clone(), self.name.clone())?;
            for alias in &model.aliases {
                registry.register_model_alias(alias.clone(), model.id.clone());
                registry.set_model_route(alias.clone(), self.name.clone())?;
            }
        }
        Ok(())
    }
}

/// Operation-level routing rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRoute {
    pub operation: AISOperationType,
    pub backend: Option<String>,
    pub model: Option<String>,
}

/// Named model alias with optional backend preference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAliasRegistration {
    pub alias: String,
    pub model: String,
    pub backend: Option<String>,
}

/// Backend fallback chain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackendFallback {
    pub backend: String,
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

/// Typed registry policy applied after backend registration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegistryPolicy {
    pub default_backend: Option<String>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub operation_routes: Vec<OperationRoute>,
    #[serde(default)]
    pub model_aliases: Vec<ModelAliasRegistration>,
    #[serde(default)]
    pub fallback_chains: Vec<BackendFallback>,
}

impl RegistryPolicy {
    /// Apply this policy to a populated registry.
    pub fn apply(&self, registry: &LLMRegistry) -> Result<()> {
        if let Some(default_backend) = &self.default_backend {
            registry.set_default(default_backend.clone())?;
        }
        if let Some(default_model) = &self.default_model {
            registry.set_default_model(default_model.clone());
        }

        for route in &self.operation_routes {
            if let Some(backend) = &route.backend {
                registry.set_operation_default(route.operation, backend.clone())?;
            }
            if let Some(model) = &route.model {
                registry.set_operation_model(route.operation, model.clone());
            }
        }

        for alias in &self.model_aliases {
            registry.register_model_alias(alias.alias.clone(), alias.model.clone());
            if let Some(backend) = &alias.backend {
                registry.set_model_route(alias.alias.clone(), backend.clone())?;
            }
        }

        for chain in &self.fallback_chains {
            registry.set_fallback(chain.backend.clone(), chain.fallbacks.clone())?;
        }

        Ok(())
    }
}
