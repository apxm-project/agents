use apxm_backends::{
    LLMRequest, Message as LLMMessage, Role as LLMRole, StreamChunk, ToolChoice, ToolDefinition,
    llm::wire::response_metadata,
};
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::error::ApiError;
use crate::helpers::now_ms;
use crate::state::AppState;
use crate::types::responses::{
    SseEventMeta, StreamErrorBody, StreamLlmDonePayload, StreamTokenPayload, StreamToolCallPayload,
    StreamUsage, StreamUsagePayload,
};

// ─── LLM Generate Types ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct GenerateRequest {
    messages: Vec<MessagePayload>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_tokens: Option<usize>,
    #[serde(default)]
    tools: Option<Vec<ToolPayload>>,
    #[serde(default)]
    tool_choice: Option<String>,
    #[serde(default)]
    trace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessagePayload {
    role: String,
    content: JsonValue, // String or array of content parts
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolPayload {
    name: String,
    description: String,
    parameters: JsonValue,
}

#[derive(Debug, Serialize)]
pub(crate) struct GenerateResponse {
    content: String,
    model: String,
    finish_reason: String,
    usage: GenerateUsage,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<JsonValue>,
    #[serde(skip_serializing_if = "String::is_empty")]
    reasoning: String,
    trace_id: String,
}

fn reasoning_from_response(response: &apxm_backends::LLMResponse) -> String {
    response
        .metadata
        .get(response_metadata::REASONING)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

#[derive(Debug, Serialize)]
pub(crate) struct GenerateUsage {
    input_tokens: usize,
    output_tokens: usize,
    total_tokens: usize,
}

impl GenerateRequest {
    /// Convert the HTTP payload into an `LLMRequest` understood by the backend registry.
    fn to_llm_request(&self, trace_id: &str) -> LLMRequest {
        // Convert message payloads to backend Message types
        let messages: Vec<LLMMessage> = self
            .messages
            .iter()
            .map(|m| {
                let role = match m.role.as_str() {
                    "system" => LLMRole::System,
                    "assistant" => LLMRole::Assistant,
                    "tool" => LLMRole::Tool,
                    _ => LLMRole::User,
                };
                let text = match &m.content {
                    JsonValue::String(s) => s.clone(),
                    other => other.to_string(),
                };
                LLMMessage::text(role, text)
            })
            .collect();

        let mut request = LLMRequest::from_messages(messages);

        if let Some(model) = &self.model {
            request = request.with_model(model.clone());
        }
        if let Some(backend) = &self.backend {
            request = request.with_backend(backend.clone());
        }
        if let Some(temp) = self.temperature {
            request = request.with_temperature(temp);
        }
        if let Some(max) = self.max_tokens {
            request = request.with_max_tokens(max);
        }
        if let Some(tools) = &self.tools {
            let tool_defs: Vec<ToolDefinition> = tools
                .iter()
                .map(|t| ToolDefinition::new(&t.name, &t.description, t.parameters.clone()))
                .collect();
            request = request.with_tools(tool_defs);
            if let Some(choice) = &self.tool_choice {
                let normalized = choice.trim().to_ascii_lowercase();
                let lowered = match normalized.as_str() {
                    "auto" => ToolChoice::Auto,
                    "none" => ToolChoice::None,
                    "required" => ToolChoice::Required,
                    _ => ToolChoice::Specific(choice.to_string()),
                };
                request = request.with_tool_choice(lowered);
            }
        }
        request.trace_id = Some(trace_id.to_string());
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_tool_choice(tool_choice: Option<&str>) -> GenerateRequest {
        GenerateRequest {
            messages: vec![MessagePayload {
                role: "user".to_string(),
                content: JsonValue::String("Read the selected record when useful.".to_string()),
            }],
            model: Some("fixture-model".to_string()),
            backend: Some("mock".to_string()),
            temperature: None,
            max_tokens: None,
            tools: Some(vec![ToolPayload {
                name: "context.resolve".to_string(),
                description: "Resolve selected CLIC context.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                }),
            }]),
            tool_choice: tool_choice.map(str::to_string),
            trace_id: None,
        }
    }

    #[test]
    fn to_llm_request_forwards_auto_tool_choice() {
        let request = request_with_tool_choice(Some("auto")).to_llm_request("trace-tools");

        assert!(matches!(request.tool_choice, Some(ToolChoice::Auto)));
        assert_eq!(request.tools.as_ref().map(Vec::len), Some(1));
        assert_eq!(request.trace_id.as_deref(), Some("trace-tools"));
    }

    #[test]
    fn to_llm_request_forwards_named_tool_choice() {
        let request =
            request_with_tool_choice(Some("context.resolve")).to_llm_request("trace-tools");

        assert!(matches!(
            request.tool_choice,
            Some(ToolChoice::Specific(ref name)) if name == "context.resolve"
        ));
    }

    #[test]
    fn to_llm_request_normalizes_standard_tool_choice_names() {
        let request = request_with_tool_choice(Some(" Required ")).to_llm_request("trace-tools");

        assert!(matches!(request.tool_choice, Some(ToolChoice::Required)));
    }
}

fn extract_trace_id(headers: &HeaderMap, body: &GenerateRequest) -> String {
    headers
        .get("X-Trace-ID")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| body.trace_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

// ─── LLM Generate Handlers ──────────────────────────────────────────────────

/// `POST /v1/generate` — Non-streaming LLM generation.
pub(crate) async fn handle_generate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, ApiError> {
    let trace_id = extract_trace_id(&headers, &body);
    let request = body.to_llm_request(&trace_id);
    let registry = state.runtime.llm_registry();
    let _permit = state.inference_limiter.acquire().await?;

    let response = registry
        .generate(request)
        .await
        .map_err(|e| ApiError::internal_message(e.to_string()))?;

    let tool_calls: Vec<JsonValue> = response
        .tool_calls
        .iter()
        .map(|tc| {
            serde_json::json!({
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.args,
            })
        })
        .collect();

    let reasoning = reasoning_from_response(&response);
    Ok(Json(GenerateResponse {
        content: response.content,
        model: response.model,
        finish_reason: format!("{:?}", response.finish_reason),
        usage: GenerateUsage {
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            total_tokens: response.usage.total_tokens,
        },
        tool_calls,
        reasoning,
        trace_id,
    }))
}

#[derive(Serialize)]
struct Envelope<'a, T: Serialize> {
    meta: &'a SseEventMeta,
    payload: &'a T,
}

// Serializes meta+payload directly into a reusable buffer — avoids the per-chunk
// json! HashMap allocation and the to_string() double-copy on the SSE hot path.
fn encode_event<T: Serialize>(buf: &mut Vec<u8>, meta: &SseEventMeta, payload: &T) -> Event {
    buf.clear();
    let envelope = Envelope { meta, payload };
    let data = match serde_json::to_writer(&mut *buf, &envelope) {
        Ok(()) => String::from_utf8_lossy(buf).into_owned(),
        Err(_) => "{}".to_string(),
    };
    Event::default().event("apxm").data(data)
}

/// `POST /v1/generate-stream` — Streaming LLM generation via SSE.
pub(crate) async fn handle_generate_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GenerateRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let trace_id = extract_trace_id(&headers, &body);
    let request = body.to_llm_request(&trace_id);
    let registry = state.runtime.llm_registry().clone();
    let permit = state.inference_limiter.acquire().await?;

    let (tx, mut rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(128);

    tokio::spawn(async move {
        let _permit = permit;
        use std::sync::atomic::{AtomicU64, Ordering};
        let seq = AtomicU64::new(1);
        let trace = trace_id;

        let raw_stream = registry.generate_stream_with_fallback(&request);
        let mut pinned = std::pin::pin!(raw_stream);

        // 60-second inactivity timeout
        let timeout_dur = std::time::Duration::from_secs(60);

        // Reusable encode buffer — one allocation per stream, cleared per chunk.
        let mut buf: Vec<u8> = Vec::with_capacity(512);

        let make_meta = |seq_val: u64, trace: &str| SseEventMeta {
            seq: seq_val,
            timestamp_ms: now_ms(),
            trace_id: trace.to_string(),
            source: "backend",
        };

        loop {
            match tokio::time::timeout(timeout_dur, pinned.next()).await {
                Ok(Some(Ok(chunk))) => {
                    let seq_val = seq.fetch_add(1, Ordering::Relaxed);
                    let meta = make_meta(seq_val, &trace);
                    let maybe_event = match &chunk {
                        StreamChunk::Token(text) => Some(encode_event(
                            &mut buf,
                            &meta,
                            &StreamTokenPayload {
                                kind: "token",
                                text: text.clone(),
                            },
                        )),
                        StreamChunk::Thought(text) => Some(encode_event(
                            &mut buf,
                            &meta,
                            &StreamTokenPayload {
                                kind: "thought",
                                text: text.clone(),
                            },
                        )),
                        StreamChunk::ToolCallStart { id, name } => Some(encode_event(
                            &mut buf,
                            &meta,
                            &StreamToolCallPayload {
                                kind: "tool_call",
                                tool_call_id: id.clone(),
                                name: Some(name.clone()),
                                arguments_delta: None,
                                phase: "start",
                            },
                        )),
                        StreamChunk::ToolCallDelta {
                            id,
                            arguments_delta,
                        } => Some(encode_event(
                            &mut buf,
                            &meta,
                            &StreamToolCallPayload {
                                kind: "tool_call",
                                tool_call_id: id.clone(),
                                name: None,
                                arguments_delta: Some(arguments_delta.clone()),
                                phase: "delta",
                            },
                        )),
                        StreamChunk::Done(response) => Some(encode_event(
                            &mut buf,
                            &meta,
                            &StreamLlmDonePayload {
                                kind: "llm_done",
                                content: response.content.clone(),
                                model: response.model.clone(),
                                finish_reason: format!("{:?}", response.finish_reason),
                                usage: StreamUsage {
                                    input_tokens: response.usage.input_tokens,
                                    output_tokens: response.usage.output_tokens,
                                    total_tokens: response.usage.total_tokens,
                                },
                                reasoning: reasoning_from_response(response),
                            },
                        )),
                        StreamChunk::Usage(usage) => Some(encode_event(
                            &mut buf,
                            &meta,
                            &StreamUsagePayload {
                                kind: "usage",
                                usage: StreamUsage {
                                    input_tokens: usage.input_tokens,
                                    output_tokens: usage.output_tokens,
                                    total_tokens: usage.total_tokens,
                                },
                            },
                        )),
                        StreamChunk::Error(msg) => {
                            tracing::warn!(error = %msg, "Fatal streaming error from backend");
                            Some(
                                Event::default().event("error").data(
                                    serde_json::to_string(&StreamErrorBody {
                                        message: msg.clone(),
                                    })
                                    .unwrap_or_else(|_| "{}".to_string()),
                                ),
                            )
                        }
                    };
                    if let Some(event) = maybe_event {
                        if tx.send(Ok(event)).await.is_err() {
                            break;
                        }
                        if matches!(chunk, StreamChunk::Error(_)) {
                            break;
                        }
                    }
                }
                Ok(Some(Err(e))) => {
                    let body = StreamErrorBody {
                        message: e.to_string(),
                    };
                    let error_event = Event::default()
                        .event("error")
                        .data(serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()));
                    let _ = tx.send(Ok(error_event)).await;
                    break;
                }
                Ok(None) => {
                    break;
                }
                Err(_) => {
                    let body = StreamErrorBody {
                        message: "Stream timeout after 60s".to_string(),
                    };
                    let timeout_event = Event::default()
                        .event("error")
                        .data(serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()));
                    let _ = tx.send(Ok(timeout_event)).await;
                    break;
                }
            }
        }
    });

    let output_stream = async_stream::stream! {
        while let Some(item) = rx.recv().await {
            yield item;
        }
    };

    Ok(Sse::new(output_stream).keep_alive(KeepAlive::default()))
}

/// `GET /v1/schema` — Event schema endpoint.
pub(crate) async fn handle_schema() -> Json<JsonValue> {
    Json(serde_json::json!({
        "event_types": [
            "token",
            "thought",
            "tool_call",
            "llm_done",
            "usage",
            "retry",
            "warning",
            "citation",
            "provider_event",
            "error",
            "context_compacted",
            "model_rerouted",
            "cancelled",
            "loop_detected",
            "context_window_warning"
        ],
        "sse_format": {
            "event_name": "apxm",
            "data_format": "ApxmEvent JSON",
            "data_schema": {
                "meta": {
                    "seq": "u64 — monotonically increasing sequence number",
                    "timestamp_ms": "u64 — Unix milliseconds",
                    "trace_id": "string — correlates all events in a request",
                    "source": "string — originating component"
                },
                "payload": {
                    "kind": "string — one of the event_types above",
                    "...": "kind-specific fields"
                }
            },
            "error_event_name": "error",
            "error_schema": {
                "message": "string — human-readable error description"
            }
        }
    }))
}
