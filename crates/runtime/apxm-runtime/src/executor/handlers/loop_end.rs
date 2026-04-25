//! LOOP_END operation - Loop termination check

use super::{ExecutionContext, Node, Result, Value, get_input};
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Check if we should continue looping
    let counter = get_input(node, &inputs, 0)?;

    let should_continue = counter.as_u64().is_some_and(|count| count > 0);

    // Record loop check in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::LOOP_END_PREFIX,
            ctx.execution_id,
            node.id
        ),
        Value::Bool(should_continue),
        label,
    );

    Ok(Value::Bool(should_continue))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::values::Number;
    use std::sync::Arc;

    async fn test_ctx() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        ExecutionContext::new(
            memory,
            Arc::new(apxm_backends::LLMRegistry::new()),
            Arc::new(CapabilitySystem::new()),
            crate::aam::Aam::new(),
        )
    }

    fn loop_end_node() -> Node {
        Node::new(1, AISOperationType::LoopEnd)
    }

    #[tokio::test]
    async fn test_loop_end_continue_when_counter_positive() {
        let ctx = test_ctx().await;
        let node = loop_end_node();
        let result = execute(&ctx, &node, vec![Value::Number(Number::Integer(5))])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[tokio::test]
    async fn test_loop_end_terminate_when_counter_zero() {
        let ctx = test_ctx().await;
        let node = loop_end_node();
        let result = execute(&ctx, &node, vec![Value::Number(Number::Integer(0))])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_loop_end_terminate_when_counter_negative() {
        let ctx = test_ctx().await;
        let node = loop_end_node();
        // Negative integer: as_u64() returns None, so should_continue = false
        let result = execute(&ctx, &node, vec![Value::Number(Number::Integer(-1))])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_loop_end_non_numeric_terminates() {
        let ctx = test_ctx().await;
        let node = loop_end_node();
        let result = execute(&ctx, &node, vec![Value::String("not a number".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_loop_end_missing_input_errors() {
        let ctx = test_ctx().await;
        let node = loop_end_node();
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }
}
