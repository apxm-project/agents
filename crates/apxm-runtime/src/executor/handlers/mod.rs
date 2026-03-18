//! Operation handlers for all AIS operation types

pub mod autonomous;
pub mod branch;
pub mod claim;
pub mod communicate;
pub mod const_str;
pub mod delegate;
pub mod err;
pub mod exc;
pub mod fence;
pub mod flow_call;
pub mod guard;
pub mod identity;
pub mod inner_plan;
pub mod inv;
pub mod jump;
pub mod llm; // Unified handler for Ask/Think/Reason operations
pub mod loop_end;
pub mod loop_start;
pub mod merge;
pub mod negotiate;
pub mod nop;
pub mod pause;
pub mod plan;
pub mod print;
pub mod qmem;
pub mod reflect;
pub mod register_capability;
pub mod resume; // Phase 1: RESUME
pub mod return_op;
pub mod spawn_agent;
pub mod switch;
pub mod try_catch;
pub mod umem;
pub mod update_goal;
pub mod verify;
pub mod wait_all;

use super::{ExecutionContext, Result};
use anyhow::Error as AnyhowError;
use apxm_backends::{LLMRequest, LLMResponse};
use apxm_core::{
    error::RuntimeError,
    types::{execution::Node, values::Value},
};

/// Helper to extract attribute from node
pub fn get_attribute(node: &Node, key: &str) -> Result<Value> {
    node.attributes
        .get(key)
        .cloned()
        .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Missing required attribute: {}", key),
        })
}

/// Helper to extract string attribute
pub fn get_string_attribute(node: &Node, key: &str) -> Result<String> {
    get_attribute(node, key)?
        .as_string()
        .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Attribute {} must be a string", key),
        })
        .map(|s| s.to_string())
}

/// Helper to extract optional string attribute
pub fn get_optional_string_attribute(node: &Node, key: &str) -> Result<Option<String>> {
    match node.attributes.get(key) {
        Some(value) => value
            .as_string()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("Attribute {} must be a string", key),
            }),
        None => Ok(None),
    }
}

/// Helper to extract optional u64 attribute
pub fn get_optional_u64_attribute(node: &Node, key: &str) -> Result<Option<u64>> {
    match node.attributes.get(key) {
        Some(value) => {
            value
                .as_u64()
                .map(Some)
                .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!("Attribute {} must be a number", key),
                })
        }
        None => Ok(None),
    }
}

/// Helper to get input by index
pub fn get_input(node: &Node, inputs: &[Value], index: usize) -> Result<Value> {
    inputs
        .get(index)
        .cloned()
        .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Missing input at index {}", index),
        })
}

/// Accumulate a finalized tool call from a `PendingToolCall`.
fn finalize_pending_tool_call(tc: PendingToolCall) -> apxm_core::types::ToolCall {
    let args: serde_json::Value =
        serde_json::from_str(&tc.arguments).unwrap_or(serde_json::Value::Null);
    apxm_core::types::ToolCall {
        id: tc.id,
        name: tc.name,
        args,
    }
}

/// Convert a low-level LLM backend error into a sanitized RuntimeError and emit tracing.
pub fn llm_error(
    ctx: &ExecutionContext,
    phase: &str,
    request: &LLMRequest,
    err: AnyhowError,
) -> RuntimeError {
    let backend_hint = request.backend.clone().or_else(|| request.model.clone());

    tracing::error!(
        execution_id = %ctx.execution_id,
        phase = phase,
        backend = backend_hint.as_deref().unwrap_or("auto"),
        error = %err,
        "LLM backend request failed"
    );

    let message = match backend_hint.as_deref() {
        Some(name) => format!(
            "LLM request failed during {phase} using '{name}'. Enable tracing logs for backend details."
        ),
        None => {
            format!("LLM request failed during {phase}. Enable tracing logs for backend details.")
        }
    };

    RuntimeError::LLM {
        message,
        backend: backend_hint,
    }
}

pub async fn execute_llm_request(
    ctx: &ExecutionContext,
    phase: &str,
    request: &LLMRequest,
) -> Result<LLMResponse> {
    // Use streaming path when an event emitter is available so we can
    // emit token-by-token events. The default generate_stream() impl
    // just wraps generate() into a single Done chunk, so this is
    // backward compatible.
    if ctx.event_emitter.is_some() {
        return execute_llm_request_streaming(ctx, phase, request).await;
    }

    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    let response = ctx
        .llm_registry
        .generate(request.clone())
        .await
        .map_err(|e| llm_error(ctx, phase, request, e))?;

    #[cfg(feature = "metrics")]
    {
        let latency = start.elapsed();
        record_llm_event(ctx, phase, request, &response, latency).await;
    }

    Ok(response)
}

/// A tool call being accumulated from streaming chunks.
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Streaming variant of execute_llm_request.
///
/// Consumes the stream from generate_stream(), emitting LlmToken events
/// for each token chunk. Tool calls arriving via `ToolCallStart` and
/// `ToolCallDelta` chunks are accumulated and dispatched mid-stream,
/// with results appended to the final response's tool_calls list.
async fn execute_llm_request_streaming(
    ctx: &ExecutionContext,
    phase: &str,
    request: &LLMRequest,
) -> Result<LLMResponse> {
    use apxm_backends::StreamChunk;
    use tokio_stream::StreamExt;

    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    let backend = ctx
        .llm_registry
        .resolve_backend_for_streaming(request)
        .map_err(|e| llm_error(ctx, phase, request, e))?;

    let mut stream = backend.generate_stream(request.clone());

    let mut final_response: Option<LLMResponse> = None;
    // Tool call accumulation state for mid-stream interleaving
    let mut pending_tool_call: Option<PendingToolCall> = None;
    let mut streamed_tool_calls: Vec<apxm_core::types::ToolCall> = Vec::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| llm_error(ctx, phase, request, e))?;
        match chunk {
            StreamChunk::Token(token) => {
                // If we had a pending tool call being accumulated, it's now
                // complete (the model moved on to producing text).
                if let Some(tc) = pending_tool_call.take() {
                    streamed_tool_calls.push(finalize_pending_tool_call(tc));
                }
                if let Some(emitter) = &ctx.event_emitter {
                    emitter.emit_llm_token(&token);
                }
            }
            StreamChunk::ToolCallStart { id, name } => {
                // Finalize any previous pending tool call before starting a new one.
                if let Some(tc) = pending_tool_call.take() {
                    streamed_tool_calls.push(finalize_pending_tool_call(tc));
                }
                pending_tool_call = Some(PendingToolCall {
                    id,
                    name,
                    arguments: String::new(),
                });
            }
            StreamChunk::ToolCallDelta { id: _, arguments_delta } => {
                if let Some(tc) = pending_tool_call.as_mut() {
                    tc.arguments.push_str(&arguments_delta);
                }
            }
            StreamChunk::Done(response) => {
                // Finalize any pending tool call.
                if let Some(tc) = pending_tool_call.take() {
                    streamed_tool_calls.push(finalize_pending_tool_call(tc));
                }
                final_response = Some(response);
                break;
            }
        }
    }

    let mut response = final_response.ok_or_else(|| RuntimeError::LLM {
        message: format!("LLM stream ended without a Done chunk during {phase}"),
        backend: request.backend.clone().or_else(|| request.model.clone()),
    })?;

    // Merge any tool calls accumulated from streaming into the response.
    // The Done chunk may already contain tool_calls (from backends that include
    // them in the final response); streaming-accumulated calls take precedence
    // when the Done chunk has none.
    if !streamed_tool_calls.is_empty() && response.tool_calls.is_empty() {
        response.tool_calls = streamed_tool_calls;
    }

    #[cfg(feature = "metrics")]
    {
        let latency = start.elapsed();
        record_llm_event(ctx, phase, request, &response, latency).await;
    }

    Ok(response)
}

#[cfg(feature = "metrics")]
async fn record_llm_event(
    ctx: &ExecutionContext,
    phase: &str,
    request: &LLMRequest,
    response: &LLMResponse,
    latency: std::time::Duration,
) {
    use apxm_core::types::values::{Number, Value};

    let backend = request
        .backend
        .clone()
        .or_else(|| request.model.clone())
        .unwrap_or_else(|| "auto".to_string());
    let mut fields = vec![
        ("phase".to_string(), Value::String(phase.to_string())),
        ("backend".to_string(), Value::String(backend)),
        ("model".to_string(), Value::String(response.model.clone())),
        (
            "latency_ms".to_string(),
            Value::Number(Number::from(latency.as_millis() as i64)),
        ),
        (
            "input_tokens".to_string(),
            Value::Number(Number::from(response.usage.input_tokens as i64)),
        ),
        (
            "output_tokens".to_string(),
            Value::Number(Number::from(response.usage.output_tokens as i64)),
        ),
        (
            "total_tokens".to_string(),
            Value::Number(Number::from(response.usage.total_tokens as i64)),
        ),
    ];

    if let Some(max_tokens) = request.max_tokens {
        fields.push((
            "max_tokens".to_string(),
            Value::Number(Number::from(max_tokens as i64)),
        ));
    }

    let _ = ctx
        .memory()
        .record_episodic_event(
            ctx.execution_id.clone(),
            "llm_call",
            Value::Object(fields.into_iter().collect()),
        )
        .await;
}

/// Extract JSON from a markdown fenced code block.
///
/// Looks for ` ```json ... ``` ` first, then bare ` ``` ... ``` ` blocks
/// whose content starts with `{` or `[`.
pub fn extract_json_from_markdown(content: &str) -> Option<String> {
    if let Some(start) = content.find("```json")
        && let Some(end) = content[start + 7..].find("```")
    {
        return Some(content[start + 7..start + 7 + end].trim().to_string());
    }

    if let Some(start) = content.find("```")
        && let Some(end) = content[start + 3..].find("```")
    {
        let extracted = content[start + 3..start + 3 + end].trim();
        if extracted.starts_with('{') || extracted.starts_with('[') {
            return Some(extracted.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finalize_pending_tool_call_valid_json() {
        let tc = PendingToolCall {
            id: "call_1".into(),
            name: "web_search".into(),
            arguments: r#"{"query":"rust async"}"#.into(),
        };
        let result = finalize_pending_tool_call(tc);
        assert_eq!(result.id, "call_1");
        assert_eq!(result.name, "web_search");
        assert_eq!(
            result.args,
            serde_json::json!({"query": "rust async"})
        );
    }

    #[test]
    fn test_finalize_pending_tool_call_invalid_json() {
        let tc = PendingToolCall {
            id: "call_2".into(),
            name: "broken".into(),
            arguments: "not json".into(),
        };
        let result = finalize_pending_tool_call(tc);
        assert_eq!(result.name, "broken");
        assert_eq!(result.args, serde_json::Value::Null);
    }

    #[test]
    fn test_finalize_pending_tool_call_empty_args() {
        let tc = PendingToolCall {
            id: "call_3".into(),
            name: "noop".into(),
            arguments: String::new(),
        };
        let result = finalize_pending_tool_call(tc);
        assert_eq!(result.args, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn test_streaming_tool_call_accumulation() {
        use apxm_backends::StreamChunk;
        use apxm_core::types::{FinishReason, TokenUsage};
        use tokio_stream::StreamExt;

        // Simulate a stream with interleaved tool calls
        let chunks: Vec<anyhow::Result<StreamChunk>> = vec![
            Ok(StreamChunk::Token("Let me ".into())),
            Ok(StreamChunk::Token("search ".into())),
            Ok(StreamChunk::ToolCallStart {
                id: "tc_1".into(),
                name: "web_search".into(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "tc_1".into(),
                arguments_delta: r#"{"query""#.into(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "tc_1".into(),
                arguments_delta: r#":"rust"}"#.into(),
            }),
            Ok(StreamChunk::ToolCallStart {
                id: "tc_2".into(),
                name: "read_file".into(),
            }),
            Ok(StreamChunk::ToolCallDelta {
                id: "tc_2".into(),
                arguments_delta: r#"{"path":"main.rs"}"#.into(),
            }),
            Ok(StreamChunk::Done(LLMResponse::new(
                "results".to_string(),
                "mock".to_string(),
                TokenUsage::new(10, 20),
                FinishReason::ToolUse,
            ))),
        ];

        let mut stream = tokio_stream::iter(chunks);
        let mut pending: Option<PendingToolCall> = None;
        let mut collected: Vec<apxm_core::types::ToolCall> = Vec::new();
        let mut final_resp: Option<LLMResponse> = None;

        while let Some(Ok(chunk)) = stream.next().await {
            match chunk {
                StreamChunk::Token(_) => {
                    if let Some(tc) = pending.take() {
                        collected.push(finalize_pending_tool_call(tc));
                    }
                }
                StreamChunk::ToolCallStart { id, name } => {
                    if let Some(tc) = pending.take() {
                        collected.push(finalize_pending_tool_call(tc));
                    }
                    pending = Some(PendingToolCall {
                        id,
                        name,
                        arguments: String::new(),
                    });
                }
                StreamChunk::ToolCallDelta { arguments_delta, .. } => {
                    if let Some(tc) = pending.as_mut() {
                        tc.arguments.push_str(&arguments_delta);
                    }
                }
                StreamChunk::Done(resp) => {
                    if let Some(tc) = pending.take() {
                        collected.push(finalize_pending_tool_call(tc));
                    }
                    final_resp = Some(resp);
                }
            }
        }

        assert!(final_resp.is_some());
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0].name, "web_search");
        assert_eq!(collected[0].args, serde_json::json!({"query": "rust"}));
        assert_eq!(collected[1].name, "read_file");
        assert_eq!(collected[1].args, serde_json::json!({"path": "main.rs"}));
    }
}
