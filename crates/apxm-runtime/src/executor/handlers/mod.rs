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

/// Streaming variant of execute_llm_request.
///
/// Consumes the stream from generate_stream(), emitting LlmToken events
/// for each token chunk and returning the final LLMResponse from the Done chunk.
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

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| llm_error(ctx, phase, request, e))?;
        match chunk {
            StreamChunk::Token(token) => {
                if let Some(emitter) = &ctx.event_emitter {
                    emitter.emit_llm_token(&token);
                }
            }
            StreamChunk::Done(response) => {
                final_response = Some(response);
                break;
            }
            StreamChunk::ToolCallStart { .. } | StreamChunk::ToolCallDelta { .. } => {
                // Tool call streaming will be handled in a future iteration
            }
        }
    }

    let response = final_response.ok_or_else(|| RuntimeError::LLM {
        message: format!("LLM stream ended without a Done chunk during {phase}"),
        backend: request.backend.clone().or_else(|| request.model.clone()),
    })?;

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
