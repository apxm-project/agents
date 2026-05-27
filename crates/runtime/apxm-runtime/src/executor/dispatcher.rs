//! Operation dispatcher - Routes operations to appropriate handlers

use super::{
    Result,
    agent_scope::{
        LAYER2_BACKEND_DEFAULT, LAYER2_FINISH_REASON_STOP, LAYER2_TOOL_STATUS_ERROR,
        LAYER2_TOOL_STATUS_OK,
    },
    context::ExecutionContext,
    handlers::*,
    middleware::{BoxFuture, Next},
};
use apxm_core::apxm_op;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::types::{
    OperationMetric, execution::Node, operations::AISOperationType, values::Value,
};

/// Layer 2 context captured at OPERATION_START for an ASK/INV_TOOL node so
/// the matching OPERATION_END can emit the paired terminal event.
struct Layer2BeginContext {
    agent_code: String,
    /// Tool name for INV_TOOL; unused for ASK.
    tool_name: Option<String>,
    /// Wall-clock instant when the begin was emitted, for latency_ms on end.
    started_at: std::time::Instant,
}

/// Collect a string array attribute as a list of safe-to-surface argument
/// keys for Layer 2 `tool_call_begin`.
fn inv_tool_argument_keys(node: &Node) -> Vec<String> {
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
            // COMMUNICATE uses `recipient` (current) or `target` (legacy);
            // mirror handlers/communicate/mod.rs and accept both.
            if let Some(target) = attr(node, graph_attrs::RECIPIENT)
                .or_else(|| attr(node, graph_attrs::TARGET))
            {
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

        // Push a new child span for this node execution.
        let parent_span_id = ctx.event_emitter.as_ref().and_then(|e| e.current_span_id());
        let node_span_id = uuid::Uuid::new_v4().to_string();
        if let Some(emitter) = &ctx.event_emitter {
            emitter.set_current_span_id(Some(node_span_id.clone()));
        }

        // Emit OperationStart event, enriched with op-specific context so
        // observer UIs can render a labeled tree without re-parsing
        // attributes. Context populated here (Phase 14.8.A):
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

        // Layer 2 — emit a paired begin event when this op runs inside an
        // agent scope. ASK/THINK/REASON paired with `subagent_llm_call_*`;
        // INV_TOOL paired with `tool_call_*`. The matching end fires after
        // the handler returns (see below) so it sees both the duration and
        // the result shape. Captures the active scope's `agent_code` at
        // begin so the end remains coherent even if a nested SPAWN_AGENT
        // mutates the stack mid-handler.
        let layer2_begin = if let Some(emitter) = &ctx.event_emitter {
            ctx.agent_scope_stack.peek().and_then(|scope| match node.op_type {
                AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason => {
                    let model = node
                        .attributes
                        .get(graph_attrs::MODEL)
                        .and_then(|v| v.as_string())
                        .cloned()
                        .unwrap_or_default();
                    let backend = node
                        .attributes
                        .get(graph_attrs::BACKEND)
                        .and_then(|v| v.as_string())
                        .cloned()
                        .unwrap_or_else(|| LAYER2_BACKEND_DEFAULT.to_string());
                    let tool_manifest_count = node
                        .attributes
                        .get(graph_attrs::TOOLS)
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    emitter.emit_subagent_llm_call_begin(
                        &scope.agent_code,
                        &model,
                        &backend,
                        tool_manifest_count,
                    );
                    Some(Layer2BeginContext {
                        agent_code: scope.agent_code.clone(),
                        tool_name: None,
                        started_at: op_start,
                    })
                }
                AISOperationType::InvTool => {
                    let tool_name = node
                        .attributes
                        .get(graph_attrs::CAPABILITY)
                        .and_then(|v| v.as_string())
                        .cloned()
                        .unwrap_or_default();
                    let argument_keys = inv_tool_argument_keys(node);
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

        let result = match node.op_type {
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
            AISOperationType::InvTool => inv_tool::execute(ctx, node, inputs).await,

            // Synchronization operations
            AISOperationType::WaitAll => wait_all::execute(ctx, node, inputs).await,
            AISOperationType::Merge => merge::execute(ctx, node, inputs).await,
            AISOperationType::Fence => fence::execute(ctx, node, inputs).await,
            AISOperationType::Checkpoint => checkpoint::execute(ctx, node, inputs).await,

            // Control flow operations
            AISOperationType::BranchOnValue => branch::execute(ctx, node, inputs).await,
            AISOperationType::Jump => jump::execute(ctx, node, inputs).await,
            AISOperationType::LoopStart => loop_start::execute(ctx, node, inputs).await,
            AISOperationType::LoopEnd => loop_end::execute(ctx, node, inputs).await,
            AISOperationType::Return => return_op::execute(ctx, node, inputs).await,
            AISOperationType::Switch => switch::execute(ctx, node, inputs).await,
            AISOperationType::FlowCall => flow_call::execute(ctx, node, inputs).await,
            AISOperationType::WorkflowSpawn => workflow_spawn::execute(ctx, node, inputs).await,
            AISOperationType::CallSkill => call_skill::execute(ctx, node, inputs).await,

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
            AISOperationType::Guard => guard::execute(ctx, node, inputs).await,
            AISOperationType::Claim => claim::execute(ctx, node, inputs).await,
            AISOperationType::Pause => pause::execute(ctx, node, inputs).await,
            AISOperationType::Resume => resume::execute(ctx, node, inputs).await,

            // Multi-agent operations
            AISOperationType::Delegate => delegate::execute(ctx, node, inputs).await,
            AISOperationType::Negotiate => negotiate::execute(ctx, node, inputs).await,
            AISOperationType::Nop => nop::execute(ctx, node, inputs).await,
            AISOperationType::Identity => identity::execute(ctx, node, inputs).await,
            AISOperationType::SpawnAgent => spawn_agent::execute(ctx, node, inputs).await,
            AISOperationType::SpawnTeam => spawn_team::execute(ctx, node, inputs).await,
            AISOperationType::RegisterCapability => {
                register_capability::execute(ctx, node, inputs).await
            }
            AISOperationType::Autonomous => autonomous::execute(ctx, node, inputs).await,

            // Literal operations
            AISOperationType::ConstStr => const_str::execute(ctx, node, inputs).await,

            // No-op: Agent is metadata, Yield is handled within sub-DAG execution
            AISOperationType::Agent | AISOperationType::Yield => Ok(Value::Null),
        };

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

            // Layer 2 terminal for ASK/INV_TOOL when the begin captured an
            // active scope at the same node.
            if let Some(begin) = layer2_begin.as_ref() {
                let latency_ms = begin.started_at.elapsed().as_millis() as u64;
                match node.op_type {
                    AISOperationType::Ask
                    | AISOperationType::Think
                    | AISOperationType::Reason => {
                        let (input_tokens, output_tokens) = tokens
                            .as_ref()
                            .map(|t| (t.input_tokens, t.output_tokens))
                            .unwrap_or((0, 0));
                        let content_len = match &result {
                            Ok(Value::String(s)) => s.len(),
                            _ => 0,
                        };
                        emitter.emit_subagent_llm_call_end(
                            &begin.agent_code,
                            LAYER2_FINISH_REASON_STOP,
                            input_tokens,
                            output_tokens,
                            content_len,
                        );
                    }
                    AISOperationType::InvTool => {
                        let tool_name = begin.tool_name.as_deref().unwrap_or("");
                        let (status, result_keys) = match &result {
                            Ok(value) => (LAYER2_TOOL_STATUS_OK, result_keys_for_layer2(value)),
                            Err(_) => (LAYER2_TOOL_STATUS_ERROR, Vec::new()),
                        };
                        emitter.emit_tool_call_end(
                            &begin.agent_code,
                            tool_name,
                            &result_keys,
                            status,
                            latency_ms,
                        );
                    }
                    _ => {}
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

            emitter.emit_operation_end(
                node.id,
                node.op_type,
                op_duration,
                success,
                tokens,
                timing,
            );
        }

        // Restore parent span after node execution completes.
        if let Some(emitter) = &ctx.event_emitter {
            emitter.set_current_span_id(parent_span_id);
        }

        match &result {
            Ok(_value) => {
                apxm_op!(trace,
                    op_type = ?node.op_type,
                    node_id = node.id,
                    "Handler completed"
                );
                // Record in episodic memory
                ctx.memory
                    .record_episode(
                        format!("operation_completed:{:?}", node.op_type),
                        _value.clone(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        aam::Aam,
        capability::CapabilitySystem,
        executor::{ExecutionContext, OperationMiddleware},
        memory::{MemoryConfig, MemorySystem},
    };
    use apxm_core::types::operations::AISOperationType;
    use async_trait::async_trait;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn all_operations_covered() {
        // Verify the total count matches what we expect.
        // If this fails, a variant was added or removed — update the dispatcher
        // match and CONTRACTS.md accordingly.
        assert_eq!(
            AISOperationType::all_operations().len(),
            44,
            "AISOperationType variant count changed — update dispatcher and CONTRACTS.md"
        );
    }

    async fn test_context() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
    }

    struct AppliesOnlyToNop {
        hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl OperationMiddleware for AppliesOnlyToNop {
        fn name(&self) -> &str {
            "nop-only"
        }

        fn applies_to(&self, node: &Node) -> bool {
            matches!(node.op_type, AISOperationType::Nop)
        }

        async fn around(
            &self,
            ctx: &ExecutionContext,
            node: &Node,
            inputs: Vec<Value>,
            next: Next<'_>,
        ) -> Result<Value> {
            self.hits.fetch_add(1, Ordering::Relaxed);
            next.run(ctx, node, inputs).await
        }
    }

    #[tokio::test]
    async fn dispatch_skips_middlewares_that_do_not_apply() {
        let hits = Arc::new(AtomicUsize::new(0));
        let ctx = test_context()
            .await
            .with_middleware(Arc::new(AppliesOnlyToNop {
                hits: Arc::clone(&hits),
            }));
        let node = Node::new(2, AISOperationType::Print);

        let value = OperationDispatcher::dispatch(&ctx, &node, vec![])
            .await
            .unwrap();

        assert_eq!(hits.load(Ordering::Relaxed), 0);
        assert_eq!(value, Value::Null);
    }
}
