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
/// Node attribute the frontend stamps on the *top-level* conversational turn
/// `ASK` (`ConversationalAgent._build_turn_flow`). It scopes turn accounting
/// and lifecycle hooks to the user-facing turn, so sub-agent `ASK`s (which
/// inherit the parent `session_id`, and thus the same `memory_scope`) do not
/// inflate `conversation:turn_count`, pollute the recall window, or fire
/// `pre_turn`/`post_turn`/`post_ask` hooks. The program declares the turn
/// (constitution #2: program owns cognition); absent the marker, the ask is
/// not a conversational turn.
const TURN_MARKER_KEY: &str = "conversational_turn";

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

    /// Only the top-level conversational ASK turn accrues history and fires
    /// turn hooks. Sub-agent `ASK`s share the session `memory_scope` (they
    /// inherit `session_id`), so without the marker gate they would inflate the
    /// turn count and re-fire lifecycle hooks. The marker is stamped by the
    /// frontend on the turn flow's ask.
    fn applies_to(&self, node: &Node) -> bool {
        node.op_type == AISOperationType::Ask
            && node
                .attributes
                .get(TURN_MARKER_KEY)
                .and_then(|v| v.as_str())
                == Some("true")
    }

    async fn around(
        &self,
        ctx: &ExecutionContext,
        node: &Node,
        inputs: Vec<Value>,
        next: Next<'_>,
    ) -> Result<Value> {
        // pre_turn hooks fire before the turn's ask (gate-capable → fail-closed).
        crate::executor::hook_driver::run_pre_turn_hooks(ctx).await?;
        let result = next.run(ctx, node, inputs).await;
        if let Ok(Value::String(answer)) = &result {
            // post_ask + post_turn hooks fire with the reply (observe; FR-005).
            crate::executor::hook_driver::run_post_ask_hooks(ctx, answer).await;
            crate::executor::hook_driver::run_post_turn_hooks(ctx, answer).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(marked: bool) -> Node {
        let mut node = Node::new(1, AISOperationType::Ask);
        if marked {
            node.set_attribute(TURN_MARKER_KEY.to_string(), Value::String("true".into()));
        }
        node
    }

    #[test]
    fn applies_only_to_marked_conversational_turn() {
        let mw = ConversationMemoryMiddleware::new();
        // The top-level turn ask carries the marker the frontend stamps.
        assert!(mw.applies_to(&ask(true)));
        // A sub-agent ask shares the session scope but is unmarked: it must NOT
        // accrue history or fire turn hooks (CONV-2).
        assert!(!mw.applies_to(&ask(false)));
        // Non-ask ops never apply.
        let inv = Node::new(2, AISOperationType::InvTool);
        assert!(!mw.applies_to(&inv));
    }
}

