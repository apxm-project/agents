use crate::BackendError;
use crate::backend::validate_backend_references;
use apxm_backends::llm::{
    BackendConfig, ProviderProtocol, normalize_anthropic_gateway_endpoint,
    normalize_endpoint_for_protocol,
};
use std::collections::HashMap;

/// Validate a backend by making a minimal API call.
///
/// Dispatches on the typed [`ProviderProtocol`] enum using only the backend's
/// explicit endpoint and registered model configuration.
pub async fn validate_backend(backend: &BackendConfig) -> Result<String, BackendError> {
    let backend = materialize_validation_backend(backend)?;

    if backend.protocol == ProviderProtocol::Mock {
        return Ok(format!("Mock backend '{}' is always valid", backend.name));
    }

    let base = backend
        .endpoint
        .as_deref()
        .ok_or_else(|| BackendError::MissingEndpoint {
            backend: backend.name.clone(),
        })?;
    let normalized = normalize_endpoint_for_protocol(backend.protocol, base);
    let base = normalized.trim_end_matches('/');

    let client = reqwest::Client::new();

    match backend.protocol {
        ProviderProtocol::OpenAI => validate_openai(&client, &backend.name, &backend, base).await,
        ProviderProtocol::Anthropic => {
            validate_anthropic(&client, &backend.name, &backend, base).await
        }
        ProviderProtocol::Google => validate_google(&client, &backend.name, &backend, base).await,
        ProviderProtocol::Ollama => validate_ollama(&client, &backend.name, &backend, base).await,
        ProviderProtocol::Vllm => validate_vllm(&client, &backend.name, &backend, base).await,
        ProviderProtocol::LlamaCpp => {
            validate_openai(&client, &backend.name, &backend, base).await
        }
        ProviderProtocol::Mock => {
            unreachable!("mock validation returns before endpoint resolution")
        }
    }
}

fn require_api_key<'a>(name: &str, backend: &'a BackendConfig) -> Result<&'a str, BackendError> {
    backend
        .api_key
        .as_deref()
        .ok_or_else(|| BackendError::MissingApiKey {
            backend: name.to_string(),
        })
}

fn require_model<'a>(name: &str, backend: &'a BackendConfig) -> Result<&'a str, BackendError> {
    backend
        .models
        .first()
        .map(|model| model.id.trim())
        .filter(|model| !model.is_empty())
        .ok_or_else(|| BackendError::MissingModel {
            backend: name.to_string(),
        })
}

fn validation_err(name: &str, reason: impl Into<String>) -> BackendError {
    BackendError::Io(std::io::Error::other(format!(
        "Backend '{}': {}",
        name,
        reason.into()
    )))
}

fn materialize_validation_backend(backend: &BackendConfig) -> Result<BackendConfig, BackendError> {
    materialize_validation_backend_with_env(backend, |key| std::env::var(key).ok())
}

fn materialize_validation_backend_with_env<F>(
    backend: &BackendConfig,
    env: F,
) -> Result<BackendConfig, BackendError>
where
    F: Fn(&str) -> Option<String>,
{
    validate_backend_references(backend)?;
    let mut materialized = backend.clone();

    if let Some(api_key) = backend.api_key.as_deref() {
        materialized.api_key = Some(resolve_env_value(&backend.name, "api_key", api_key, &env)?);
    }

    if let Some(endpoint) = backend.endpoint.as_deref() {
        let resolved = resolve_env_value(&backend.name, "endpoint", endpoint, &env)?;
        let normalized = normalize_endpoint_for_protocol(backend.protocol, &resolved);
        materialized.endpoint = Some(if backend.protocol == ProviderProtocol::Anthropic {
            normalize_anthropic_gateway_endpoint(&normalized)
        } else {
            normalized
        });
    }

    let mut headers = backend
        .headers
        .iter()
        .map(|(key, value)| {
            resolve_env_value(&backend.name, key, value, &env)
                .map(|resolved| (key.clone(), resolved))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    if backend.protocol == ProviderProtocol::Anthropic
        && let Some(raw) = env("ANTHROPIC_CUSTOM_HEADERS")
    {
        merge_custom_headers(&mut headers, &raw);
    }
    materialized.headers = headers;

    Ok(materialized)
}

fn resolve_env_value<F>(
    backend_name: &str,
    field: &str,
    value: &str,
    env: &F,
) -> Result<String, BackendError>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(var_name) = value.strip_prefix("env:") {
        env(var_name).ok_or_else(|| BackendError::MissingEnvironmentReference {
            backend: backend_name.to_string(),
            field: field.to_string(),
            variable: var_name.to_string(),
        })
    } else {
        Ok(value.to_string())
    }
}

fn merge_custom_headers(headers: &mut HashMap<String, String>, raw: &str) {
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

async fn validate_openai(
    client: &reqwest::Client,
    name: &str,
    backend: &BackendConfig,
    base: &str,
) -> Result<String, BackendError> {
    let api_key = require_api_key(name, backend)?;

    // Try GET /models first (standard OpenAI).
    // Convention: `base` already includes the version prefix (e.g. `/v1`).
    let models_url = format!("{base}/models");
    let mut req = client.get(&models_url).bearer_auth(api_key);
    for (k, v) in &backend.headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| validation_err(name, format!("Request failed: {e}")))?;

    if resp.status().is_success() {
        return Ok(format!("OK ({})", resp.status()));
    }

    // Probe a minimal chat completion for explicitly configured gateways
    // that don't expose /v1/models but do serve /chat/completions).
    let model = require_model(name, backend)?;
    let chat_url = format!("{base}/chat/completions");
    let mut req = client
        .post(&chat_url)
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .body(format!(
            r#"{{"model":"{model}","max_completion_tokens":1,"messages":[{{"role":"user","content":"hi"}}]}}"#,
        ));
    for (k, v) in &backend.headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| validation_err(name, format!("Chat completions request failed: {e}")))?;

    if resp.status().is_success() || resp.status().as_u16() == 400 {
        Ok(format!("OK ({})", resp.status()))
    } else {
        Err(validation_err(name, format!("HTTP {}", resp.status())))
    }
}

async fn validate_anthropic(
    client: &reqwest::Client,
    name: &str,
    backend: &BackendConfig,
    base: &str,
) -> Result<String, BackendError> {
    let api_key = require_api_key(name, backend)?;
    // Convention: `base` already includes the version prefix (e.g. `/v1`).
    let url = format!("{base}/messages");

    let model = require_model(name, backend)?;

    let body = format!(
        "{{\"model\":\"{}\",\"max_tokens\":1,\"messages\":[{{\"role\":\"user\",\"content\":\"hi\"}}]}}",
        model
    );

    // Build request with standard Anthropic headers.
    let mut req = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json");

    // Inject custom headers from backend config (e.g. X-Custom-Gateway-Key).
    // Values prefixed with "env:" are resolved from environment variables.
    for (k, v) in &backend.headers {
        let resolved = if let Some(var) = v.strip_prefix("env:") {
            std::env::var(var).unwrap_or_else(|_| v.clone())
        } else {
            v.clone()
        };
        req = req.header(k.as_str(), resolved);
    }

    let resp = req
        .body(body)
        .send()
        .await
        .map_err(|e| validation_err(name, format!("Request failed: {e}")))?;

    if resp.status().is_success() || resp.status().as_u16() == 400 {
        // 400 means auth worked but request payload was invalid — key is valid.
        Ok(format!("OK ({})", resp.status()))
    } else {
        Err(validation_err(name, format!("HTTP {}", resp.status())))
    }
}

async fn validate_google(
    client: &reqwest::Client,
    name: &str,
    backend: &BackendConfig,
    base: &str,
) -> Result<String, BackendError> {
    let api_key = require_api_key(name, backend)?;
    // Convention: `base` already includes the version prefix (e.g. `/v1`).
    let url = format!("{base}/models?key={api_key}");

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| validation_err(name, format!("Request failed: {e}")))?;

    if resp.status().is_success() {
        Ok(format!("OK ({})", resp.status()))
    } else {
        Err(validation_err(name, format!("HTTP {}", resp.status())))
    }
}

async fn validate_ollama(
    client: &reqwest::Client,
    name: &str,
    _backend: &BackendConfig,
    base: &str,
) -> Result<String, BackendError> {
    let url = format!("{base}/api/tags");

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| validation_err(name, format!("Connection failed: {e}")))?;

    if resp.status().is_success() {
        Ok(format!("OK ({})", resp.status()))
    } else {
        Err(validation_err(name, format!("HTTP {}", resp.status())))
    }
}

async fn validate_vllm(
    client: &reqwest::Client,
    name: &str,
    backend: &BackendConfig,
    base: &str,
) -> Result<String, BackendError> {
    let models_url = format!("{base}/models");
    let mut req = client.get(&models_url);
    if let Some(api_key) = backend.api_key.as_deref().filter(|key| !key.is_empty()) {
        req = req.bearer_auth(api_key);
    }
    for (k, v) in &backend.headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| validation_err(name, format!("Request failed: {e}")))?;

    if resp.status().is_success() {
        // The vllm protocol contract is the APXM-fork's graph-aware HTTP
        // surface. A backend that serves /v1/models but is missing
        // /v1/apxm/scheduler is upstream vanilla vLLM, not the APXM
        // fork — refuse to validate so the operator catches the
        // mismatch at registration time instead of at first graph
        // execution.
        let scheduler_url = format!("{base}/apxm/scheduler");
        let mut sreq = client.get(&scheduler_url);
        if let Some(api_key) = backend.api_key.as_deref().filter(|key| !key.is_empty()) {
            sreq = sreq.bearer_auth(api_key);
        }
        for (k, v) in &backend.headers {
            sreq = sreq.header(k.as_str(), v.as_str());
        }
        let sresp = sreq
            .send()
            .await
            .map_err(|e| validation_err(name, format!("/v1/apxm/scheduler probe failed: {e}")))?;
        if !sresp.status().is_success() {
            return Err(validation_err(
                name,
                format!(
                    "vLLM server at {base} is missing /v1/apxm/scheduler (HTTP {}). \
                     The vllm protocol expects the APXM fork; register vanilla \
                     vLLM under protocol=openai instead.",
                    sresp.status()
                ),
            ));
        }
        return Ok(format!("OK ({})", resp.status()));
    }

    let model = require_model(name, backend)?;

    let chat_url = format!("{base}/chat/completions");
    let mut req = client
        .post(&chat_url)
        .header("content-type", "application/json")
        .body(format!(
            r#"{{"model":"{model}","max_completion_tokens":1,"messages":[{{"role":"user","content":"hi"}}]}}"#,
        ));
    if let Some(api_key) = backend.api_key.as_deref().filter(|key| !key.is_empty()) {
        req = req.bearer_auth(api_key);
    }
    for (k, v) in &backend.headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| validation_err(name, format!("Chat completions request failed: {e}")))?;

    if resp.status().is_success() || resp.status().as_u16() == 400 {
        Ok(format!("OK ({})", resp.status()))
    } else {
        Err(validation_err(name, format!("HTTP {}", resp.status())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_backends::llm::{BackendType, ModelConfig};
    use std::collections::HashMap;

    fn anthropic_backend() -> BackendConfig {
        BackendConfig {
            name: "anthropic".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::Anthropic,
            endpoint: Some("env:APXM_TEST_ANTHROPIC_BASE_URL".to_string()),
            api_key: Some("env:APXM_TEST_ANTHROPIC_API_KEY".to_string()),
            headers: HashMap::new(),
            models: vec![ModelConfig {
                id: "claude-sonnet-4-6".to_string(),
                context_window: 0,
                supports_vision: false,
                supports_functions: true,
                supports_fine_tuning: false,
                supports_thinking: true,
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
    fn materialized_validation_backend_resolves_env_and_anthropic_gateway_headers() {
        let materialized =
            materialize_validation_backend_with_env(&anthropic_backend(), |key| match key {
                "APXM_TEST_ANTHROPIC_BASE_URL" => {
                    Some("https://llm-api.amd.com/Anthropic".to_string())
                }
                "APXM_TEST_ANTHROPIC_API_KEY" => Some("test-api-key".to_string()),
                "ANTHROPIC_CUSTOM_HEADERS" => Some(
                    "Ocp-Apim-Subscription-Key: test-subscription\nX-Trace: trace-id".to_string(),
                ),
                _ => None,
            })
            .unwrap();

        assert_eq!(materialized.api_key.as_deref(), Some("test-api-key"));
        assert_eq!(
            materialized.endpoint.as_deref(),
            Some("https://llm-api.amd.com/Anthropic/v1")
        );
        assert_eq!(
            materialized.headers.get("Ocp-Apim-Subscription-Key"),
            Some(&"test-subscription".to_string())
        );
        assert_eq!(
            materialized.headers.get("X-Trace"),
            Some(&"trace-id".to_string())
        );
    }

    #[test]
    fn validation_rejects_literal_api_key_before_materializing() {
        let mut backend = anthropic_backend();
        backend.api_key = Some("literal-secret".to_string());
        let err = materialize_validation_backend_with_env(&backend, |_| None).unwrap_err();
        assert!(matches!(
            err,
            BackendError::LiteralSecretReference { field, .. } if field == "api_key"
        ));
    }

    #[tokio::test]
    async fn validation_requires_explicit_endpoint_and_model() {
        let mut backend = anthropic_backend();
        backend.endpoint = None;
        let error = validate_backend(&backend)
            .await
            .expect_err("missing endpoint must fail closed");
        assert!(matches!(error, BackendError::MissingEndpoint { .. }));

        let mut backend = anthropic_backend();
        backend.models.clear();
        let error = validate_backend(&backend)
            .await
            .expect_err("missing model must fail closed");
        assert!(matches!(error, BackendError::MissingModel { .. }));
    }
}
