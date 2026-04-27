//! Streaming chat completion endpoint.
//!
//! Proxies chat requests to the configured LLM backend (OpenAI-compatible API)
//! and streams tokens back via SSE.

use std::convert::Infallible;
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use futures::StreamExt;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub stream: bool,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

fn default_model() -> String {
    "claude-sonnet-4-5@20250929".to_string()
}

fn default_temperature() -> f32 {
    0.7
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
struct ModelInfo {
    id: String,
    aliases: Vec<String>,
}

// ---------------------------------------------------------------------------
// Config helpers
// ---------------------------------------------------------------------------

struct BackendConfig {
    endpoint: String,
    api_key: String,
    headers: Vec<(String, String)>,
}

fn load_backend_config() -> Result<BackendConfig, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let path = std::path::Path::new(&home).join(".apxm/config.toml");
    let content = std::fs::read_to_string(&path).map_err(|e| format!("cannot read config: {e}"))?;

    // Use line-based extraction for endpoint, api_key, and headers
    // since the strict TOML parser may fail on this config file.
    let mut endpoint = String::new();
    let mut api_key = String::new();
    let mut headers = Vec::new();
    let mut in_first_backend = false;
    let mut in_headers = false;
    let mut found_backend = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        if trimmed == "[[backends]]" {
            if found_backend {
                break;
            } // only use the first backend
            in_first_backend = true;
            found_backend = true;
            in_headers = false;
            continue;
        }

        if !in_first_backend {
            continue;
        }

        if trimmed == "[backends.headers]" {
            in_headers = true;
            continue;
        }

        if trimmed.starts_with('[') {
            if trimmed == "[[backends.models]]" {
                in_headers = false;
                continue;
            }
            if trimmed.starts_with("[[backends") || trimmed.starts_with("[backends.") {
                in_headers = false;
                continue;
            }
            break; // different top-level section
        }

        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim();
            let val = trimmed[eq_pos + 1..].trim().trim_matches('"');

            if in_headers {
                let resolved = if let Some(var_name) =
                    val.strip_prefix(apxm_backends::llm::wire::config_keys::ENV_PREFIX)
                {
                    std::env::var(var_name).unwrap_or_default()
                } else {
                    val.to_string()
                };
                headers.push((key.to_string(), resolved));
            } else {
                match key {
                    "endpoint" => endpoint = val.to_string(),
                    "api_key" => {
                        api_key = if let Some(var_name) =
                            val.strip_prefix(apxm_backends::llm::wire::config_keys::ENV_PREFIX)
                        {
                            std::env::var(var_name).unwrap_or_default()
                        } else {
                            val.to_string()
                        };
                    }
                    _ => {}
                }
            }
        }
    }

    if endpoint.is_empty() {
        return Err("no backend endpoint found in config".to_string());
    }

    Ok(BackendConfig {
        endpoint,
        api_key,
        headers,
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/chat/models — list available models for the chat dropdown.
/// Reuses the same config extraction logic as the backends handler.
pub async fn models_handler() -> impl IntoResponse {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let path = std::path::Path::new(&home).join(".apxm/config.toml");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("cannot read config: {e}") })),
            )
                .into_response();
        }
    };

    let backends = crate::extract_backends_from_config(&content);
    let mut models = Vec::new();

    for backend in &backends {
        if let Some(backend_models) = backend.get("models").and_then(|m| m.as_array()) {
            for m in backend_models {
                let id = m
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let aliases: Vec<String> = m
                    .get("aliases")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                if !id.is_empty() {
                    models.push(ModelInfo { id, aliases });
                }
            }
        }
    }

    Json(serde_json::json!({ "models": models })).into_response()
}

/// POST /api/chat — streaming chat completion via SSE.
pub async fn chat_handler(
    axum::extract::Json(req): axum::extract::Json<ChatRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let config = load_backend_config().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
    })?;

    let url = format!("{}/chat/completions", config.endpoint.trim_end_matches('/'));

    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
        .collect();

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
        "temperature": req.temperature,
        "stream": true,
    });

    if let Some(max_tokens) = req.max_tokens {
        body.as_object_mut()
            .unwrap()
            .insert("max_tokens".to_string(), serde_json::json!(max_tokens));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("http client: {e}") })),
            )
        })?;

    let mut request = client
        .post(&url)
        .header(
            apxm_backends::llm::wire::headers::CONTENT_TYPE,
            apxm_backends::llm::wire::headers::CONTENT_TYPE_JSON,
        )
        .json(&body);
    if !config.api_key.is_empty() {
        request = request.header(
            apxm_backends::llm::wire::headers::AUTHORIZATION,
            format!("Bearer {}", config.api_key),
        );
    }

    for (k, v) in &config.headers {
        request = request.header(k.as_str(), v.as_str());
    }

    let response = request.send().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("LLM request failed: {e}") })),
        )
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("LLM returned {status}: {text}") })),
        ));
    }

    let stream = async_stream::stream! {
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk_result) = byte_stream.next().await {
            match chunk_result {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].trim().to_string();
                        buffer = buffer[newline_pos + 1..].to_string();

                        if line.is_empty() || line.starts_with("event:") { continue; }
                        if line == "data: [DONE]" {
                            yield Ok::<Event, Infallible>(Event::default().event("done").data("[DONE]"));
                            continue;
                        }
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                                // Handle Anthropic SSE format: content_block_delta
                                if let Some(event_type) = parsed.get("type").and_then(|t| t.as_str()) {
                                    match event_type {
                                        "content_block_delta" => {
                                            let token = parsed
                                                .get("delta")
                                                .and_then(|d| d.get("text"))
                                                .and_then(|t| t.as_str())
                                                .unwrap_or("");
                                            if !token.is_empty() {
                                                yield Ok::<Event, Infallible>(
                                                    Event::default()
                                                        .event("token")
                                                        .data(serde_json::json!({ "token": token }).to_string())
                                                );
                                            }
                                        }
                                        "message_delta" => {
                                            let stop = parsed
                                                .get("delta")
                                                .and_then(|d| d.get("stop_reason"))
                                                .and_then(|r| r.as_str());
                                            if stop.is_some() {
                                                yield Ok::<Event, Infallible>(
                                                    Event::default()
                                                        .event("done")
                                                        .data(serde_json::json!({ "finish_reason": stop }).to_string())
                                                );
                                            }
                                        }
                                        _ => {} // ping, message_start, content_block_start/stop
                                    }
                                    continue;
                                }

                                // Handle OpenAI SSE format: choices[0].delta.content
                                let token = parsed
                                    .get("choices")
                                    .and_then(|c| c.as_array())
                                    .and_then(|c| c.first())
                                    .and_then(|c| c.get("delta"))
                                    .and_then(|d| d.get("content"))
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("");

                                if !token.is_empty() {
                                    yield Ok::<Event, Infallible>(
                                        Event::default()
                                            .event("token")
                                            .data(serde_json::json!({ "token": token }).to_string())
                                    );
                                }

                                let finish = parsed
                                    .get("choices")
                                    .and_then(|c| c.as_array())
                                    .and_then(|c| c.first())
                                    .and_then(|c| c.get("finish_reason"))
                                    .and_then(|r| r.as_str());

                                if finish.is_some() {
                                    yield Ok::<Event, Infallible>(
                                        Event::default()
                                            .event("done")
                                            .data(serde_json::json!({ "finish_reason": finish }).to_string())
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    yield Ok::<Event, Infallible>(
                        Event::default()
                            .event("error")
                            .data(serde_json::json!({ "error": e.to_string() }).to_string())
                    );
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}
