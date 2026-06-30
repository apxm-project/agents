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
