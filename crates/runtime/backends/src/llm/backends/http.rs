use anyhow::{Context, Result};
use futures::StreamExt as _;
use serde::de::DeserializeOwned;
use std::time::Duration;

pub(crate) const MAX_PROVIDER_ERROR_BODY_BYTES: usize = 64 * 1024;
pub(crate) const MAX_PROVIDER_RESPONSE_BODY_BYTES: usize = 8 * 1024 * 1024;

const BODY_TRUNCATION_MARKER: &str = " [body truncated]";
const REDACTED_MARKER: &str = "[REDACTED]";

const CONNECT_TIMEOUT_ENV: &str = "APXM_LLM_CONNECT_TIMEOUT_MS";
const READ_TIMEOUT_ENV: &str = "APXM_LLM_READ_TIMEOUT_MS";
const POOL_IDLE_TIMEOUT_ENV: &str = "APXM_LLM_POOL_IDLE_TIMEOUT_MS";
const TCP_KEEPALIVE_ENV: &str = "APXM_LLM_TCP_KEEPALIVE_MS";

const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_READ_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_POOL_IDLE_TIMEOUT_MS: u64 = 90_000;
const DEFAULT_TCP_KEEPALIVE_MS: u64 = 60_000;

pub(crate) fn llm_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(env_duration_ms(
            CONNECT_TIMEOUT_ENV,
            DEFAULT_CONNECT_TIMEOUT_MS,
        ))
        .read_timeout(env_duration_ms(READ_TIMEOUT_ENV, DEFAULT_READ_TIMEOUT_MS))
        .pool_idle_timeout(env_duration_ms(
            POOL_IDLE_TIMEOUT_ENV,
            DEFAULT_POOL_IDLE_TIMEOUT_MS,
        ))
        .tcp_keepalive(Some(env_duration_ms(
            TCP_KEEPALIVE_ENV,
            DEFAULT_TCP_KEEPALIVE_MS,
        )))
        .build()
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to build tuned LLM HTTP client");
            reqwest::Client::new()
        })
}

pub(crate) async fn read_provider_json<T>(
    response: reqwest::Response,
    context: &'static str,
    secrets: &[&str],
) -> Result<T>
where
    T: DeserializeOwned,
{
    let body = read_bounded_body(response, MAX_PROVIDER_RESPONSE_BODY_BYTES)
        .await
        .with_context(|| format!("{context}: failed to read response body"))?;

    if body.truncated {
        anyhow::bail!(
            "{context}: response body exceeded the {} byte limit",
            MAX_PROVIDER_RESPONSE_BODY_BYTES
        );
    }

    serde_json::from_slice(&body.bytes).with_context(|| {
        format!(
            "{context}: invalid JSON response body: {}",
            redact_body_preview(&body.bytes, secrets)
        )
    })
}

pub(crate) async fn read_provider_error_body(
    response: reqwest::Response,
    secrets: &[&str],
) -> String {
    match read_bounded_body(response, MAX_PROVIDER_ERROR_BODY_BYTES).await {
        Ok(body) => {
            let mut text = redact_body(&String::from_utf8_lossy(&body.bytes), secrets);
            if body.truncated {
                text.push_str(BODY_TRUNCATION_MARKER);
            }
            if text.trim().is_empty() {
                "<empty response body>".to_string()
            } else {
                text
            }
        }
        Err(error) => format!(
            "<response body unavailable: {}>",
            redact_body(&error.to_string(), secrets)
        ),
    }
}

pub(crate) async fn require_provider_success(
    response: reqwest::Response,
    provider: &'static str,
    secrets: &[&str],
) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let body = read_provider_error_body(response, secrets).await;
    anyhow::bail!("{provider} API error (status {status}): {body}");
}

struct BoundedBody {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_bounded_body(response: reqwest::Response, limit: usize) -> Result<BoundedBody> {
    let content_length = response.content_length();
    let mut stream = response.bytes_stream();
    let mut bytes =
        Vec::with_capacity(content_length.map_or(0, |length| (length as usize).min(limit)));

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("provider response stream read failed")?;
        let remaining = limit.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            return Ok(BoundedBody {
                bytes,
                truncated: true,
            });
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(BoundedBody {
        bytes,
        truncated: false,
    })
}

fn redact_body(body: &str, secrets: &[&str]) -> String {
    let mut redacted = if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body) {
        redact_json_value(&mut value, false);
        value.to_string()
    } else {
        body.to_string()
    };

    for secret in secrets.iter().copied().filter(|secret| !secret.is_empty()) {
        redacted = redacted.replace(secret, REDACTED_MARKER);
    }
    redact_inline_credentials(&redacted)
}

fn redact_body_preview(body: &[u8], secrets: &[&str]) -> String {
    let preview_len = body.len().min(MAX_PROVIDER_ERROR_BODY_BYTES);
    let mut preview = redact_body(&String::from_utf8_lossy(&body[..preview_len]), secrets);
    if body.len() > preview_len {
        preview.push_str(BODY_TRUNCATION_MARKER);
    }
    preview
}

fn redact_json_value(value: &mut serde_json::Value, sensitive_parent: bool) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                let sensitive = sensitive_parent || is_sensitive_key(key);
                if sensitive && !value.is_null() {
                    *value = serde_json::Value::String(REDACTED_MARKER.to_string());
                } else {
                    redact_json_value(value, sensitive);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_json_value(value, sensitive_parent);
            }
        }
        serde_json::Value::String(text) if sensitive_parent => {
            *text = REDACTED_MARKER.to_string();
        }
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    key == "authorization"
        || normalized.contains("apikey")
        || normalized.contains("accesstoken")
        || normalized.contains("refreshtoken")
        || normalized.contains("idtoken")
        || key == "token"
        || key.contains("secret")
        || key.contains("password")
        || key.contains("credential")
}

fn redact_inline_credentials(body: &str) -> String {
    let mut output = String::with_capacity(body.len());
    let mut index = 0;
    while index < body.len() {
        let remaining = &body[index..];
        if let Some(prefix_len) = ["Bearer ", "bearer ", "sk-", "sk_", "xoxb-"]
            .iter()
            .find_map(|prefix| remaining.starts_with(prefix).then_some(prefix.len()))
        {
            output.push_str(&remaining[..prefix_len]);
            index += prefix_len;
            let secret_end = remaining[prefix_len..]
                .find(|character: char| {
                    character.is_whitespace() || matches!(character, '"' | '\'' | ',' | '}' | ']')
                })
                .map_or(body.len(), |offset| index + offset);
            if secret_end > index {
                output.push_str(REDACTED_MARKER);
                index = secret_end;
                continue;
            }
        }

        let character = remaining
            .chars()
            .next()
            .expect("remaining is non-empty while redacting body");
        if character.is_control() {
            output.push(' ');
        } else {
            output.push(character);
        }
        index += character.len_utf8();
    }
    output
}

fn env_duration_ms(name: &str, default_ms: u64) -> Duration {
    duration_ms(std::env::var(name).ok().as_deref(), default_ms)
}

fn duration_ms(value: Option<&str>, default_ms: u64) -> Duration {
    value
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map_or_else(|| Duration::from_millis(default_ms), Duration::from_millis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn redacts_provider_credentials_without_discarding_diagnostic_message() {
        let body = r#"{"error":{"message":"deployment is unavailable","api-key":"secret-key","access-token":"access-secret"},"hint":"Bearer inline-secret"}"#;
        let redacted = redact_body(body, &["secret-key"]);

        assert!(redacted.contains("deployment is unavailable"));
        assert!(!redacted.contains("secret-key"));
        assert!(!redacted.contains("access-secret"));
        assert!(!redacted.contains("inline-secret"));
        assert!(!redacted.contains('\n'));
    }

    #[tokio::test]
    async fn error_reader_caps_and_marks_oversized_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(502)
                    .set_body_string("x".repeat(MAX_PROVIDER_ERROR_BODY_BYTES + 1_024)),
            )
            .mount(&server)
            .await;

        let response = reqwest::Client::new()
            .get(server.uri())
            .send()
            .await
            .expect("mock response");
        let body = read_provider_error_body(response, &[]).await;

        assert!(body.ends_with(BODY_TRUNCATION_MARKER));
        assert!(body.len() <= MAX_PROVIDER_ERROR_BODY_BYTES + BODY_TRUNCATION_MARKER.len());
    }

    #[tokio::test]
    async fn json_reader_rejects_oversized_body_before_parsing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("x".repeat(MAX_PROVIDER_RESPONSE_BODY_BYTES + 1)),
            )
            .mount(&server)
            .await;

        let response = reqwest::Client::new()
            .get(server.uri())
            .send()
            .await
            .expect("mock response");
        let error = read_provider_json::<serde_json::Value>(response, "test JSON", &[])
            .await
            .expect_err("oversized response must fail closed");

        assert!(error.to_string().contains("body exceeded"));
    }
}
