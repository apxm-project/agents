//! RETURN operation - Early return

use super::{ExecutionContext, Node, Result, Value, get_input};

pub async fn execute(_ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Return first input or Null
    if !inputs.is_empty() {
        Ok(get_input(node, &inputs, 0)?)
    } else {
        Ok(Value::Null)
    }
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

    fn return_node() -> Node {
        Node::new(1, AISOperationType::Return)
    }

    #[tokio::test]
    async fn test_return_with_value() {
        let ctx = test_ctx().await;
        let node = return_node();
        let result = execute(&ctx, &node, vec![Value::String("result".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("result".to_string()));
    }

    #[tokio::test]
    async fn test_return_no_inputs_returns_null() {
        let ctx = test_ctx().await;
        let node = return_node();
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::Null);
    }

    #[tokio::test]
    async fn test_return_passes_first_input_only() {
        let ctx = test_ctx().await;
        let node = return_node();
        let result = execute(
            &ctx,
            &node,
            vec![Value::Bool(true), Value::String("ignored".to_string())],
        )
        .await
        .unwrap();
        assert_eq!(result, Value::Bool(true));
    }
}
