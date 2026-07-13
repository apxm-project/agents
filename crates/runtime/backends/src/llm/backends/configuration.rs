//! Required configuration for concrete LLM backend adapters.
//!
//! Provider adapters receive only explicit registration data. They do not infer
//! deployment endpoints or model identifiers from a provider catalog.

use crate::llm::ProviderProtocol;
use apxm_core::types::ModelInfo;
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

/// Per-model behavior declared through backend registration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConfiguredModelCapabilities {
    pub context_window: usize,
    pub supports_vision: bool,
    pub supports_functions: bool,
    pub supports_fine_tuning: bool,
    pub uses_reasoning_token_fields: bool,
    pub supports_custom_temperature: Option<bool>,
    pub supports_structured_outputs: Option<bool>,
}

/// A model declared by backend registration together with its capability evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfiguredModel {
    pub id: String,
    pub capabilities: ConfiguredModelCapabilities,
}

impl ConfiguredModel {
    /// Converts registration data into the public inspection shape.
    fn model_info(self) -> ModelInfo {
        ModelInfo {
            id: self.id.clone(),
            name: self.id,
            context_window: self.capabilities.context_window,
            supports_vision: self.capabilities.supports_vision,
            supports_functions: self.capabilities.supports_functions,
        }
    }
}

/// Read the models attached to explicit backend registration data.
pub(crate) fn configured_models(config: Option<&Value>) -> Vec<ConfiguredModel> {
    config
        .and_then(|config| config.get(crate::llm::wire::config_keys::MODELS))
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|entry| {
                    let id = entry
                        .get(crate::llm::wire::config_keys::ID)
                        .and_then(Value::as_str)?;
                    Some(ConfiguredModel {
                        id: id.to_string(),
                        capabilities: ConfiguredModelCapabilities {
                            context_window: entry
                                .get(crate::llm::wire::config_keys::CONTEXT_WINDOW)
                                .and_then(Value::as_u64)
                                .and_then(|value| usize::try_from(value).ok())
                                .unwrap_or_default(),
                            supports_vision: entry
                                .get(crate::llm::wire::config_keys::SUPPORTS_VISION)
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            supports_functions: entry
                                .get(crate::llm::wire::config_keys::SUPPORTS_FUNCTIONS)
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            supports_fine_tuning: entry
                                .get(crate::llm::wire::config_keys::SUPPORTS_FINE_TUNING)
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            uses_reasoning_token_fields: entry
                                .get(crate::llm::wire::config_keys::USES_REASONING_TOKEN_FIELDS)
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            supports_custom_temperature: entry
                                .get(crate::llm::wire::config_keys::SUPPORTS_CUSTOM_TEMPERATURE)
                                .and_then(Value::as_bool),
                            supports_structured_outputs: entry
                                .get(crate::llm::wire::config_keys::SUPPORTS_STRUCTURED_OUTPUTS)
                                .and_then(Value::as_bool),
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Read the capability entries attached to explicit backend registration data.
pub(crate) fn configured_model_capabilities(
    config: Option<&Value>,
) -> HashMap<String, ConfiguredModelCapabilities> {
    configured_models(config)
        .into_iter()
        .map(|model| (model.id, model.capabilities))
        .collect()
}

/// Return public model metadata derived only from registered model data.
pub(crate) fn configured_model_info(config: Option<&Value>) -> Vec<ModelInfo> {
    configured_models(config)
        .into_iter()
        .map(ConfiguredModel::model_info)
        .collect()
}

/// A required backend adapter configuration value is absent or unusable.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BackendConfigurationError {
    /// The adapter was constructed without a registration payload.
    #[error("{protocol} backend requires an explicit registration configuration")]
    MissingConfiguration { protocol: ProviderProtocol },

    /// A required registration value is absent.
    #[error("{protocol} backend requires an explicit '{field}' configuration value")]
    MissingRequiredField {
        protocol: ProviderProtocol,
        field: &'static str,
    },

    /// A required registration value cannot be used.
    #[error("{protocol} backend configuration field '{field}' must be a non-empty string")]
    InvalidRequiredField {
        protocol: ProviderProtocol,
        field: &'static str,
    },

    /// An optional configured value has an unsupported representation.
    #[error("{protocol} backend configuration field '{field}' must be a string")]
    InvalidOptionalField {
        protocol: ProviderProtocol,
        field: String,
    },

    /// A configured environment reference could not be materialized.
    #[error(
        "{protocol} backend configuration field '{field}' references unset environment variable '{variable}'"
    )]
    MissingEnvironmentReference {
        protocol: ProviderProtocol,
        field: String,
        variable: String,
    },

    /// A string provider name did not identify a supported protocol.
    #[error("unknown backend provider '{provider}'")]
    UnknownProvider { provider: String },
}

/// Read a required non-empty string from a backend registration payload.
pub(crate) fn required_config_string(
    config: Option<&Value>,
    protocol: ProviderProtocol,
    field: &'static str,
) -> Result<String, BackendConfigurationError> {
    let config = config.ok_or(BackendConfigurationError::MissingConfiguration { protocol })?;
    let value = config
        .get(field)
        .ok_or(BackendConfigurationError::MissingRequiredField { protocol, field })?
        .as_str()
        .ok_or(BackendConfigurationError::InvalidRequiredField { protocol, field })?
        .trim();

    if value.is_empty() {
        return Err(BackendConfigurationError::InvalidRequiredField { protocol, field });
    }

    Ok(value.to_string())
}

/// Resolve an optional configuration value without converting an absent env var
/// into a literal header value.
pub(crate) fn resolve_configured_value(
    protocol: ProviderProtocol,
    field: String,
    value: &str,
) -> Result<String, BackendConfigurationError> {
    if let Some(variable) = value.strip_prefix("env:") {
        return std::env::var(variable).map_err(|_| {
            BackendConfigurationError::MissingEnvironmentReference {
                protocol,
                field,
                variable: variable.to_string(),
            }
        });
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_config_string_rejects_missing_and_blank_values() {
        let missing = required_config_string(None, ProviderProtocol::OpenAI, "model")
            .expect_err("missing config must fail closed");
        assert_eq!(
            missing,
            BackendConfigurationError::MissingConfiguration {
                protocol: ProviderProtocol::OpenAI,
            }
        );

        let blank = required_config_string(
            Some(&serde_json::json!({ "model": "  " })),
            ProviderProtocol::OpenAI,
            "model",
        )
        .expect_err("blank model must fail closed");
        assert_eq!(
            blank,
            BackendConfigurationError::InvalidRequiredField {
                protocol: ProviderProtocol::OpenAI,
                field: "model",
            }
        );
    }

    #[test]
    fn configured_model_info_preserves_registered_metadata() {
        let models = configured_model_info(Some(&serde_json::json!({
            "models": [{
                "id": "registered-deployment",
                "context_window": 32768,
                "supports_vision": true,
                "supports_functions": false
            }]
        })));

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "registered-deployment");
        assert_eq!(models[0].context_window, 32768);
        assert!(models[0].supports_vision);
        assert!(!models[0].supports_functions);
    }
}
