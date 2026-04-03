//! LLM registry configuration for the runtime.

use crate::config::ApXmConfig;
use crate::error::DriverError;
use apxm_backends::{
    BackendFallback, BackendRegistration, LLMRegistry, ModelAliasRegistration, ModelRegistration,
    OperationRoute, RegistryPolicy,
};
use apxm_core::types::AISOperationType;
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

    // Load from config.toml's [[backends]] (unified system)
    if config.backends.is_empty() {
        return Err(DriverError::Driver(
            "No backends configured. Add backends with: apxm backend add <name> ...".to_string(),
        ));
    }

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

