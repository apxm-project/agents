//! Typed registration and policy entities layered over `LLMRegistry`.
//!
//! These types keep registration dynamic while making the control plane
//! explicit enough for APXM runtimes and adapters.

use crate::llm::LLMRegistry;
use crate::llm::Provider;
use crate::llm::backends::BackendConfigurationError;
use crate::llm::wire::config_keys;
use crate::llm::{
    BackendConfig, BackendType, ProviderProtocol, normalize_anthropic_gateway_endpoint,
    normalize_endpoint_for_protocol,
};
use anyhow::{Result, anyhow};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::types::{AISOperationType, ModelInfo};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue, json};
use std::collections::HashMap;
use std::env;
use std::fmt::Write as _;

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
    /// Whether the model supports vision/image inputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    /// Whether the model supports function/tool calling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_functions: Option<bool>,
    /// Whether the model supports fine-tuning through the registered backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_fine_tuning: Option<bool>,
    /// Whether the model supports extended thinking/reasoning. `None` means
    /// "use backend default". When `Some(false)`, compatible backends may send
    /// an explicit chat-template control to suppress thinking output. Sourced
    /// from `ModelConfig.supports_thinking`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    /// Whether the OpenAI-compatible request uses reasoning token fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses_reasoning_token_fields: Option<bool>,
    /// Whether the model accepts an explicit custom `temperature` field.
    /// `Some(false)` means the OpenAI-compatible adapter must omit
    /// `temperature` and rely on the provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_custom_temperature: Option<bool>,
    /// Whether the model accepts provider-enforced structured output schemas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_structured_outputs: Option<bool>,
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
    /// Whether this backend accepts provider-enforced structured output
    /// schemas. `None` means use the backend adapter default.
    #[serde(default)]
    pub supports_structured_outputs: Option<bool>,
}

impl BackendRegistration {
    pub fn from_backend_config(backend: &BackendConfig) -> Result<Self> {
        let endpoint = backend
            .endpoint
            .as_deref()
            .map(|value| resolve_env_reference(value, "endpoint", &backend.name))
            .transpose()?
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let endpoint = match (backend.protocol, endpoint) {
            (ProviderProtocol::Mock, endpoint) => endpoint,
            (_, Some(endpoint)) => {
                Some(normalize_endpoint_for_protocol(backend.protocol, &endpoint))
            }
            (_, None) => {
                return Err(BackendConfigurationError::MissingRequiredField {
                    protocol: backend.protocol,
                    field: "endpoint",
                }
                .into());
            }
        }
        .map(|value| {
            if backend.protocol == ProviderProtocol::Anthropic {
                normalize_anthropic_gateway_endpoint(&value)
            } else {
                value
            }
        });

        let default_model = backend
            .models
            .first()
            .map(|model| model.id.trim())
            .filter(|model| !model.is_empty())
            .ok_or(BackendConfigurationError::MissingRequiredField {
                protocol: backend.protocol,
                field: "model",
            })?
            .to_string();

        // Validate the structural registration contract before materializing
        // secrets so missing model/endpoint errors remain typed and stable
        // regardless of the caller's current environment.
        let api_key = match backend.api_key.as_deref() {
            Some(key) => resolve_secret_reference(key, "api_key", &backend.name)?,
            None if backend.backend_type == BackendType::Local
                || backend.protocol == ProviderProtocol::Ollama
                || backend.protocol == ProviderProtocol::Vllm =>
            {
                String::new()
            }
            None => {
                return Err(anyhow!(
                    "Missing API-key reference for backend '{}'. Set `api_key = \"env:VAR\"`.",
                    backend.name
                ));
            }
        };

        let mut extra_headers: HashMap<String, String> = backend
            .headers
            .iter()
            .map(|(key, value)| {
                let resolved = if is_sensitive_header(key) {
                    resolve_secret_reference(value, key, &backend.name)?
                } else {
                    resolve_env_reference(value, key, &backend.name)?
                };
                Ok((key.clone(), resolved))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        merge_custom_headers_from_env(backend.protocol, &mut extra_headers);

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
                supports_vision: Some(model.supports_vision),
                supports_functions: Some(model.supports_functions),
                supports_fine_tuning: Some(model.supports_fine_tuning),
                supports_thinking: Some(model.supports_thinking),
                uses_reasoning_token_fields: Some(model.uses_reasoning_token_fields),
                supports_custom_temperature: model.supports_custom_temperature,
                supports_structured_outputs: model.supports_structured_outputs,
            })
            .collect();

        Ok(Self {
            name: backend.name.clone(),
            protocol: backend.protocol,
            api_key,
            default_model: Some(default_model),
            models,
            endpoint,
            options: HashMap::new(),
            extra_headers,
            auto_tool_choice: backend.auto_tool_choice,
            supports_structured_outputs: backend.supports_structured_outputs,
        })
    }

    fn backend_config_json(&self) -> Result<JsonValue> {
        let model = self
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .ok_or(BackendConfigurationError::MissingRequiredField {
                protocol: self.protocol,
                field: "model",
            })?;
        let mut map = Map::new();

        map.insert(MODEL.to_string(), json!(model));
        if let Some(endpoint) = self
            .endpoint
            .as_deref()
            .map(str::trim)
            .filter(|endpoint| !endpoint.is_empty())
        {
            map.insert(BASE_URL.to_string(), json!(endpoint));
        } else if self.protocol != ProviderProtocol::Mock {
            return Err(BackendConfigurationError::MissingRequiredField {
                protocol: self.protocol,
                field: "endpoint",
            }
            .into());
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
        if let Some(supports_structured_outputs) = self.supports_structured_outputs {
            map.insert(
                config_keys::SUPPORTS_STRUCTURED_OUTPUTS.to_string(),
                json!(supports_structured_outputs),
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
                entry.insert(config_keys::ID.to_string(), json!(m.id));
                if let Some(info) = &m.info {
                    entry.insert(
                        config_keys::CONTEXT_WINDOW.to_string(),
                        json!(info.context_window),
                    );
                }
                if let Some(supports_vision) = m.supports_vision {
                    entry.insert(
                        config_keys::SUPPORTS_VISION.to_string(),
                        json!(supports_vision),
                    );
                }
                if let Some(supports_functions) = m.supports_functions {
                    entry.insert(
                        config_keys::SUPPORTS_FUNCTIONS.to_string(),
                        json!(supports_functions),
                    );
                }
                if let Some(supports_fine_tuning) = m.supports_fine_tuning {
                    entry.insert(
                        config_keys::SUPPORTS_FINE_TUNING.to_string(),
                        json!(supports_fine_tuning),
                    );
                }
                if let Some(supports_thinking) = m.supports_thinking {
                    entry.insert(
                        config_keys::SUPPORTS_THINKING.to_string(),
                        json!(supports_thinking),
                    );
                }
                if let Some(uses_reasoning_token_fields) = m.uses_reasoning_token_fields {
                    entry.insert(
                        config_keys::USES_REASONING_TOKEN_FIELDS.to_string(),
                        json!(uses_reasoning_token_fields),
                    );
                }
                if let Some(supports_custom_temperature) = m.supports_custom_temperature {
                    entry.insert(
                        config_keys::SUPPORTS_CUSTOM_TEMPERATURE.to_string(),
                        json!(supports_custom_temperature),
                    );
                }
                if let Some(supports_structured_outputs) = m.supports_structured_outputs {
                    entry.insert(
                        config_keys::SUPPORTS_STRUCTURED_OUTPUTS.to_string(),
                        json!(supports_structured_outputs),
                    );
                }
                (entry.len() > 1).then_some(JsonValue::Object(entry))
            })
            .collect();
        if !model_entries.is_empty() {
            map.insert(config_keys::MODELS.to_string(), json!(model_entries));
        }

        Ok(JsonValue::Object(map))
    }

    /// Register this backend into the supplied registry.
    pub async fn register(&self, registry: &LLMRegistry) -> Result<()> {
        let provider = Provider::from_protocol(
            self.protocol,
            &self.api_key,
            Some(self.backend_config_json()?),
        )
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
    if let Some(var_name) = value.strip_prefix(config_keys::ENV_PREFIX) {
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

fn resolve_secret_reference(value: &str, field: &str, backend_name: &str) -> Result<String> {
    if !value.starts_with(config_keys::ENV_PREFIX) {
        let mut message = String::new();
        let _ = write!(
            &mut message,
            "Backend '{}' field '{}' must be an env:VAR reference, not a literal secret. Durable credential custody belongs to auth.",
            backend_name, field
        );
        return Err(anyhow!(message));
    }
    resolve_env_reference(value, field, backend_name)
}

fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "authorization"
        || lower.contains("api-key")
        || lower.contains("apikey")
        || lower.contains("key")
        || lower.contains("token")
        || lower.contains("secret")
        || lower.contains("subscription-key")
}

/// Merge `ANTHROPIC_CUSTOM_HEADERS` (multi-line `Name: value`) into backend headers.
fn merge_custom_headers_from_env(
    protocol: ProviderProtocol,
    headers: &mut HashMap<String, String>,
) {
    if protocol != ProviderProtocol::Anthropic {
        return;
    }
    let Ok(raw) = env::var("ANTHROPIC_CUSTOM_HEADERS") else {
        return;
    };
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if !name.is_empty() && !value.is_empty() {
            headers.insert(name.to_string(), value.to_string());
        }
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

        for alias in &self.model_aliases {
            registry.register_model_alias(alias.alias.clone(), alias.model.clone());
            if let Some(backend) = &alias.backend {
                registry.set_model_route(alias.alias.clone(), backend.clone())?;
            }
        }

        if let Some(default_model) = &self.default_model {
            registry.set_default_model(default_model.clone())?;
        }

        for route in &self.operation_routes {
            if let Some(backend) = &route.backend {
                registry.set_operation_default(route.operation, backend.clone())?;
            }
            if let Some(model) = &route.model {
                registry.set_operation_model(route.operation, model.clone())?;
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

    fn backend(api_key: Option<&str>, extra_headers: HashMap<String, String>) -> BackendConfig {
        BackendConfig {
            name: "openai".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: Some("https://api.openai.com/v1".to_string()),
            api_key: api_key.map(str::to_string),
            headers: extra_headers,
            models: vec![crate::llm::ModelConfig {
                id: "fixture-model".to_string(),
                aliases: vec![],
                context_window: 0,
                supports_vision: false,
                supports_functions: false,
                supports_fine_tuning: false,
                supports_thinking: false,
                uses_reasoning_token_fields: false,
                supports_custom_temperature: None,
                supports_structured_outputs: None,
                max_output_tokens: None,
            }],
            auto_tool_choice: None,
            supports_structured_outputs: None,
        }
    }

    #[test]
    fn backend_registration_rejects_literal_api_key() {
        let err =
            BackendRegistration::from_backend_config(&backend(Some("sk-literal"), HashMap::new()))
                .unwrap_err();
        assert!(err.to_string().contains("env:VAR reference"));
    }

    #[test]
    fn backend_registration_rejects_literal_sensitive_header() {
        let mut backend = backend(None, HashMap::new());
        backend.backend_type = BackendType::Local;
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer literal".to_string());
        backend.headers = headers;
        let err = BackendRegistration::from_backend_config(&backend).unwrap_err();
        assert!(err.to_string().contains("env:VAR reference"));
    }

    #[test]
    fn backend_registration_allows_literal_non_sensitive_header() {
        let mut headers = HashMap::new();
        headers.insert("X-Trace".to_string(), "trace-id".to_string());
        let mut configured_backend = backend(None, headers);
        configured_backend.backend_type = BackendType::Local;
        let registration = BackendRegistration::from_backend_config(&configured_backend)
            .expect("non-sensitive literal headers are valid local-backend metadata");
        assert!(
            registration.extra_headers.contains_key("X-Trace"),
            "the non-sensitive header must be retained"
        );
    }

    #[test]
    fn backend_registration_requires_registered_model_and_endpoint() {
        let mut configured_backend = backend(Some("env:OPENAI_API_KEY"), HashMap::new());
        configured_backend.backend_type = BackendType::Local;
        configured_backend.api_key = None;
        configured_backend.models.clear();
        let error = BackendRegistration::from_backend_config(&configured_backend)
            .expect_err("missing model must fail closed");
        assert!(matches!(
            error.downcast_ref::<BackendConfigurationError>(),
            Some(BackendConfigurationError::MissingRequiredField {
                protocol: ProviderProtocol::OpenAI,
                field: "model",
            })
        ));

        let mut configured_backend = backend(Some("env:OPENAI_API_KEY"), HashMap::new());
        configured_backend.backend_type = BackendType::Local;
        configured_backend.api_key = None;
        configured_backend.endpoint = None;
        let error = BackendRegistration::from_backend_config(&configured_backend)
            .expect_err("missing endpoint must fail closed");
        assert!(matches!(
            error.downcast_ref::<BackendConfigurationError>(),
            Some(BackendConfigurationError::MissingRequiredField {
                protocol: ProviderProtocol::OpenAI,
                field: "endpoint",
            })
        ));
    }

    #[test]
    fn backend_registration_forwards_registered_model_capabilities() {
        let mut configured_backend = backend(None, HashMap::new());
        configured_backend.backend_type = BackendType::Local;
        let model = configured_backend
            .models
            .first_mut()
            .expect("fixture model");
        model.supports_vision = true;
        model.supports_functions = true;
        model.supports_fine_tuning = true;
        model.uses_reasoning_token_fields = true;
        model.supports_custom_temperature = Some(false);
        model.supports_structured_outputs = Some(true);

        let registration = BackendRegistration::from_backend_config(&configured_backend)
            .expect("registered backend config");
        let config = registration
            .backend_config_json()
            .expect("adapter configuration");
        let model = &config[config_keys::MODELS][0];

        assert_eq!(model[config_keys::SUPPORTS_VISION], true);
        assert_eq!(model[config_keys::SUPPORTS_FUNCTIONS], true);
        assert_eq!(model[config_keys::SUPPORTS_FINE_TUNING], true);
        assert_eq!(model[config_keys::USES_REASONING_TOKEN_FIELDS], true);
        assert_eq!(model[config_keys::SUPPORTS_CUSTOM_TEMPERATURE], false);
        assert_eq!(model[config_keys::SUPPORTS_STRUCTURED_OUTPUTS], true);
    }
}
