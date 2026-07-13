//! Operation handlers for all AIS operation types

pub mod autonomous;
pub mod branch;
pub mod call_skill;
pub mod checkpoint;
pub mod communicate;
pub mod const_str;
pub mod delegate;
pub mod err;
pub mod exc;
pub mod fence;
pub mod flow_call;
pub mod handoff;
pub mod identity;
pub mod inner_plan;
pub mod inv_cap;
pub mod jump;
pub mod llm; // Unified handler for Ask/Think/Reason operations
pub mod merge;
pub mod nop;
pub mod pause;
pub mod plan;
pub mod print;
pub mod qmem;
pub mod reflect;
pub mod register_capability;
pub mod register_hook;
pub mod resume; // RESUME handler
pub mod return_op;
pub mod spawn_agent;
pub mod switch;
pub mod target_resolution;
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
use crate::model_router::{ProfileRouter, RoutingDecision};
use anyhow::Error as AnyhowError;
use apxm_backends::llm::wire::response_metadata;
use apxm_backends::{LLMRequest, LLMResponse, StreamingBackendError};
use apxm_core::{
    constants::graph::attrs as graph_attrs,
    error::RuntimeError,
    events::payload::GenerationIdentity,
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
    if let Some(profile) = get_optional_string_attribute(node, graph_attrs::MODEL_PROFILE)? {
        request = request.with_model_profile(profile);
    }
    Ok(request)
}

/// Resolve a node's `model_profile` (or, absent that, the execution's
/// package-level default profile) into a concrete `model` on the request,
/// narrowing the candidate the normal `ModelRouter::select` ranks over.
///
/// Runs **before** `ModelRouter::select`/`select_for_dispatch` so profile
/// resolution always happens first: profiles resolve to candidates, then
/// `ModelRouter::select` ranks them.
///
/// Precedence (explicit wins, platform.md rule 1):
/// 1. Request already carries an explicit `backend` or `model` (from a
///    node-level `BACKEND`/`MODEL` attribute, or a prior resolution) — no
///    profile lookup, unchanged.
/// 2. Node-level `model_profile` attribute.
/// 3. `ExecutionContext::default_model_profile` — the owning package's
///    `agent.toml [runtime].default_model_profile`, when threaded in by the
///    host loading the package.
/// 4. Neither router nor registry attached, or profile has no healthy
///    candidate — falls through unchanged so `ModelRouter::select` still
///    runs its normal full-candidate-set routing rather than failing the
///    request outright.
pub fn resolve_model_profile(ctx: &ExecutionContext, mut request: LLMRequest) -> LLMRequest {
    if request.backend.is_some() || request.model.is_some() {
        return request;
    }

    let Some(profile_name) = request
        .model_profile
        .clone()
        .or_else(|| ctx.default_model_profile.clone())
    else {
        return request;
    };

    let (Some(model_router), Some(profile_registry)) = (&ctx.model_router, &ctx.profile_registry)
    else {
        tracing::debug!(
            profile = %profile_name,
            "model_profile set but ModelRouter/ProfileRegistry unavailable; \
             falling back to normal routing"
        );
        return request;
    };

    let profile_router = ProfileRouter::new(
        profile_registry,
        model_router,
        model_router.model_registry(),
    );
    match profile_router.select_from_profile(&profile_name) {
        Ok(model) => {
            tracing::debug!(
                profile = %profile_name,
                model = %model,
                "Resolved model_profile to candidate model before ModelRouter::select"
            );
            request = request.with_model(model);
        }
        Err(e) => {
            tracing::warn!(
                profile = %profile_name,
                error = %e,
                "model_profile resolution failed; falling back to full candidate set"
            );
        }
    }
    request
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
    if let Some(profile) = &source.model_profile {
        request = request.with_model_profile(profile.clone());
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
    execute_llm_request_for_node_with_generation(ctx, node, phase, request, None).await
}

pub async fn execute_llm_request_for_node_with_generation(
    ctx: &ExecutionContext,
    node: &Node,
    phase: &str,
    request: &LLMRequest,
    generation: Option<&GenerationIdentity>,
) -> Result<LLMResponse> {
    execute_llm_request_with_node_name(
        ctx,
        node.id,
        node.metadata.name.as_deref(),
        phase,
        request,
        generation,
    )
    .await
}

async fn execute_llm_request_with_node_name(
    ctx: &ExecutionContext,
    node_id: u64,
    node_name: Option<&str>,
    phase: &str,
    request: &LLMRequest,
    generation: Option<&GenerationIdentity>,
) -> Result<LLMResponse> {
    if ctx.cancellation_token.is_cancelled() {
        return Err(RuntimeError::SchedulerCancelled);
    }

    // Resolve model_profile -> candidate model before either dispatch
    // path runs ModelRouter::select. No-op when the request already carries
    // an explicit backend/model or declares no profile.
    let resolved = resolve_model_profile(ctx, request.clone());
    let request = &resolved;

    let active_agent = ctx
        .agent_scope_stack
        .peek()
        .map(|scope| scope.agent_code.clone())
        .or_else(|| ctx.current_agent.as_ref().map(|agent| agent.name.clone()));

    if let (Some(emitter), Some(agent_code)) = (&ctx.event_emitter, active_agent.as_deref()) {
        emitter.emit_subagent_llm_call_begin_with_generation(
            agent_code,
            request.model.as_deref().unwrap_or("auto"),
            request.backend.as_deref().unwrap_or("auto"),
            request.tools.as_ref().map_or(0, Vec::len),
            generation,
        );
    }

    // Use streaming path when an event emitter is available so we can
    // emit token-by-token events. The default generate_stream() impl
    // wraps generate() into a single Done chunk for non-streaming backends.
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_llm_prompt_with_generation(node_id, node_name, &request.prompt, generation);
    }

    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    // Route through ModelRouter when available (circuit breakers + policy routing).
    let response = if ctx.event_emitter.is_some() {
        execute_llm_request_streaming(ctx, node_id, phase, request, generation).await?
    } else if let Some(router) = &ctx.model_router {
        tokio::select! {
            result = router.generate(request.clone()) => {
                result.map_err(|e| llm_error(ctx, phase, request, e))?
            }
            _ = ctx.cancellation_token.cancelled() => {
                return Err(RuntimeError::SchedulerCancelled);
            }
        }
    } else {
        tokio::select! {
            result = ctx.llm_registry.generate(request.clone()) => {
                result.map_err(|e| llm_error(ctx, phase, request, e))?
            }
            _ = ctx.cancellation_token.cancelled() => {
                return Err(RuntimeError::SchedulerCancelled);
            }
        }
    };

    if let (Some(emitter), Some(agent_code)) = (&ctx.event_emitter, active_agent.as_deref()) {
        emitter.emit_subagent_llm_call_end_with_generation(
            agent_code,
            &response.finish_reason.to_string(),
            response.usage.input_tokens,
            response.usage.output_tokens,
            response.content.chars().count(),
            generation,
        );
    }

    #[cfg(feature = "metrics")]
    {
        let latency = start.elapsed();
        record_llm_event(ctx, phase, request, &response, latency).await;
    }

    Ok(response)
}

/// Forward a [`RoutingDecision`] to the execution event emitter.
/// Shared helper so model-routing observability stays identical regardless
/// of call site.
fn emit_model_route_decision_event(
    emitter: &dyn crate::executor::events::ExecutionEventEmitter,
    decision: &RoutingDecision,
) {
    let rejected: Vec<(String, String, &'static str, String)> = decision
        .rejected_candidates
        .iter()
        .map(|rejection| {
            (
                rejection.candidate.clone(),
                rejection.backend.clone(),
                rejection.reason_kind.as_str(),
                rejection.reason.clone(),
            )
        })
        .collect();
    emitter.emit_model_route_decision(
        &decision.backend,
        decision.model.as_deref(),
        decision.was_failover,
        &decision.reason,
        &rejected,
    );
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
    generation: Option<&GenerationIdentity>,
) -> Result<LLMResponse> {
    use apxm_backends::StreamChunk;
    use tokio_stream::StreamExt;

    #[cfg(feature = "metrics")]
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

        // make the routing decision observable — a rollout event
        // carrying the chosen backend/model, why, and every rejected
        // candidate with its own reason (never silently dropped).
        if let Some(emitter) = &ctx.event_emitter {
            emit_model_route_decision_event(emitter.as_ref(), &decision);
        }

        (Some(decision), ctx.llm_registry.prepare_request(&req))
    } else {
        (None, ctx.llm_registry.prepare_request(request))
    };

    let mut stream = ctx
        .llm_registry
        .generate_stream_with_fallback(&prepared_request);

    let mut final_response: Option<LLMResponse> = None;
    let mut emitted_text = false;
    // Tool call accumulation state for mid-stream interleaving
    let mut pending_tool_call: Option<PendingToolCall> = None;
    let mut streamed_tool_calls: Vec<apxm_core::types::ToolCall> = Vec::new();

    loop {
        let chunk_result = tokio::select! {
            chunk_result = stream.next() => chunk_result,
            _ = ctx.cancellation_token.cancelled() => {
                return Err(RuntimeError::SchedulerCancelled);
            }
        };
        let Some(chunk_result) = chunk_result else {
            break;
        };
        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(e) => {
                if let Some(router) = &ctx.model_router
                    && let Some(ref decision) = router_decision
                {
                    let backend = streaming_error_backend(&e, &decision.backend).to_string();
                    router.record_failure(&backend);
                }
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
                    emitter.emit_llm_token_for_generation(node_id, &token, generation);
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
                // Extended thinking — emit as a distinct `thought` event so
                // clients render reasoning apart from the answer. Not counted as
                // answer text (`emitted_text`): the answer comes from `token`
                // chunks and the final response content.
                if let Some(emitter) = &ctx.event_emitter {
                    emitter.emit_llm_thought_for_generation(node_id, &thought, generation);
                }
            }
            StreamChunk::Usage(_usage) => {
                // Incremental usage update — final usage comes in Done chunk
            }
            StreamChunk::Error(msg) => {
                if let Some(router) = &ctx.model_router
                    && let Some(ref decision) = router_decision
                {
                    router.record_failure(&decision.backend);
                }
                return Err(RuntimeError::LLM {
                    message: format!("LLM stream error during {phase}: {msg}"),
                    backend: prepared_request
                        .backend
                        .clone()
                        .or_else(|| prepared_request.model.clone()),
                });
            }
        }
    }

    let mut response = match final_response {
        Some(resp) => resp,
        None => {
            if let Some(router) = &ctx.model_router
                && let Some(ref decision) = router_decision
            {
                router.record_failure(&decision.backend);
            }
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
            emitter.emit_llm_token_for_generation(node_id, &response.content, generation);
        }
    }

    #[cfg(feature = "metrics")]
    {
        let latency = start.elapsed();
        record_llm_event(ctx, phase, request, &response, latency).await;
    }

    // Record success in ModelRouter circuit breaker (streaming path).
    if let Some(router) = &ctx.model_router {
        if let Some(ref decision) = router_decision {
            router.record_success(streamed_response_backend(&response, &decision.backend));
        }
    }

    Ok(response)
}

fn streamed_response_backend<'a>(response: &'a LLMResponse, fallback_backend: &'a str) -> &'a str {
    response
        .metadata
        .get(response_metadata::APXM_BACKEND_NAME)
        .and_then(serde_json::Value::as_str)
        .unwrap_or(fallback_backend)
}

fn streaming_error_backend<'a>(error: &'a AnyhowError, fallback_backend: &'a str) -> &'a str {
    error
        .downcast_ref::<StreamingBackendError>()
        .map(StreamingBackendError::backend_name)
        .unwrap_or(fallback_backend)
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
mod model_profile_routing_tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use crate::model_router::registry::{ModelEntry, ModelRegistry};
    use crate::model_router::{ModelRouter, ModelRouterConfig, ProfileRegistry, RoutingTarget};
    use apxm_backends::LLMRegistry;
    use apxm_backends::llm::backends::MockLLMBackend;
    use apxm_core::model_profiles::{ModelProfile, ProfileCandidate};
    use apxm_core::types::AISOperationType;
    use std::sync::Arc;

    /// Two backends ("cheap", "expensive") with sharply different costs, so a
    /// `Cost`-targeted `select` would normally prefer "cheap" — proving that
    /// when a `model_profile` narrows the field to "expensive" only, that
    /// narrowing (not the cost ranking) determines the outcome.
    fn router_with_cheap_and_expensive() -> ModelRouter {
        let llm_registry = Arc::new(LLMRegistry::new());
        let model_registry = Arc::new(ModelRegistry::new());

        llm_registry
            .register("cheap", MockLLMBackend::static_response("ok"))
            .expect("register cheap backend");
        model_registry.register(ModelEntry {
            name: "cheap-model".to_string(),
            backend: "cheap".to_string(),
            cost_per_1k_input: 0.01,
            cost_per_1k_output: 0.01,
            ..Default::default()
        });

        llm_registry
            .register("expensive", MockLLMBackend::static_response("ok"))
            .expect("register expensive backend");
        model_registry.register(ModelEntry {
            name: "expensive-model".to_string(),
            backend: "expensive".to_string(),
            cost_per_1k_input: 10.0,
            cost_per_1k_output: 10.0,
            ..Default::default()
        });

        let config = ModelRouterConfig {
            target: RoutingTarget::Cost,
            ..Default::default()
        };
        ModelRouter::with_model_registry(llm_registry, model_registry, config)
            .expect("router construction")
    }

    fn single_candidate_profile_registry() -> ProfileRegistry {
        let registry = ProfileRegistry::new();
        registry.register(ModelProfile {
            name: "single-tier".to_string(),
            description: "only the expensive model".to_string(),
            tags: vec![],
            min_context_window: None,
            candidates: vec![ProfileCandidate {
                model: "expensive-model".to_string(),
                priority: 1,
            }],
        });
        registry
    }

    async fn test_context() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam)
    }

    fn ask_request() -> LLMRequest {
        LLMRequest::new("hello").with_operation_type(AISOperationType::Ask)
    }

    #[tokio::test]
    async fn no_model_profile_leaves_full_candidate_set_unchanged() {
        let router = Arc::new(router_with_cheap_and_expensive());
        let profiles = Arc::new(single_candidate_profile_registry());
        let mut ctx = test_context().await;
        ctx.model_router = Some(Arc::clone(&router));
        ctx.profile_registry = Some(profiles);

        let request = ask_request();
        let resolved = resolve_model_profile(&ctx, request.clone());
        assert!(
            resolved.model.is_none(),
            "no model_profile set: request must be unchanged"
        );

        // Normal Cost-targeted routing picks the cheap backend.
        let decision = router.select(&resolved).expect("selection succeeds");
        assert_eq!(decision.backend, "cheap");
    }

    #[tokio::test]
    async fn node_model_profile_narrows_candidates_before_select() {
        let router = Arc::new(router_with_cheap_and_expensive());
        let profiles = Arc::new(single_candidate_profile_registry());
        let mut ctx = test_context().await;
        ctx.model_router = Some(Arc::clone(&router));
        ctx.profile_registry = Some(profiles);

        let request = ask_request().with_model_profile("single-tier");
        let resolved = resolve_model_profile(&ctx, request);
        assert_eq!(resolved.model.as_deref(), Some("expensive-model"));

        // Even though Cost target would otherwise pick "cheap", the profile
        // narrowed the candidate set to the single expensive model, so that
        // backend is selected regardless of cost ranking.
        let decision = router.select(&resolved).expect("selection succeeds");
        assert_eq!(decision.backend, "expensive");
    }

    #[tokio::test]
    async fn package_default_model_profile_applies_when_node_declares_none() {
        let router = Arc::new(router_with_cheap_and_expensive());
        let profiles = Arc::new(single_candidate_profile_registry());
        let mut ctx = test_context().await;
        ctx.model_router = Some(Arc::clone(&router));
        ctx.profile_registry = Some(profiles);
        ctx.default_model_profile = Some("single-tier".to_string());

        // Node/request declares no model_profile of its own.
        let request = ask_request();
        let resolved = resolve_model_profile(&ctx, request);
        assert_eq!(resolved.model.as_deref(), Some("expensive-model"));

        let decision = router.select(&resolved).expect("selection succeeds");
        assert_eq!(decision.backend, "expensive");
    }

    #[tokio::test]
    async fn explicit_node_model_profile_overrides_package_default() {
        let router = Arc::new(router_with_cheap_and_expensive());
        let profiles = ProfileRegistry::new();
        profiles.register(ModelProfile {
            name: "single-tier".to_string(),
            description: "only the expensive model".to_string(),
            tags: vec![],
            min_context_window: None,
            candidates: vec![ProfileCandidate {
                model: "expensive-model".to_string(),
                priority: 1,
            }],
        });
        profiles.register(ModelProfile {
            name: "cheap-tier".to_string(),
            description: "only the cheap model".to_string(),
            tags: vec![],
            min_context_window: None,
            candidates: vec![ProfileCandidate {
                model: "cheap-model".to_string(),
                priority: 1,
            }],
        });
        let mut ctx = test_context().await;
        ctx.model_router = Some(Arc::clone(&router));
        ctx.profile_registry = Some(Arc::new(profiles));
        // Package default says "cheap-tier"...
        ctx.default_model_profile = Some("cheap-tier".to_string());

        // ...but the node explicitly declares "single-tier", which must win.
        let request = ask_request().with_model_profile("single-tier");
        let resolved = resolve_model_profile(&ctx, request);
        assert_eq!(resolved.model.as_deref(), Some("expensive-model"));
    }

    #[tokio::test]
    async fn explicit_request_model_bypasses_profile_resolution_entirely() {
        let router = Arc::new(router_with_cheap_and_expensive());
        let profiles = Arc::new(single_candidate_profile_registry());
        let mut ctx = test_context().await;
        ctx.model_router = Some(Arc::clone(&router));
        ctx.profile_registry = Some(profiles);

        // Explicit model wins over any model_profile (explicit wins, rule 1).
        let request = ask_request()
            .with_model("cheap-model")
            .with_model_profile("single-tier");
        let resolved = resolve_model_profile(&ctx, request);
        assert_eq!(
            resolved.model.as_deref(),
            Some("cheap-model"),
            "explicit request.model must not be overridden by model_profile"
        );
    }

    #[test]
    fn apply_llm_request_routing_from_node_reads_model_profile_attribute() {
        use apxm_core::types::execution::Node;

        let mut node = Node::new(1, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::MODEL_PROFILE.to_string(),
            Value::from("reasoning-tier"),
        );

        let request =
            apply_llm_request_routing_from_node(LLMRequest::new("hi"), &node).expect("routing ok");
        assert_eq!(request.model_profile.as_deref(), Some("reasoning-tier"));
    }
}
