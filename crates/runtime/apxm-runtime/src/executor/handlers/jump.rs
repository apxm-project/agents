//! JUMP operation - Unconditional jump

use super::{ExecutionContext, Node, Result, Value};

pub async fn execute(_ctx: &ExecutionContext, _node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Pass through first input or Null
    inputs
        .first()
        .cloned()
        .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
            op_type: apxm_core::types::operations::AISOperationType::Jump,
            message: "JUMP requires at least one input".to_string(),
        })
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

    fn jump_node() -> Node {
        Node::new(1, AISOperationType::Jump)
    }

    #[tokio::test]
    async fn test_jump_passes_through_value() {
        let ctx = test_ctx().await;
        let node = jump_node();
        let result = execute(&ctx, &node, vec![Value::String("target".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("target".to_string()));
    }

    #[tokio::test]
    async fn test_jump_passes_through_first_of_multiple() {
        let ctx = test_ctx().await;
        let node = jump_node();
        let result = execute(
            &ctx,
            &node,
            vec![
                Value::String("first".to_string()),
                Value::String("second".to_string()),
            ],
        )
        .await
        .unwrap();
        assert_eq!(result, Value::String("first".to_string()));
    }

    #[tokio::test]
    async fn test_jump_no_inputs_errors() {
        let ctx = test_ctx().await;
        let node = jump_node();
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }
}
