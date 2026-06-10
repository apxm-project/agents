use crate::executor::{ExecutionContext, Next, OperationMiddleware, Result};
use apxm_core::{
    error::RuntimeError,
    types::{execution::Node, values::Value},
};
use async_trait::async_trait;
use std::sync::atomic::Ordering;

/// Pre-flight token-budget guard.
///
/// Before each node runs, if the execution context carries a token budget and
/// the already-consumed token count has reached it, the node is rejected
/// instead of dispatching another (potentially costly) LLM call. This gives a
/// uniform, op-agnostic cost ceiling for a whole multi-agent turn.
///
/// It is strictly READ-ONLY: it never charges tokens (the LLM handler does that
/// in `handlers/llm/pipeline.rs`), so there is no double-counting. Token
/// accounting flows through `ctx.consumed_tokens` (shared `Arc<AtomicU64>`),
/// which children inherit, so the budget spans spawned sub-agents.
#[derive(Debug, Clone, Default)]
pub struct TokenBudgetMiddleware;

impl TokenBudgetMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl OperationMiddleware for TokenBudgetMiddleware {
    fn name(&self) -> &str {
        "token-budget"
    }

    async fn around(
        &self,
        ctx: &ExecutionContext,
        node: &Node,
        inputs: Vec<Value>,
        next: Next<'_>,
    ) -> Result<Value> {
        if let Some(budget) = ctx.token_budget {
            let used = ctx.consumed_tokens.load(Ordering::Relaxed);
            if used >= budget {
                return Err(RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!(
                        "token budget exhausted: {used}/{budget} tokens consumed \
                         before node {}",
                        node.id
                    ),
                });
            }
        }
        next.run(ctx, node, inputs).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        aam::Aam,
        capability::CapabilitySystem,
        executor::OperationDispatcher,
        memory::{MemoryConfig, MemorySystem},
    };
    use apxm_backends::LLMRegistry;
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    async fn test_context() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
    }

    #[tokio::test]
    async fn rejects_node_when_budget_exhausted() {
        let ctx = test_context()
            .await
            .with_token_budget(Some(100))
            .with_middlewares(vec![Arc::new(TokenBudgetMiddleware::new())]);
        ctx.consumed_tokens.store(100, Ordering::Relaxed);
        let node = Node::new(7, AISOperationType::Nop);

        let error = OperationDispatcher::dispatch(&ctx, &node, vec![])
            .await
            .unwrap_err();
        assert!(
            matches!(error, RuntimeError::Operation { op_type, .. } if op_type == AISOperationType::Nop)
        );
    }

    #[tokio::test]
    async fn allows_node_within_budget() {
        let ctx = test_context()
            .await
            .with_token_budget(Some(100))
            .with_middlewares(vec![Arc::new(TokenBudgetMiddleware::new())]);
        ctx.consumed_tokens.store(10, Ordering::Relaxed);
        // Nop dispatches to a passthrough; within budget it must not error.
        let node = Node::new(8, AISOperationType::Nop);
        assert!(
            OperationDispatcher::dispatch(&ctx, &node, vec![])
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn no_budget_is_unlimited() {
        let ctx = test_context()
            .await
            .with_middlewares(vec![Arc::new(TokenBudgetMiddleware::new())]);
        ctx.consumed_tokens.store(1_000_000, Ordering::Relaxed);
        let node = Node::new(9, AISOperationType::Nop);
        assert!(
            OperationDispatcher::dispatch(&ctx, &node, vec![])
                .await
                .is_ok()
        );
    }
}
