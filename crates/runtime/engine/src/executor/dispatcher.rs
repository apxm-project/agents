//! Operation dispatcher - Routes operations to appropriate handlers

use super::{
    Result,
    context::ExecutionContext,
    handlers::*,
    middleware::{BoxFuture, Next},
};
use apxm_core::apxm_op;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::events::payload::ToolCallStatus;
use apxm_core::types::{
    OperationMetric, execution::Node, operations::AISOperationType, values::Value,
};

/// Layer 2 context captured at OPERATION_START for an ASK/INV_CAP node so
/// the matching OPERATION_END can emit the paired terminal event.
struct Layer2BeginContext {
    agent_code: String,
    /// Tool name for INV_CAP; unused for ASK.
    tool_name: Option<String>,
    /// Wall-clock instant when the begin was emitted, for latency_ms on end.
    started_at: std::time::Instant,
}

/// Collect a string array attribute as a list of safe-to-surface argument
/// keys for Layer 2 `tool_call_begin`.
fn inv_cap_argument_keys(node: &Node) -> Vec<String> {
    let Some(params_json) = node
        .attributes
        .get(graph_attrs::PARAMS_JSON)
        .and_then(|v| v.as_string())
    else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(params_json) else {
        return Vec::new();
    };
    parsed
        .as_object()
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

/// Result-shape keys for Layer 2 `tool_call_end`. Only the top-level keys
/// of an object result surface; everything else is summarized.
fn result_keys_for_layer2(value: &Value) -> Vec<String> {
    match value {
        Value::Object(map) => map.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// Build the optional context value attached to OperationStart events for
/// ops that carry agent/tool identity. Returns None for ops where the
/// node id + op_type already convey everything an observer cares about.
fn operation_start_context(node: &Node) -> Option<serde_json::Value> {
    fn attr(node: &Node, key: &str) -> Option<String> {
        node.attributes
            .get(key)
            .and_then(|value| value.as_string())
            .cloned()
    }

    match node.op_type {
        AISOperationType::SpawnAgent => {
            let mut ctx = serde_json::Map::new();
            if let Some(agent_code) = attr(node, graph_attrs::AGENT_NAME) {
                ctx.insert("agent_code".to_string(), agent_code.into());
            }
            if let Some(profile) = attr(node, graph_attrs::PROFILE) {
                ctx.insert("profile".to_string(), profile.into());
            }
            if ctx.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(ctx))
            }
        }
        AISOperationType::Communicate => {
            let mut ctx = serde_json::Map::new();
            if let Some(target) = attr(node, graph_attrs::RECIPIENT) {
                ctx.insert("target_agent".to_string(), target.into());
            }
            if let Some(protocol) = attr(node, graph_attrs::PROTOCOL) {
                ctx.insert("protocol".to_string(), protocol.into());
            }
            if ctx.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(ctx))
            }
        }
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason => {
            let mut ctx = serde_json::Map::new();
            if let Some(model) = attr(node, graph_attrs::MODEL) {
                ctx.insert("model".to_string(), model.into());
            }
            if let Some(backend) = attr(node, graph_attrs::BACKEND) {
                ctx.insert("backend".to_string(), backend.into());
            }
            // ASK ops carry their tool surface as a JSON list under
            // `tools`; expose the names so the observer UI can label the
            // request without re-parsing the manifest.
            if let Some(tools_json) = attr(node, graph_attrs::TOOLS)
                && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&tools_json)
                && let Some(arr) = parsed.as_array()
            {
                let names: Vec<serde_json::Value> = arr
                    .iter()
                    .filter_map(|item| {
                        item.get("name")
                            .cloned()
                            .or_else(|| item.as_str().map(serde_json::Value::from))
                    })
                    .collect();
                if !names.is_empty() {
                    ctx.insert("tool_names".to_string(), names.into());
                }
            }
            if ctx.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(ctx))
            }
        }
        _ => None,
    }
}

/// Operation dispatcher routes operations to their handlers
pub struct OperationDispatcher;

impl OperationDispatcher {
    /// Dispatch an operation to its handler
    ///
    /// # Arguments
    /// * `ctx` - Execution context
    /// * `node` - Operation node from DAG
    /// * `inputs` - Input values from dependencies
    ///
    /// # Returns
    /// Result value from operation execution
    pub fn dispatch<'a>(
        ctx: &'a ExecutionContext,
        node: &'a Node,
        inputs: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value>> {
        let chain: Vec<_> = ctx
            .middlewares
            .iter()
            .filter(|middleware| middleware.applies_to(node))
            .cloned()
            .collect();

        if chain.is_empty() {
            return Self::dispatch_inner_boxed(ctx, node, inputs);
        }

        Box::pin(async move {
            Next {
                chain: &chain,
                idx: 0,
                terminal: Self::dispatch_inner_boxed,
            }
            .run(ctx, node, inputs)
            .await
        })
    }

    pub(super) fn dispatch_inner_boxed<'a>(
        ctx: &'a ExecutionContext,
        node: &'a Node,
        inputs: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value>> {
        Box::pin(Self::dispatch_inner(ctx, node, inputs))
    }

    async fn dispatch_inner(
        ctx: &ExecutionContext,
        node: &Node,
        inputs: Vec<Value>,
    ) -> Result<Value> {
        apxm_op!(trace,
            op_type = ?node.op_type,
            node_id = node.id,
            inputs = inputs.len(),
            "Handler dispatch"
        );

        // Check cancellation before starting any work.
        if ctx.cancellation_token.is_cancelled() {
            return Err(RuntimeError::SchedulerCancelled);
        }

        // Permanent op-usage counter: every dispatched node — including
        // pseudo-ops like AGENT/YIELD — increments the in-process counter that
        // `apxm ops usage` reports on. See `executor::op_usage`.
        super::op_usage::record(node.op_type);

        // Push a new child span for this node execution.
        let parent_span_id = ctx.event_emitter.as_ref().and_then(|e| e.current_span_id());
        let node_span_id = uuid::Uuid::new_v4().to_string();
        if let Some(emitter) = &ctx.event_emitter {
            emitter.set_current_span_id(Some(node_span_id.clone()));
        }

        // Emit OperationStart event, enriched with op-specific context so
        // observer UIs can render a labeled tree without re-parsing
        // attributes. Context populated here:
        //   - SPAWN_AGENT → { agent_code, profile }
        //   - COMMUNICATE → { target_agent, protocol }
        //   - ASK/THINK/REASON → { model, backend, tool_names, tool_choice }
        // Other ops fall through to the plain emit_operation_start.
        if let Some(emitter) = &ctx.event_emitter {
            if let Some(context) = operation_start_context(node) {
                emitter.emit_operation_start_with_context(node.id, node.op_type, context);
            } else {
                emitter.emit_operation_start(node.id, node.op_type);
            }
        }
        let op_start = std::time::Instant::now();

        // Direct INV_CAP remains a generic runtime call. Model-derived tool
        // calls emit their correlated lifecycle from the LLM tool dispatcher.
        let layer2_begin = if let Some(emitter) = &ctx.event_emitter {
            ctx.agent_scope_stack
                .peek()
                .and_then(|scope| match node.op_type {
                    AISOperationType::InvCap => {
                        let tool_name = node
                            .attributes
                            .get(graph_attrs::CAPABILITY)
                            .and_then(|v| v.as_string())
                            .cloned()
                            .unwrap_or_default();
                        let argument_keys = inv_cap_argument_keys(node);
                        emitter.emit_tool_call_begin(&scope.agent_code, &tool_name, &argument_keys);
                        Some(Layer2BeginContext {
                            agent_code: scope.agent_code.clone(),
                            tool_name: Some(tool_name),
                            started_at: op_start,
                        })
                    }
                    _ => None,
                })
        } else {
            None
        };

        // Push this operation onto the AAM call stack so that
        // `current_exception_handler()` can resolve TryCatch scopes.
        ctx.aam.enter_operation(node.id);

        // Run the op-specific handler, applying the generic declarative
        // retry/backoff + continue-on-error primitive (additive node attributes;
        // a no-op for nodes that declare none). See `run_handler_with_retry`.
        let result = Self::run_handler_with_retry(ctx, node, inputs).await;

        // Pop the call stack frame (must happen regardless of success/failure).
        ctx.aam.exit_operation();

        let op_duration = op_start.elapsed();
        let success = result.is_ok();
        ctx.graph_metrics.record_operation(OperationMetric {
            node_id: node.id,
            op_type: node.op_type,
            duration_ms: op_duration.as_millis() as u64,
            success,
        });
        let node_metrics = ctx.graph_metrics.get_node(node.id);

        // Emit OperationEnd event. Layer 2 terminals fire FIRST so the
        // resulting wire stream reads
        //   operation_start[X] → tool_call_begin/subagent_llm_call_begin
        //   → (handler) → tool_call_end/subagent_llm_call_end →
        //   operation_end[X]
        // — matching the CLAUDE.md §10 pairing rule.
        if let Some(emitter) = &ctx.event_emitter {
            if let Some(metrics) = &node_metrics {
                emitter.emit_node_metrics_with_name(
                    node.id,
                    node.metadata.name.as_deref(),
                    metrics,
                );
            }
            let tokens = ctx.token_accountant.get_node(node.id);
            let timing = ctx.timing_tracker.get_node(node.id);

            // Direct INV_CAP terminal for the generic runtime call above.
            if let Some(begin) = layer2_begin.as_ref() {
                let latency_ms = begin.started_at.elapsed().as_millis() as u64;
                if node.op_type == AISOperationType::InvCap {
                    let tool_name = begin.tool_name.as_deref().unwrap_or("");
                    let (status, result_keys) = match &result {
                        Ok(value) => (ToolCallStatus::Ok, result_keys_for_layer2(value)),
                        Err(_) => (ToolCallStatus::Error, Vec::new()),
                    };
                    emitter.emit_tool_call_end(
                        &begin.agent_code,
                        tool_name,
                        &result_keys,
                        status,
                        latency_ms,
                    );
                }
            }

            // Layer 2 — coordinator final answer. When an ASK fires at
            // top-level (no agent scope is active) and yields text, treat
            // that as the coordinator's `agent_message`. Sub-agent ASKs
            // are already covered by `subagent_llm_call_end` above; this
            // branch only fires for the outermost coordinator turn.
            if matches!(node.op_type, AISOperationType::Ask) && ctx.agent_scope_stack.is_empty() {
                if let Ok(Value::String(text)) = &result {
                    let (input_tokens, output_tokens) = tokens
                        .as_ref()
                        .map(|t| (Some(t.input_tokens), Some(t.output_tokens)))
                        .unwrap_or((None, None));
                    emitter.emit_agent_message(text, None, None, input_tokens, output_tokens);
                }
            }

            emitter.emit_operation_end(node.id, node.op_type, op_duration, success, tokens, timing);
        }

        // Restore parent span after node execution completes.
        if let Some(emitter) = &ctx.event_emitter {
            emitter.set_current_span_id(parent_span_id);
        }

        match &result {
            Ok(value) => {
                apxm_op!(trace,
                    op_type = ?node.op_type,
                    node_id = node.id,
                    "Handler completed"
                );
                // Record in episodic memory
                ctx.memory
                    .record_episode(
                        format!("operation_completed:{:?}", node.op_type),
                        value.clone(),
                        ctx.execution_id.clone(),
                        Some(node.id),
                        None, // session_dir not available in dispatcher context
                    )
                    .await
                    .ok(); // Ignore episodic recording errors
            }
            Err(e) => {
                apxm_op!(error,
                    op_type = ?node.op_type,
                    node_id = node.id,
                    error = %e,
                    "Handler failed"
                );
                // Record error in episodic memory
                ctx.memory
                    .record_episode(
                        format!("operation_failed:{:?}", node.op_type),
                        Value::String(e.to_string()),
                        ctx.execution_id.clone(),
                        Some(node.id),
                        None, // session_dir not available in dispatcher context
                    )
                    .await
                    .ok();
            }
        }

        result
    }

    /// Apply the generic declarative retry/backoff + continue-on-error primitive
    /// around the op-specific handler. The contract is additive: a node that
    /// declares none of `retry_max` / `retry_backoff_ms` / `continue_on_error`
    /// behaves exactly as before (one attempt, error halts).
    ///
    /// - `retry_max` (≥1)         : re-run the handler up to N more times after a
    ///   failure, sleeping `retry_backoff_ms * 2^(attempt-1)` between tries.
    /// - `continue_on_error=true` : a node that still fails does NOT propagate the
    ///   error; it emits a structured error value (`{ "__apxm_error": {...} }`)
    ///   downstream so an error edge can consume it. The run continues.
    ///
    /// Cancellation always wins: a `SchedulerCancelled` error is never retried or
    /// swallowed (it must propagate to abort the run promptly).
    async fn run_handler_with_retry(
        ctx: &ExecutionContext,
        node: &Node,
        inputs: Vec<Value>,
    ) -> Result<Value> {
        let retry_max = node
            .attributes
            .get(graph_attrs::RETRY_MAX)
            .and_then(Self::attr_as_u64)
            .unwrap_or(0);
        let continue_on_error = node
            .attributes
            .get(graph_attrs::CONTINUE_ON_ERROR)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Fast path: no retry, no continue — the common case, zero overhead.
        if retry_max == 0 && !continue_on_error {
            return Self::run_handler(ctx, node, inputs).await;
        }

        let backoff_base_ms = node
            .attributes
            .get(graph_attrs::RETRY_BACKOFF_MS)
            .and_then(Self::attr_as_u64)
            .unwrap_or(graph_attrs::DEFAULT_RETRY_BACKOFF_MS);

        let mut last_err: Option<RuntimeError> = None;
        for attempt in 0..=retry_max {
            if ctx.cancellation_token.is_cancelled() {
                return Err(RuntimeError::SchedulerCancelled);
            }
            match Self::run_handler(ctx, node, inputs.clone()).await {
                Ok(value) => return Ok(value),
                // Never retry or swallow a cancellation — propagate immediately.
                Err(RuntimeError::SchedulerCancelled) => {
                    return Err(RuntimeError::SchedulerCancelled);
                }
                Err(e) => {
                    if attempt < retry_max {
                        let backoff_ms = backoff_base_ms.saturating_mul(1u64 << attempt);
                        apxm_op!(warn,
                            op_type = ?node.op_type,
                            node_id = node.id,
                            attempt = attempt + 1,
                            retry_max,
                            backoff_ms,
                            error = %e,
                            "Handler failed; retrying after backoff"
                        );
                        if backoff_ms > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        }
                    }
                    last_err = Some(e);
                }
            }
        }

        let err = last_err.unwrap_or_else(|| RuntimeError::Operation {
            op_type: node.op_type,
            message: "handler failed with no error captured".to_string(),
        });

        if continue_on_error {
            apxm_op!(warn,
                op_type = ?node.op_type,
                node_id = node.id,
                error = %err,
                "Handler failed after retries; continuing (continue_on_error) with error output"
            );
            Ok(Self::error_output_value(node, &err))
        } else {
            Err(err)
        }
    }

    /// Build the structured error-output value a continue-on-error node emits
    /// downstream. The `__apxm_error` key lets a downstream node detect the
    /// failure and branch on it (the "error edge"), while keeping the value a
    /// plain object the rest of the graph can carry.
    fn error_output_value(node: &Node, err: &RuntimeError) -> Value {
        use apxm_core::types::values::Number;
        let mut error_obj: std::collections::HashMap<String, Value> =
            std::collections::HashMap::new();
        error_obj.insert("message".to_string(), Value::String(err.to_string()));
        error_obj.insert(
            "op_type".to_string(),
            Value::String(format!("{:?}", node.op_type)),
        );
        error_obj.insert(
            "node_id".to_string(),
            Value::Number(Number::Integer(i64::try_from(node.id).unwrap_or(i64::MAX))),
        );
        let mut map: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
        map.insert(
            graph_attrs::ERROR_OUTPUT_KEY.to_string(),
            Value::Object(error_obj),
        );
        Value::Object(map)
    }

    /// Parse a node attribute as a u64, accepting either an integer or a numeric
    /// string (AIR attributes are often serialized as strings).
    fn attr_as_u64(value: &Value) -> Option<u64> {
        if let Some(i) = value.as_i64() {
            return u64::try_from(i).ok();
        }
        value.as_string().and_then(|s| s.trim().parse::<u64>().ok())
    }

    /// Route a node to its op-specific handler. Extracted from `dispatch_inner`
    /// so the generic retry primitive can re-invoke it per attempt.
    async fn run_handler(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
        match node.op_type {
            // Memory operations
            AISOperationType::QMem => qmem::execute(ctx, node, inputs).await,
            AISOperationType::UMem => umem::execute(ctx, node, inputs).await,

            // LLM operations (Ask/Think/Reason → unified llm handler)
            AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason => {
                llm::execute(ctx, node, inputs).await
            }

            // Planning & analysis operations
            AISOperationType::Plan => plan::execute(ctx, node, inputs).await,
            AISOperationType::Reflect => reflect::execute(ctx, node, inputs).await,
            AISOperationType::Verify => verify::execute(ctx, node, inputs).await,

            // Invocation operations
            AISOperationType::InvCap => inv_cap::execute(ctx, node, inputs).await,

            // Synchronization operations
            AISOperationType::WaitAll => wait_all::execute(ctx, node, inputs).await,
            AISOperationType::AwaitInput => await_input::execute(ctx, node, inputs).await,
            AISOperationType::Merge => merge::execute(ctx, node, inputs).await,
            AISOperationType::Fence => fence::execute(ctx, node, inputs).await,
            AISOperationType::Checkpoint => checkpoint::execute(ctx, node, inputs).await,

            // Control flow operations
            AISOperationType::BranchOnValue => branch::execute(ctx, node, inputs).await,
            AISOperationType::Jump => jump::execute(ctx, node, inputs).await,
            AISOperationType::Return => return_op::execute(ctx, node, inputs).await,
            AISOperationType::Switch => switch::execute(ctx, node, inputs).await,
            AISOperationType::FlowCall => flow_call::execute(ctx, node, inputs).await,
            AISOperationType::WorkflowSpawn => workflow_spawn::execute(ctx, node, inputs).await,

            // Error handling operations
            AISOperationType::TryCatch => try_catch::execute(ctx, node, inputs).await,
            AISOperationType::Err => err::execute(ctx, node, inputs).await,
            AISOperationType::Exc => exc::execute(ctx, node, inputs).await,

            // Output operations
            AISOperationType::Print => print::execute(ctx, node, inputs).await,

            // Communication operations
            AISOperationType::Communicate => communicate::execute(ctx, node, inputs).await,
            AISOperationType::Handoff => handoff::execute(ctx, node, inputs).await,

            // Coordination operations
            AISOperationType::UpdateGoal => update_goal::execute(ctx, node, inputs).await,
            AISOperationType::Pause => pause::execute(ctx, node, inputs).await,
            AISOperationType::Resume => resume::execute(ctx, node, inputs).await,

            // Multi-agent operations
            AISOperationType::Delegate => delegate::execute(ctx, node, inputs).await,
            AISOperationType::Nop => nop::execute(ctx, node, inputs).await,
            AISOperationType::Identity => identity::execute(ctx, node, inputs).await,
            AISOperationType::SpawnAgent => spawn_agent::execute(ctx, node, inputs).await,
            AISOperationType::RegisterCapability => {
                register_capability::execute(ctx, node, inputs).await
            }
            AISOperationType::RegisterHook => register_hook::execute(ctx, node, inputs).await,
            AISOperationType::Autonomous => autonomous::execute(ctx, node, inputs).await,

            // Literal operations
            AISOperationType::ConstStr => const_str::execute(ctx, node, inputs).await,

            // No-op: Agent is metadata, Yield is handled within sub-DAG execution
            AISOperationType::Agent | AISOperationType::Yield => Ok(Value::Null),
        }
    }
}

#[cfg(test)]
mod retry_primitive_tests {
    use super::*;
    use apxm_core::types::values::Number;

    fn node(op: AISOperationType) -> Node {
        let mut n = Node::new(7, op);
        n.metadata.name = Some("retry-test".to_string());
        n
    }

    #[test]
    fn attr_as_u64_accepts_int_and_numeric_string() {
        assert_eq!(
            OperationDispatcher::attr_as_u64(&Value::Number(Number::Integer(3))),
            Some(3)
        );
        assert_eq!(
            OperationDispatcher::attr_as_u64(&Value::String(" 5 ".to_string())),
            Some(5)
        );
        assert_eq!(
            OperationDispatcher::attr_as_u64(&Value::String("not-a-number".to_string())),
            None
        );
        // A negative integer cannot be a retry count / backoff.
        assert_eq!(
            OperationDispatcher::attr_as_u64(&Value::Number(Number::Integer(-1))),
            None
        );
    }

    #[test]
    fn error_output_value_carries_structured_error_under_well_known_key() {
        let node = node(AISOperationType::InvCap);
        let err = RuntimeError::Operation {
            op_type: AISOperationType::InvCap,
            message: "boom".to_string(),
        };
        let value = OperationDispatcher::error_output_value(&node, &err);
        let Value::Object(map) = value else {
            panic!("error output must be an object");
        };
        let inner = map
            .get(graph_attrs::ERROR_OUTPUT_KEY)
            .expect("error output keyed under the well-known error key");
        let Value::Object(error_obj) = inner else {
            panic!("error payload must be an object");
        };
        assert!(
            error_obj
                .get("message")
                .and_then(|v| v.as_string())
                .is_some_and(|m| m.contains("boom")),
            "error output carries the failing handler's message"
        );
        assert_eq!(
            error_obj.get("node_id").and_then(|v| v.as_i64()),
            Some(7),
            "error output records the failing node id for the error edge"
        );
        assert!(
            error_obj.contains_key("op_type"),
            "error output records the failing op type"
        );
    }
}
