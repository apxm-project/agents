//! Operation handlers for all AIS operation types

pub mod autonomous;
pub mod branch;
pub mod checkpoint;
pub mod claim;
pub mod communicate;
pub mod const_str;
pub mod delegate;
pub mod err;
pub mod exc;
pub mod fence;
pub mod flow_call;
pub mod guard;
pub mod handoff;
pub mod identity;
pub mod inner_plan;
pub mod inv_tool;
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
pub mod spawn_team;
pub mod switch;
pub mod template;
pub mod try_catch;
pub mod umem;
pub mod update_goal;
pub mod verify;
pub mod wait_all;
#[allow(dead_code)]
pub mod warmup;
pub mod workflow_spawn;

use super::{ExecutionContext, Result};
use anyhow::Error as AnyhowError;
use apxm_backends::{LLMRequest, LLMResponse};
use apxm_core::{
    constants::graph::attrs as graph_attrs,
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

/// Apply per-node LLM routing hints to a request without conflating backend and model.
pub fn apply_llm_request_routing_from_node(
    mut request: LLMRequest,
    node: &Node,
) -> Result<LLMRequest> {
    if let Some(backend) = get_optional_string_attribute(node, graph_attrs::BACKEND)? {
        request = request.with_backend(backend);
    }
    if let Some(model) = get_optional_string_attribute(node, graph_attrs::MODEL)? {
        request = request.with_model(model);
    }
    Ok(request)
}

/// Preserve the routing identity of an LLM request across retries or continuations.
pub fn copy_llm_request_routing(mut request: LLMRequest, source: &LLMRequest) -> LLMRequest {
    if let Some(operation) = source.operation_type {
        request = request.with_operation_type(operation);
    }
    if let Some(backend) = &source.backend {
        request = request.with_backend(backend.clone());
    }
    if let Some(model) = &source.model {
        request = request.with_model(model.clone());
    }
    if let Some(extra_body) = &source.extra_body {
        request = request.with_extra_body(extra_body.clone());
    }
    request
}

/// Read an STM key looking in the parent (flow-root) scope first, then the
/// current scope. SPAWN_AGENT writes `_agent_info:<name>` to the parent scope
/// so sibling worker scopes (created per node by `scheduler/worker.rs`) can
/// see it; this helper centralizes the lookup for HANDOFF and COMMUNICATE
/// inline-agent fallbacks.
pub async fn read_stm_with_scope_fallback(ctx: &ExecutionContext, key: &str) -> Option<Value> {
    use crate::metadata_keys as metadata;
    let parent_scope = ctx.metadata.get(metadata::PARENT_SCOPE_ID).cloned();
    let primary = parent_scope.as_deref().unwrap_or_else(|| ctx.scope_id());
    if let Ok(Some(v)) = ctx
        .memory
        .read_scoped(crate::memory::MemorySpace::Stm, primary, key)
        .await
    {
        return Some(v);
    }
    if parent_scope.is_some() {
        if let Ok(Some(v)) = ctx
            .memory
            .read_scoped(crate::memory::MemorySpace::Stm, ctx.scope_id(), key)
            .await
        {
            return Some(v);
        }
    }
    None
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

pub async fn execute_llm_request_for_node(
    ctx: &ExecutionContext,
    node: &Node,
    phase: &str,
    request: &LLMRequest,
) -> Result<LLMResponse> {
    execute_llm_request_with_node_name(ctx, node.id, node.metadata.name.as_deref(), phase, request)
        .await
}

async fn execute_llm_request_with_node_name(
    ctx: &ExecutionContext,
    node_id: u64,
    node_name: Option<&str>,
    phase: &str,
    request: &LLMRequest,
) -> Result<LLMResponse> {
    // Use streaming path when an event emitter is available so we can
    // emit token-by-token events. The default generate_stream() impl
    // wraps generate() into a single Done chunk for non-streaming backends.
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_llm_prompt_with_name(node_id, node_name, &request.prompt);
        return execute_llm_request_streaming(ctx, node_id, phase, request).await;
    }

    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    // Route through ModelRouter when available (circuit breakers + policy routing).
    let response = if let Some(router) = &ctx.model_router {
        router
            .generate(request.clone())
            .await
            .map_err(|e| llm_error(ctx, phase, request, e))?
    } else {
        ctx.llm_registry
            .generate(request.clone())
            .await
            .map_err(|e| llm_error(ctx, phase, request, e))?
    };

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
    node_id: u64,
    phase: &str,
    request: &LLMRequest,
) -> Result<LLMResponse> {
    use apxm_backends::StreamChunk;
    use tokio_stream::StreamExt;

    let start = std::time::Instant::now();

    // When a ModelRouter is present, let it choose backend/model first.
    let (router_decision, prepared_request) = if let Some(router) = &ctx.model_router {
        let decision = router
            .select_for_dispatch(request)
            .await
            .map_err(|e| llm_error(ctx, phase, request, e))?;

        let mut req = request.clone();
        req.backend = Some(decision.backend.clone());
        if let Some(ref m) = decision.model {
            req.model = Some(m.clone());
        }

        (Some(decision), ctx.llm_registry.prepare_request(&req))
    } else {
        (None, ctx.llm_registry.prepare_request(request))
    };

    let backend = ctx
        .llm_registry
        .resolve_backend_for_streaming(&prepared_request)
        .map_err(|e| llm_error(ctx, phase, &prepared_request, e))?;

    let resolved_backend_name = ctx
        .llm_registry
        .resolved_backend_name(&prepared_request)
        .unwrap_or_else(|_| {
            prepared_request
                .backend
                .clone()
                .or_else(|| prepared_request.model.clone())
                .unwrap_or_else(|| "auto".to_string())
        });
    let resolved_backend_model = backend.model().to_string();

    let mut stream = backend.generate_stream(prepared_request.clone());

    let mut final_response: Option<LLMResponse> = None;
    let mut emitted_text = false;
    // Tool call accumulation state for mid-stream interleaving
    let mut pending_tool_call: Option<PendingToolCall> = None;
    let mut streamed_tool_calls: Vec<apxm_core::types::ToolCall> = Vec::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(e) => {
                ctx.llm_registry.record_streaming_outcome(
                    &resolved_backend_name,
                    &resolved_backend_model,
                    start.elapsed(),
                    None,
                    false,
                );
                return Err(llm_error(ctx, phase, &prepared_request, e));
            }
        };
        match chunk {
            StreamChunk::Token(token) => {
                // If we had a pending tool call being accumulated, it's now
                // complete (the model moved on to producing text).
                if let Some(tc) = pending_tool_call.take() {
                    streamed_tool_calls.push(finalize_pending_tool_call(tc));
                }
                if let Some(emitter) = &ctx.event_emitter {
                    emitter.emit_llm_token_for_node(node_id, &token);
                    emitted_text = true;
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
            StreamChunk::ToolCallDelta {
                id: _,
                arguments_delta,
            } => {
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
            StreamChunk::Thought(thought) => {
                // Extended thinking tokens — emit as LLM token for now
                if let Some(emitter) = &ctx.event_emitter {
                    emitter.emit_llm_token_for_node(node_id, &thought);
                    emitted_text = true;
                }
            }
            StreamChunk::Usage(_usage) => {
                // Incremental usage update — final usage comes in Done chunk
            }
            StreamChunk::Error(msg) => {
                // Non-fatal stream error — log and continue
                tracing::warn!(error = %msg, "Non-fatal stream error during {}", phase);
            }
        }
    }

    let mut response = match final_response {
        Some(resp) => resp,
        None => {
            ctx.llm_registry.record_streaming_outcome(
                &resolved_backend_name,
                &resolved_backend_model,
                start.elapsed(),
                None,
                false,
            );
            return Err(RuntimeError::LLM {
                message: format!("LLM stream ended without a Done chunk during {phase}"),
                backend: request.backend.clone().or_else(|| request.model.clone()),
            });
        }
    };

    // Merge any tool calls accumulated from streaming into the response.
    // The Done chunk may already contain tool_calls (from backends that include
    // them in the final response); streaming-accumulated calls take precedence
    // when the Done chunk has none.
    if !streamed_tool_calls.is_empty() && response.tool_calls.is_empty() {
        response.tool_calls = streamed_tool_calls;
    }

    if !emitted_text && !response.content.is_empty() {
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_llm_token_for_node(node_id, &response.content);
        }
    }

    let latency = start.elapsed();

    ctx.llm_registry.record_streaming_outcome(
        &resolved_backend_name,
        &resolved_backend_model,
        latency,
        Some(response.usage.clone()),
        true,
    );

    #[cfg(feature = "metrics")]
    {
        record_llm_event(ctx, phase, request, &response, latency).await;
    }

    // Record success in ModelRouter circuit breaker (streaming path).
    if let Some(router) = &ctx.model_router {
        if let Some(ref decision) = router_decision {
            router.record_success(&decision.backend);
        }
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
            None, // node_id not available in this context
            None, // session_dir not available in this context
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
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::executor::events::ExecutionEventEmitter;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::llm::backends::mock::MockLLMBackend;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

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
        assert_eq!(result.args, serde_json::json!({"query": "rust async"}));
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
                StreamChunk::ToolCallDelta {
                    arguments_delta, ..
                } => {
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
                StreamChunk::Thought(_) | StreamChunk::Usage(_) | StreamChunk::Error(_) => {
                    // Informational chunks — not relevant for tool call parsing
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

    #[derive(Default)]
    struct RecordingEmitter {
        events: Mutex<Vec<String>>,
    }

    impl RecordingEmitter {
        fn snapshot(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl ExecutionEventEmitter for RecordingEmitter {
        fn emit_llm_token(&self, content: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("token:global:{content}"));
        }

        fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}

        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_llm_prompt(&self, node_id: u64, prompt: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("prompt:{node_id}:{prompt}"));
        }

        fn emit_llm_prompt_with_name(&self, node_id: u64, node_name: Option<&str>, prompt: &str) {
            self.events.lock().unwrap().push(format!(
                "prompt:{node_id}:{}:{prompt}",
                node_name.unwrap_or("")
            ));
        }

        fn emit_llm_token_for_node(&self, node_id: u64, content: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("token:{node_id}:{content}"));
        }
    }

    #[tokio::test]
    async fn execute_llm_request_emits_prompt_before_streamed_tokens() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        llm_registry
            .register("mock", MockLLMBackend::static_response("alpha beta"))
            .unwrap();
        llm_registry.set_default("mock").unwrap();

        let emitter = Arc::new(RecordingEmitter::default());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            Arc::new(CapabilitySystem::new()),
            Aam::new(),
        )
        .with_event_emitter(Some(emitter.clone()));

        let node = Node {
            id: 7,
            op_type: apxm_core::types::operations::AISOperationType::Ask,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata {
                name: Some("ask_node".to_string()),
                ..Default::default()
            },
        };

        let response =
            execute_llm_request_for_node(&ctx, &node, "ask", &LLMRequest::new("plan this"))
                .await
                .unwrap();

        assert_eq!(response.content, "alpha beta");

        let events = emitter.snapshot();
        assert!(!events.is_empty());
        assert_eq!(events[0], "prompt:7:ask_node:plan this");
        assert!(events.iter().any(|event| event == "token:7:alpha "));
        assert!(events.iter().any(|event| event == "token:7:beta "));
    }
}
