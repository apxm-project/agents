//! Operation dispatcher - Routes operations to appropriate handlers

use super::{
    Result,
    context::ExecutionContext,
    handlers::*,
    middleware::{BoxFuture, Next},
};
use apxm_core::apxm_op;
use apxm_core::error::RuntimeError;
use apxm_core::types::{
    OperationMetric, execution::Node, operations::AISOperationType, values::Value,
};

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

        // Emit OperationStart event
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_operation_start(node.id, node.op_type);
        }
        let op_start = std::time::Instant::now();

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

        // Emit OperationEnd event
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
            emitter.emit_operation_end(node.id, node.op_type, op_duration, success, tokens, timing);
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
