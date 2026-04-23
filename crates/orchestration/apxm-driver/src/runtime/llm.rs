//! LLM registry configuration for the runtime.

use crate::config::ApXmConfig;
use crate::error::DriverError;
use apxm_backends::{
    BackendFallback, BackendRegistration, LLMRegistry, ModelAliasRegistration, OperationRoute,
    RegistryPolicy,
};
use apxm_core::types::AISOperationType;
use std::env;

pub async fn configure_llm_registry(
    registry: &LLMRegistry,
    config: &ApXmConfig,
) -> Result<(), DriverError> {
    // Check for mock backend override (for benchmarking)
    if env::var("APXM_MOCK_BACKEND").is_ok() {
        use apxm_backends::llm::backends::MockLLMBackend;

        let latency_ms = env::var("APXM_MOCK_LATENCY_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(500);

        eprintln!(
            "[MOCK] Registering mock LLM backend with latency_ms={}",
            latency_ms
        );

        let mock = MockLLMBackend::new().with_latency_ms(latency_ms).default(
            apxm_backends::llm::backends::MockResponse::new("Mock LLM response for benchmarking"),
        );

        registry
            .register("mock", mock)
            .map_err(|e| DriverError::Driver(format!("Failed to register mock backend: {e}")))?;
        registry.set_default("mock").map_err(|e| {
            DriverError::Driver(format!("Failed to set mock as default backend: {e}"))
        })?;

        eprintln!("[MOCK] Mock backend registered successfully");
        eprintln!("[MOCK] Registered backends: {:?}", registry.backend_names());

        return Ok(());
    }

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
            "No backends configured.\n\n\
             Register at least one LLM backend:\n\n\
             \x20 dekk apxm backend add openai --type cloud --protocol openai\n\
             \x20 dekk apxm backend add anthropic --type cloud --protocol anthropic\n\
             \x20 dekk apxm backend add ollama --protocol ollama\n\n\
             Verify with: dekk apxm backend list"
                .to_string(),
        ));
    }

    for backend in &config.backends {
        if let Some(allowed) = &allowed_backends
            && !allowed.contains(&backend.name)
        {
            continue;
        }

        let registration = BackendRegistration::from_backend_config(backend).map_err(|e| {
            DriverError::Driver(format!(
                "Failed to build backend registration for '{}': {e}",
                backend.name
            ))
        })?;
        registration.register(registry).await.map_err(|e| {
            DriverError::Driver(format!(
                "Failed to register backend '{}': {e}",
                backend.name
            ))
        })?;
        registry.register_backend_provider(&backend.name, backend.protocol.as_str());
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

fn parse_operation_type(value: &str) -> Result<AISOperationType, DriverError> {
    value.parse::<AISOperationType>().map_err(|err| {
        DriverError::Driver(format!(
            "Invalid AIS operation '{}' in chat.routing.operation_routes: {}",
            value, err
        ))
    })
}
