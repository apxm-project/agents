use apxm_core::types::provider_spec::{ProviderProtocol, resolve_builtin_provider};
use apxm_core::types::BackendConfig;
use crate::BackendError;

/// Validate a backend by making a minimal API call.
///
/// Dispatches on the typed [`ProviderProtocol`] enum. Default base URLs are
/// resolved from [`BUILTIN_PROVIDERS`] if not specified in the backend config.
pub async fn validate_backend(backend: &BackendConfig) -> Result<String, BackendError> {
    let spec = resolve_builtin_provider(&backend.protocol.to_string()).ok_or_else(|| {
        BackendError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Unknown protocol '{}' — cannot validate", backend.protocol),
        ))
    })?;

    let base = backend
        .endpoint
        .as_deref()
        .or(spec.default_base_url)
        .unwrap_or("");
    let base = base.trim_end_matches('/');

    let client = reqwest::Client::new();

    match backend.protocol {
        ProviderProtocol::OpenAI => validate_openai(&client, &backend.name, backend, base).await,
        ProviderProtocol::Anthropic => validate_anthropic(&client, &backend.name, backend, base).await,
        ProviderProtocol::Google => validate_google(&client, &backend.name, backend, base).await,
        ProviderProtocol::Ollama => validate_ollama(&client, &backend.name, backend, base).await,
        // vLLM uses OpenAI-compatible validation (same /v1/models endpoint)
        ProviderProtocol::Vllm => validate_openai(&client, &backend.name, backend, base).await,
    }
}

fn require_api_key<'a>(name: &str, backend: &'a BackendConfig) -> Result<&'a str, BackendError> {
    backend.api_key.as_deref().ok_or_else(|| {
        BackendError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Backend '{}': No API key set", name),
        ))
    })
}

fn validation_err(name: &str, reason: impl Into<String>) -> BackendError {
    BackendError::Io(std::io::Error::new(
        std::io::ErrorKind::Other,
        format!("Backend '{}': {}", name, reason.into()),
    ))
}

async fn validate_openai(
    client: &reqwest::Client,
    name: &str,
    backend: &BackendConfig,
    base: &str,
) -> Result<String, BackendError> {
    let api_key = require_api_key(name, backend)?;

    // Try GET /v1/models first (standard OpenAI).
    let models_url = format!("{base}/v1/models");
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

    // Fallback: minimal chat completion (for on-premises/custom gateways
    // that don't expose /v1/models but do serve /chat/completions).
    let model = backend
        .models
        .first()
        .map(|m| m.id.as_str())
        .unwrap_or("gpt-4o-mini");
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
    let url = format!("{base}/v1/messages");

    let resp = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-3-haiku-20240307","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .map_err(|e| validation_err(name, format!("Request failed: {e}")))?;

    if resp.status().is_success() || resp.status().as_u16() == 400 {
        // 400 means auth worked but request was bad — key is valid
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
    let url = format!("{base}/v1/models?key={api_key}");

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
