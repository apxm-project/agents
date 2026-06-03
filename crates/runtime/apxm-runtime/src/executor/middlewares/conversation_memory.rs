//! Conversation-memory middleware — context *management* as a composable layer.
//!
//! After every `ASK` turn, append the answer to session-scoped memory so the
//! conversation transcript accrues automatically as the agent runs. The agent
//! program never has to plumb history: this is the "context management is
//! middleware" half of the conversational-agent vision (the program owns
//! deliberate task memory via `qmem`/`umem`; the conversation window is
//! middleware). STM is keyed by `memory_scope()` (the session id), so turns
//! written here are readable by the next turn's `qmem` recall within a server
//! run — the same session-scoping the runtime already uses.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::executor::OperationDispatcher;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use std::sync::Arc;

    async fn test_context() -> ExecutionContext {
        let memory = Arc::new(MemorySystem::new(MemoryConfig::in_memory_ltm()).await.unwrap());
        ExecutionContext::new(
            memory,
            Arc::new(LLMRegistry::new()),
            Arc::new(CapabilitySystem::new()),
            Aam::new(),
        )
    }

    /// Short-circuits with a fixed answer so the test never needs a live LLM.
    struct AnswerStub(&'static str);

    #[async_trait]
    impl OperationMiddleware for AnswerStub {
        fn name(&self) -> &str {
            "answer-stub"
        }
        async fn around(
            &self,
            _ctx: &ExecutionContext,
            _node: &Node,
            _inputs: Vec<Value>,
            _next: Next<'_>,
        ) -> Result<Value> {
            Ok(Value::String(self.0.to_string()))
        }
    }

    #[tokio::test]
    async fn records_ask_answer_into_session_memory() {
        let ctx = test_context()
            .await
            .with_session_id("sess-X".to_string())
            .with_middlewares(vec![
                Arc::new(ConversationMemoryMiddleware::new()),
                Arc::new(AnswerStub("the answer")),
            ]);
        let node = Node::new(1, AISOperationType::Ask);

        let out = OperationDispatcher::dispatch(&ctx, &node, vec![Value::String("hi".into())])
            .await
            .unwrap();
        assert_eq!(out, Value::String("the answer".into()));

        let stored = ctx
            .memory()
            .read_scoped(MemorySpace::Stm, ctx.memory_scope(), "conversation:turn:1")
            .await
            .unwrap();
        assert_eq!(stored, Some(Value::String("the answer".into())));
    }

    #[tokio::test]
    async fn does_not_record_non_ask_nodes() {
        let mw = ConversationMemoryMiddleware::new();
        assert!(!mw.applies_to(&Node::new(1, AISOperationType::Nop)));
        assert!(mw.applies_to(&Node::new(1, AISOperationType::Ask)));
    }
}
