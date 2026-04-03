use apxm_core::types::provider_spec::{ProviderProtocol, resolve_builtin_provider};

use crate::CredentialError;
use crate::credential::Credential;

/// Validate a credential by making a minimal API call.
///
/// Dispatches on the typed [`ProviderProtocol`] enum instead of raw provider
/// strings. Default base URLs are resolved from [`BUILTIN_PROVIDERS`].
pub async fn validate_credential(name: &str, cred: &Credential) -> Result<String, CredentialError> {
    let spec = resolve_builtin_provider(&cred.provider).ok_or_else(|| {
        CredentialError::Validation {
            name: name.to_string(),
            reason: format!("Unknown provider '{}' — cannot validate", cred.provider),
        }
    })?;

    let base = cred
        .base_url
        .as_deref()
        .or(spec.default_base_url)
        .unwrap_or("");
    let base = base.trim_end_matches('/');

    let client = reqwest::Client::new();

    match spec.protocol {
        ProviderProtocol::OpenAI => validate_openai(&client, name, cred, base).await,
        ProviderProtocol::Anthropic => validate_anthropic(&client, name, cred, base).await,
        ProviderProtocol::Google => validate_google(&client, name, cred, base).await,
        ProviderProtocol::Ollama => validate_ollama(&client, name, cred, base).await,
        // vLLM uses OpenAI-compatible validation (same /v1/models endpoint)
        ProviderProtocol::Vllm => validate_openai(&client, name, cred, base).await,
    }
}

fn require_api_key<'a>(name: &str, cred: &'a Credential) -> Result<&'a str, CredentialError> {
    cred.api_key
        .as_deref()
        .ok_or_else(|| CredentialError::Validation {
            name: name.to_string(),
            reason: "No API key set".to_string(),
        })
}

fn validation_err(name: &str, reason: impl Into<String>) -> CredentialError {
    CredentialError::Validation {
        name: name.to_string(),
        reason: reason.into(),
    }
}

async fn validate_openai(
    client: &reqwest::Client,
    name: &str,
    cred: &Credential,
    base: &str,
) -> Result<String, CredentialError> {
    let api_key = require_api_key(name, cred)?;

    // Try GET /v1/models first (standard OpenAI).
    let models_url = format!("{base}/v1/models");
    let mut req = client.get(&models_url).bearer_auth(api_key);
    for (k, v) in &cred.headers {
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
    let model = cred.model.as_deref().unwrap_or("gpt-4o-mini");
    let chat_url = format!("{base}/chat/completions");
    let mut req = client
        .post(&chat_url)
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .body(format!(
            r#"{{"model":"{model}","max_completion_tokens":1,"messages":[{{"role":"user","content":"hi"}}]}}"#,
        ));
    for (k, v) in &cred.headers {
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
    cred: &Credential,
    base: &str,
) -> Result<String, CredentialError> {
    let api_key = require_api_key(name, cred)?;
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
    cred: &Credential,
    base: &str,
) -> Result<String, CredentialError> {
    let api_key = require_api_key(name, cred)?;
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
    _cred: &Credential,
    base: &str,
) -> Result<String, CredentialError> {
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
