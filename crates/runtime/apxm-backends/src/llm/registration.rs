//! Typed registration and policy entities layered over `LLMRegistry`.
//!
//! These types keep registration dynamic while making the control plane
//! explicit enough for APXM runtimes and adapters.

use crate::llm::LLMRegistry;
use crate::llm::Provider;
use anyhow::{Result, anyhow};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::constants::llm::config_keys;
use apxm_core::types::{
    AISOperationType, BackendConfig, BackendType, ModelInfo, ProviderProtocol,
    normalize_endpoint_for_protocol,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue, json};
use std::collections::HashMap;
use std::env;

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
    /// Whether the model supports extended thinking/reasoning. `None` means
    /// "use backend default". When `Some(false)` the OpenAI-compatible backend
    /// will inject `chat_template_kwargs.enable_thinking = false` so vLLM
    /// suppresses Qwen3 `<think>` blocks. Sourced from `ModelConfig.supports_thinking`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    /// Whether the model accepts an explicit custom `temperature` field.
    /// `Some(false)` means the OpenAI-compatible adapter must omit
    /// `temperature` and rely on the provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_custom_temperature: Option<bool>,
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
    /// Whether the backend accepts `tool_choice="auto"`. `None` means use
    /// the backend's default (typically `true`). See `BackendConfig.auto_tool_choice`.
    #[serde(default)]
    pub auto_tool_choice: Option<bool>,
    /// vLLM-only: hard-fail at `health_check` if the server doesn't expose
    /// `/v1/apxm/*`. `None`/`Some(true)` mean fail-fast; `Some(false)` opts
    /// out (allows stock vLLM). See `BackendConfig.require_apxm_endpoints`.
    #[serde(default)]
    pub require_apxm_endpoints: Option<bool>,
}

impl BackendRegistration {
    pub fn from_backend_config(backend: &BackendConfig) -> Result<Self> {
        let api_key = match backend.api_key.as_deref() {
            Some(key) => resolve_env_reference(key, "api_key", &backend.name)?,
            None if backend.backend_type == BackendType::Local
                || backend.protocol == ProviderProtocol::Ollama =>
            {
                String::new()
            }
            None => {
                return Err(anyhow!(
                    "Missing API key for backend '{}'. Set `api_key` or use `env:VAR`.",
                    backend.name
                ));
            }
        };

        let endpoint = backend
            .endpoint
            .as_deref()
            .map(|value| resolve_env_reference(value, "endpoint", &backend.name))
            .transpose()?
            .map(|value| normalize_endpoint_for_protocol(backend.protocol, &value));

        let extra_headers = backend
            .headers
            .iter()
            .map(|(key, value)| {
                let resolved = resolve_env_reference(value, key, &backend.name)?;
                Ok((key.clone(), resolved))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        let models = backend
            .models
            .iter()
            .map(|model| ModelRegistration {
                id: model.id.clone(),
                aliases: model.aliases.clone(),
                info: Some(ModelInfo {
                    id: model.id.clone(),
                    name: model.id.clone(),
                    context_window: model.context_window,
                    supports_vision: model.supports_vision,
                    supports_functions: model.supports_functions,
                }),
                supports_thinking: Some(model.supports_thinking),
                supports_custom_temperature: model.supports_custom_temperature,
            })
            .collect();

        Ok(Self {
            name: backend.name.clone(),
            protocol: backend.protocol,
            api_key,
            default_model: None,
            models,
            endpoint,
            options: HashMap::new(),
            extra_headers,
            auto_tool_choice: backend.auto_tool_choice,
            require_apxm_endpoints: backend.require_apxm_endpoints,
        })
    }

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
            map.insert(
                config_keys::EXTRA_HEADERS.to_string(),
                JsonValue::Object(headers),
            );
        }
        if let Some(auto_tool_choice) = self.auto_tool_choice {
            map.insert(
                config_keys::AUTO_TOOL_CHOICE.to_string(),
                json!(auto_tool_choice),
            );
        }
        if let Some(require_apxm_endpoints) = self.require_apxm_endpoints {
            map.insert(
                config_keys::REQUIRE_APXM_ENDPOINTS.to_string(),
                json!(require_apxm_endpoints),
            );
        }

        // Forward per-model capability flags that backends consult at request
        // time. Only emit entries whose flags actually differ from the default
        // so we don't bloat the JSON for the common case.
        let model_entries: Vec<JsonValue> = self
            .models
            .iter()
            .filter_map(|m| {
                let mut entry = Map::new();
                entry.insert("id".to_string(), json!(m.id));
                if let Some(supports_thinking) = m.supports_thinking {
                    entry.insert(
                        config_keys::SUPPORTS_THINKING.to_string(),
                        json!(supports_thinking),
                    );
                }
                if let Some(supports_custom_temperature) = m.supports_custom_temperature {
                    entry.insert(
                        config_keys::SUPPORTS_CUSTOM_TEMPERATURE.to_string(),
                        json!(supports_custom_temperature),
                    );
                }
                (entry.len() > 1).then_some(JsonValue::Object(entry))
            })
            .collect();
        if !model_entries.is_empty() {
            map.insert(config_keys::MODELS.to_string(), json!(model_entries));
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

fn resolve_env_reference(value: &str, field: &str, backend_name: &str) -> Result<String> {
    if let Some(var_name) = value.strip_prefix("env:") {
        env::var(var_name).map_err(|_| {
            anyhow!(
                "Environment variable '{}' not set for {} in backend '{}'",
                var_name,
                field,
                backend_name
            )
        })
    } else {
        Ok(value.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::{BackendType, ModelConfig};

    #[test]
    fn backend_registration_from_vllm_backend_config_preserves_backend_model_split() {
        let backend = BackendConfig {
            name: "vllm-fork".to_string(),
            backend_type: BackendType::Local,
            protocol: ProviderProtocol::Vllm,
            endpoint: Some("http://localhost:8916/v1".to_string()),
            api_key: None,
            headers: HashMap::from([("x-tenant".to_string(), "lab".to_string())]),
            models: vec![ModelConfig {
                id: "DataPilot/ArrowMint-Gemma3-4B-ChocoMint-instruct-v0.2".to_string(),
                aliases: vec!["gemma-smoke".to_string()],
                context_window: 8192,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                supports_vision: false,
                supports_functions: true,
                supports_thinking: false,
                supports_custom_temperature: Some(false),
                max_output_tokens: Some(2048),
                tags: Vec::new(),
            }],
            docker: None,
            auto_tool_choice: Some(false),
            require_apxm_endpoints: None,
        };

        let registration =
            BackendRegistration::from_backend_config(&backend).expect("registration builds");

        assert_eq!(registration.name, "vllm-fork");
        assert_eq!(registration.protocol, ProviderProtocol::Vllm);
        assert_eq!(
            registration.endpoint.as_deref(),
            Some("http://localhost:8916/v1")
        );
        assert_eq!(registration.api_key, "");
        assert_eq!(
            registration.models.first().map(|model| model.id.as_str()),
            Some("DataPilot/ArrowMint-Gemma3-4B-ChocoMint-instruct-v0.2")
        );
        assert_eq!(
            registration
                .models
                .first()
                .map(|model| model.aliases.clone()),
            Some(vec!["gemma-smoke".to_string()])
        );
        assert_eq!(registration.auto_tool_choice, Some(false));
        assert_eq!(
            registration
                .extra_headers
                .get("x-tenant")
                .map(String::as_str),
            Some("lab")
        );
    }
}
