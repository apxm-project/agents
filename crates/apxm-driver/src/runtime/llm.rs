//! LLM registry configuration for the runtime.

use crate::config::{ApXmConfig, LlmBackendConfig};
use crate::error::DriverError;
use apxm_backends::{
    BackendFallback, BackendRegistration, LLMRegistry, ModelAliasRegistration, ModelRegistration,
    OperationRoute, RegistryPolicy,
};
use apxm_core::types::{AISOperationType, ProviderProtocol, resolve_builtin_provider};
use std::env;

pub async fn configure_llm_registry(
    registry: &LLMRegistry,
    config: &ApXmConfig,
) -> Result<(), DriverError> {
    let allowed_backends = if config.chat.providers.is_empty() {
        None
    } else {
        Some(
            config
                .chat
                .providers
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>(),
        )
    };

    // Priority 1: Load from config.toml's [[backends]] (new unified system)
    let mut loaded_from_backends = false;
    if !config.backends.is_empty() {
        loaded_from_backends = true;
        for backend in &config.backends {
            if let Some(allowed) = &allowed_backends
                && !allowed.contains(&backend.name)
            {
                continue;
            }

            let registration = unified_backend_to_registration(backend)?;
            registration.register(registry).await.map_err(|e| {
                DriverError::Driver(format!("Failed to register backend '{}': {e}", backend.name))
            })?;
        }
    }

    // Priority 2: Load from ~/.apxm/credentials.toml (legacy)
    let mut loaded_from_credentials = false;
    if !loaded_from_backends {
        if let Ok(store) = apxm_credentials::CredentialStore::open()
            && let Ok(credentials) = store.list_all()
            && !credentials.is_empty()
        {
            loaded_from_credentials = true;
            for (name, credential) in &credentials {
                if let Some(allowed) = &allowed_backends
                    && !allowed.contains(name)
                {
                    continue;
                }

                let backend_config = credential_to_backend_config(name, credential);
                let provider_name = &credential.provider;

                let protocol = resolve_provider_protocol(None, provider_name)?;
                let registration = credential_to_registration(name, &backend_config, protocol);
                registration.register(registry).await.map_err(|e| {
                    DriverError::Driver(format!("Failed to register backend '{}': {e}", name))
                })?;
            }
        }
    }

    // Priority 3: Load from config.toml's [[llm_backends]] (legacy)
    if !loaded_from_backends && !loaded_from_credentials {
        for backend in &config.llm_backends {
            if let Some(allowed) = &allowed_backends
                && !allowed.contains(&backend.name)
            {
                continue;
            }

            let provider_name = backend.provider.as_deref().unwrap_or("openai");
            let protocol = resolve_provider_protocol(Some(backend), provider_name)?;
            let registration = backend_to_registration(backend, protocol)?;
            registration.register(registry).await.map_err(|e| {
                DriverError::Driver(format!(
                    "Failed to register backend '{}': {e}",
                    backend.name
                ))
            })?;
        }
    }

    let default_backend = config
        .chat
        .default_backend
        .clone()
        .or_else(|| config.chat.providers.first().cloned())
        .or_else(|| registry.backend_names().into_iter().next());

    let mut operation_routes = config
        .chat
        .routing
        .operation_routes
        .iter()
        .map(|(operation, route)| {
            let operation = parse_operation_type(operation)?;
            Ok(OperationRoute {
                operation,
                backend: route.backend.clone(),
                model: route.model.clone(),
            })
        })
        .collect::<Result<Vec<_>, DriverError>>()?;

    if let Some(planning_model) = &config.chat.planning_model {
        operation_routes.push(OperationRoute {
            operation: AISOperationType::Plan,
            backend: None,
            model: Some(planning_model.clone()),
        });
    }

    let model_aliases = config
        .chat
        .routing
        .model_aliases
        .iter()
        .map(|(alias, target)| ModelAliasRegistration {
            alias: alias.clone(),
            model: target.model.clone(),
            backend: target.backend.clone(),
        })
        .collect::<Vec<_>>();

    let fallback_chains = config
        .chat
        .routing
        .fallback_chains
        .iter()
        .map(|chain| BackendFallback {
            backend: chain.backend.clone(),
            fallbacks: chain.fallbacks.clone(),
        })
        .collect::<Vec<_>>();

    let policy = RegistryPolicy {
        default_backend,
        default_model: config.chat.default_model.clone(),
        operation_routes,
        model_aliases,
        fallback_chains,
    };

    policy
        .apply(registry)
        .map_err(|e| DriverError::Driver(format!("Failed to apply LLM registry policy: {e}")))?;

    Ok(())
}

/// Resolve a backend protocol using explicit config first, then builtin provider aliases.
fn resolve_provider_protocol(
    config: Option<&LlmBackendConfig>,
    provider_name: &str,
) -> Result<ProviderProtocol, DriverError> {
    if let Some(protocol) = config.and_then(|cfg| cfg.protocol) {
        return Ok(protocol);
    }

    if let Ok(protocol) = provider_name.parse::<ProviderProtocol>() {
        return Ok(protocol);
    }

    let spec = resolve_builtin_provider(provider_name)
        .ok_or_else(|| DriverError::Driver(format!("Unknown provider '{}'", provider_name)))?;
    Ok(spec.protocol)
}

/// Convert a credential from the credential store to an LlmBackendConfig.
fn credential_to_backend_config(
    name: &str,
    credential: &apxm_credentials::credential::Credential,
) -> LlmBackendConfig {
    LlmBackendConfig {
        name: name.to_string(),
        provider: Some(credential.provider.clone()),
        protocol: None,
        default_model: credential.model.clone(),
        models: Vec::new(),
        api_key: credential.api_key.clone(),
        endpoint: credential.base_url.clone(),
        options: std::collections::HashMap::new(),
        extra_headers: credential
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

fn credential_to_registration(
    name: &str,
    backend_config: &LlmBackendConfig,
    protocol: ProviderProtocol,
) -> BackendRegistration {
    BackendRegistration {
        name: name.to_string(),
        protocol,
        api_key: backend_config.api_key.clone().unwrap_or_default(),
        default_model: backend_config.default_model.clone(),
        models: backend_config
            .models
            .iter()
            .map(|model| ModelRegistration {
                id: model.id.clone(),
                aliases: model.aliases.clone(),
                info: model.to_model_info(),
            })
            .collect(),
        endpoint: backend_config.endpoint.clone(),
        options: backend_config.options.clone(),
        extra_headers: backend_config.extra_headers.clone(),
    }
}

fn backend_to_registration(
    config: &LlmBackendConfig,
    protocol: ProviderProtocol,
) -> Result<BackendRegistration, DriverError> {
    let api_key = resolve_api_key(protocol, config)?;
    let default_model = config
        .default_model
        .as_deref()
        .map(|value| resolve_env_value(value, "default_model", &config.name))
        .transpose()?;
    let endpoint = config
        .endpoint
        .as_deref()
        .map(|value| resolve_env_value(value, "endpoint", &config.name))
        .transpose()?;
    let options = config
        .options
        .iter()
        .map(|(key, value)| resolve_env_value(value, key, &config.name).map(|v| (key.clone(), v)))
        .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
    let extra_headers = config
        .extra_headers
        .iter()
        .map(|(key, value)| {
            resolve_env_value(value, &format!("extra_headers.{}", key), &config.name)
                .map(|v| (key.clone(), v))
        })
        .collect::<Result<std::collections::HashMap<_, _>, _>>()?;

    let models = config
        .models
        .iter()
        .map(|model| {
            let id = resolve_env_value(&model.id, "models.id", &config.name)?;
            let aliases = model
                .aliases
                .iter()
                .map(|alias| resolve_env_value(alias, "models.aliases", &config.name))
                .collect::<Result<Vec<_>, _>>()?;
            let info = model.to_model_info().map(|mut info| {
                info.id = id.clone();
                info
            });
            Ok(ModelRegistration { id, aliases, info })
        })
        .collect::<Result<Vec<_>, DriverError>>()?;

    Ok(BackendRegistration {
        name: config.name.clone(),
        protocol,
        api_key,
        default_model,
        models,
        endpoint,
        options,
        extra_headers,
    })
}

/// Convert a unified BackendConfig to a BackendRegistration.
fn unified_backend_to_registration(
    backend: &apxm_core::types::BackendConfig,
) -> Result<BackendRegistration, DriverError> {
    use apxm_core::types::{BackendType, ProviderProtocol};

    // Resolve API key (with env: support)
    let api_key = match backend.api_key.as_deref() {
        Some(key) if key.starts_with("env:") => {
            let env_name = key.strip_prefix("env:").unwrap();
            env::var(env_name).map_err(|_| {
                DriverError::Driver(format!(
                    "Environment variable '{}' not set for backend '{}'",
                    env_name, backend.name
                ))
            })?
        }
        Some(key) => key.to_string(),
        None if backend.backend_type == BackendType::Local
            || backend.protocol == ProviderProtocol::Ollama =>
        {
            String::new()
        }
        None => {
            return Err(DriverError::Driver(format!(
                "Missing API key for backend '{}'. Set `api_key` or use `env:VAR`.",
                backend.name
            )))
        }
    };

    // Resolve endpoint (with env: support)
    let endpoint = backend
        .endpoint
        .as_deref()
        .map(|value| {
            if let Some(var_name) = value.strip_prefix("env:") {
                env::var(var_name).map_err(|_| {
                    DriverError::Driver(format!(
                        "Environment variable '{}' not set for endpoint in backend '{}'",
                        var_name, backend.name
                    ))
                })
            } else {
                Ok(value.to_string())
            }
        })
        .transpose()?;

    // Resolve headers (with env: support)
    let extra_headers = backend
        .headers
        .iter()
        .map(|(key, value)| {
            let resolved_value = if let Some(var_name) = value.strip_prefix("env:") {
                env::var(var_name).map_err(|_| {
                    DriverError::Driver(format!(
                        "Environment variable '{}' not set for header '{}' in backend '{}'",
                        var_name, key, backend.name
                    ))
                })?
            } else {
                value.clone()
            };
            Ok((key.clone(), resolved_value))
        })
        .collect::<Result<std::collections::HashMap<_, _>, DriverError>>()?;

    // Convert models
    let models = backend
        .models
        .iter()
        .map(|model| {
            let info = Some(apxm_core::types::ModelInfo {
                id: model.id.clone(),
                name: model.id.clone(), // Use id as name if not specified
                context_window: model.context_window,
                supports_vision: model.supports_vision,
                supports_functions: model.supports_functions,
            });
            ModelRegistration {
                id: model.id.clone(),
                aliases: model.aliases.clone(),
                info,
            }
        })
        .collect();

    Ok(BackendRegistration {
        name: backend.name.clone(),
        protocol: backend.protocol,
        api_key,
        default_model: None, // BackendConfig doesn't have default_model; models are explicit
        models,
        endpoint,
        options: std::collections::HashMap::new(), // BackendConfig doesn't have options
        extra_headers,
    })
}

fn parse_operation_type(value: &str) -> Result<AISOperationType, DriverError> {
    value.parse::<AISOperationType>().map_err(|err| {
        DriverError::Driver(format!(
            "Invalid AIS operation '{}' in chat.routing.operation_routes: {}",
            value, err
        ))
    })
}

fn resolve_api_key(
    provider: ProviderProtocol,
    config: &LlmBackendConfig,
) -> Result<String, DriverError> {
    match config.api_key.as_deref() {
        Some(key) if key.starts_with("env:") => {
            let env_name = key.strip_prefix("env:").unwrap();
            env::var(env_name).map_err(|_| {
                DriverError::Driver(format!(
                    "Environment variable '{}' not set for backend '{}'",
                    env_name, config.name
                ))
            })
        }
        Some(key) if !key.is_empty() => Ok(key.to_string()),
        _ if matches!(provider, ProviderProtocol::Ollama) => Ok(String::new()),
        _ => Err(DriverError::Driver(format!(
            "Missing API key for backend '{}'. Set `api_key` or use `env:VAR`.",
            config.name
        ))),
    }
}

/// Resolve a value that may be prefixed with `env:` to read from an environment variable.
///
/// Returns an error when an `env:` prefix is present but the variable is not set,
/// ensuring misconfiguration is caught at startup rather than at request time.
fn resolve_env_value(value: &str, field: &str, backend: &str) -> Result<String, DriverError> {
    if let Some(var_name) = value.strip_prefix("env:") {
        env::var(var_name).map_err(|_| {
            DriverError::Driver(format!(
                "Environment variable '{}' not set for field '{}' in backend '{}'",
                var_name, field, backend
            ))
        })
    } else {
        Ok(value.to_string())
    }
}
