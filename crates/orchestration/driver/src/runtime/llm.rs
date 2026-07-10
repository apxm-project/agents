//! LLM registry configuration for the runtime.

use crate::config::ApXmConfig;
use crate::error::DriverError;
use apxm_backends::{
    BackendFallback, BackendRegistration, LLMRegistry, ModelAliasRegistration, OperationRoute,
    RegistryPolicy,
};
use apxm_core::constants::env as apxm_env;
use apxm_core::types::AISOperationType;
use std::env;

pub async fn configure_llm_registry(
    registry: &LLMRegistry,
    config: &ApXmConfig,
) -> Result<(), DriverError> {
    // Check for mock backend override (for benchmarking)
    if env::var(apxm_env::APXM_MOCK_BACKEND).is_ok() {
        use apxm_backends::llm::backends::MockLLMBackend;

        let latency_ms = env::var(apxm_env::APXM_MOCK_LATENCY_MS)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(500);

        eprintln!(
            "[MOCK] Registering mock LLM backend with latency_ms={}",
            latency_ms
        );

        let mut mock = MockLLMBackend::new().with_latency_ms(latency_ms).default(
            apxm_backends::llm::backends::MockResponse::new("Mock LLM response for benchmarking"),
        );

        if let Ok(script_path) = env::var(apxm_env::APXM_MOCK_SCRIPT_PATH) {
            mock = load_mock_script(mock, &script_path)?;
            eprintln!("[MOCK] Loaded scripted responses from {}", script_path);
        }

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
             Register at least one LLM backend in APXM backend configuration, \
             then verify the configured backend list before executing a graph."
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

/// One ordered pattern rule in an `APXM_MOCK_SCRIPT_PATH` fixture: the first
/// rule whose `contains` substring appears in the prompt wins (mirrors
/// `MockLLMBackend::when_prompt_contains`'s own first-match-wins order).
#[derive(serde::Deserialize)]
struct MockScriptRule {
    contains: String,
    response: String,
}

/// Top-level shape of an `APXM_MOCK_SCRIPT_PATH` fixture file: an ordered
/// list of pattern rules plus an optional `default` response for prompts
/// that match none of them. This is deliberately a thin JSON reader over the
/// existing `MockLLMBackend::when_prompt_contains`/`default` API — not a new
/// mock implementation (docs/plans/tasks/W5.1.md's threat model: "reusing
/// `MockLLMBackend::when_prompt_contains`, not a new mock implementation").
#[derive(serde::Deserialize)]
struct MockScript {
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    patterns: Vec<MockScriptRule>,
}

/// Load an `APXM_MOCK_SCRIPT_PATH` fixture and wire its ordered pattern
/// rules (plus optional default) into `mock`. A missing or malformed script
/// is a hard error (never a silent fall-back to the single canned default) —
/// per the threat model, a flaky/ambiguous scripted transcript must fail
/// loud, not degrade quietly into non-deterministic single-response
/// behavior.
fn load_mock_script(
    mock: apxm_backends::llm::backends::MockLLMBackend,
    script_path: &str,
) -> Result<apxm_backends::llm::backends::MockLLMBackend, DriverError> {
    let text = std::fs::read_to_string(script_path).map_err(|e| {
        DriverError::Driver(format!(
            "Failed to read APXM_MOCK_SCRIPT_PATH '{script_path}': {e}"
        ))
    })?;
    let script: MockScript = serde_json::from_str(&text).map_err(|e| {
        DriverError::Driver(format!(
            "Failed to parse APXM_MOCK_SCRIPT_PATH '{script_path}' as a mock script: {e}"
        ))
    })?;

    let mut mock = mock;
    for rule in script.patterns {
        mock = mock.when_prompt_contains(rule.contains, rule.response);
    }
    if let Some(default) = script.default {
        mock = mock.default(apxm_backends::llm::backends::MockResponse::new(default));
    }
    Ok(mock)
}

#[cfg(test)]
mod mock_script_tests {
    use super::*;
    use apxm_backends::llm::backends::MockLLMBackend;

    fn write_script(json: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().expect("tempfile");
        f.write_all(json.as_bytes()).expect("write script");
        f.flush().expect("flush");
        f
    }

    #[test]
    fn ordered_patterns_first_match_wins() {
        let script = write_script(
            r#"{
                "default": "fallback",
                "patterns": [
                    {"contains": "turn one", "response": "answer one"},
                    {"contains": "turn", "response": "generic turn answer"}
                ]
            }"#,
        );
        let mock = load_mock_script(MockLLMBackend::new(), script.path().to_str().unwrap())
            .expect("load script");

        let req_one = apxm_backends::LLMRequest::new("this is turn one".to_string());
        let resp_one = futures::executor::block_on(
            <MockLLMBackend as apxm_backends::LLMBackend>::generate(&mock, req_one),
        )
        .expect("generate turn one");
        assert_eq!(resp_one.content, "answer one");

        let req_two = apxm_backends::LLMRequest::new("this is turn two".to_string());
        let resp_two = futures::executor::block_on(
            <MockLLMBackend as apxm_backends::LLMBackend>::generate(&mock, req_two),
        )
        .expect("generate turn two");
        assert_eq!(resp_two.content, "generic turn answer");

        let req_none = apxm_backends::LLMRequest::new("unrelated prompt".to_string());
        let resp_none = futures::executor::block_on(
            <MockLLMBackend as apxm_backends::LLMBackend>::generate(&mock, req_none),
        )
        .expect("generate default");
        assert_eq!(resp_none.content, "fallback");
    }

    #[test]
    fn missing_script_file_is_a_hard_error_not_a_silent_fallback() {
        let result = load_mock_script(MockLLMBackend::new(), "/nonexistent/mock-script.json");
        let Err(err) = result else {
            panic!("missing script must fail loud");
        };
        assert!(format!("{err}").contains("APXM_MOCK_SCRIPT_PATH"));
    }

    #[test]
    fn malformed_script_json_is_a_hard_error() {
        let script = write_script("not json");
        let result = load_mock_script(MockLLMBackend::new(), script.path().to_str().unwrap());
        let Err(err) = result else {
            panic!("malformed script must fail loud");
        };
        assert!(format!("{err}").contains("APXM_MOCK_SCRIPT_PATH"));
    }
}
