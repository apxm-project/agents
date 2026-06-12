//! Conversation-memory middleware.
//!
//! After every `ASK` turn, appends the answer to session-scoped STM so the
//! conversation transcript accrues automatically. Keyed by `memory_scope()`
//! (the session id), so turns are readable by the next turn's `qmem` recall.

use crate::executor::{ExecutionContext, Next, OperationMiddleware, Result};
use crate::memory::MemorySpace;
use apxm_core::types::{
    execution::Node,
    operations::AISOperationType,
    values::{Number, Value},
};
use async_trait::async_trait;

/// Session-memory key holding the running turn count.
const TURN_COUNT_KEY: &str = "conversation:turn_count";
/// Prefix for per-turn answer entries (`conversation:turn:<n>`).
const TURN_PREFIX: &str = "conversation:turn:";

/// Records each ASK answer into session memory so conversation history accrues
/// without the program threading a transcript.
#[derive(Debug, Clone, Default)]
pub struct ConversationMemoryMiddleware;

impl ConversationMemoryMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl OperationMiddleware for ConversationMemoryMiddleware {
    fn name(&self) -> &str {
        "conversation-memory"
    }

    /// Only conversational ASK turns accrue history.
    fn applies_to(&self, node: &Node) -> bool {
        node.op_type == AISOperationType::Ask
    }

    async fn around(
        &self,
        ctx: &ExecutionContext,
        node: &Node,
        inputs: Vec<Value>,
        next: Next<'_>,
    ) -> Result<Value> {
        let result = next.run(ctx, node, inputs).await;
        if let Ok(Value::String(answer)) = &result {
            let scope = ctx.memory_scope().to_string();
            let mem = ctx.memory();
            let next_turn = mem
                .read_scoped(MemorySpace::Stm, &scope, TURN_COUNT_KEY)
                .await
                .ok()
                .flatten()
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                + 1;
            // Best-effort: a memory write failure must not fail the turn.
            let _ = mem
                .write_scoped(
                    MemorySpace::Stm,
                    &scope,
                    TURN_COUNT_KEY.to_string(),
                    Value::Number(Number::Integer(next_turn)),
                )
                .await;
            let _ = mem
                .write_scoped(
                    MemorySpace::Stm,
                    &scope,
                    format!("{TURN_PREFIX}{next_turn}"),
                    Value::String(answer.clone()),
                )
                .await;
        }
        result
    }
}

