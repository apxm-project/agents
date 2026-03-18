//! BRANCH operation - Conditional branching

use super::{ExecutionContext, Node, Result, Value, get_input};
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // First input is the condition
    let condition = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        Value::Bool(false)
    };

    // Evaluate condition as boolean
    let is_true = condition.as_boolean().unwrap_or(false);

    // Record branch decision in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!("{}{}:{}", belief_keys::BRANCH_PREFIX, ctx.execution_id, node.id),
        Value::Bool(is_true),
        label,
    );

    // Return boolean result (actual branching handled by scheduler)
    Ok(Value::Bool(is_true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
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

    fn branch_node() -> Node {
        Node::new(1, AISOperationType::BranchOnValue)
    }

    #[tokio::test]
    async fn test_branch_true() {
        let ctx = test_ctx().await;
        let node = branch_node();
        let result = execute(&ctx, &node, vec![Value::Bool(true)])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[tokio::test]
    async fn test_branch_false() {
        let ctx = test_ctx().await;
        let node = branch_node();
        let result = execute(&ctx, &node, vec![Value::Bool(false)])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_branch_missing_condition_defaults_false() {
        let ctx = test_ctx().await;
        let node = branch_node();
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_branch_non_bool_input_defaults_false() {
        let ctx = test_ctx().await;
        let node = branch_node();
        let result = execute(&ctx, &node, vec![Value::String("hello".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(false));
    }
}
