//! FENCE operation - Memory fence/barrier

use super::{ExecutionContext, Node, Result, Value, get_input};

pub async fn execute(_ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // FENCE ensures ordering - pass through first input or Null
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

    fn make_node() -> Node {
        Node {
            id: 1,
            op_type: AISOperationType::Fence,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        }
    }

    #[tokio::test]
    async fn test_fence_passthrough_with_input() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = make_node();
        let input = Value::String("barrier_data".to_string());
        let result = execute(&ctx, &node, vec![input.clone()]).await.unwrap();
        assert_eq!(result, input);
    }

    #[tokio::test]
    async fn test_fence_no_inputs_returns_null() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = make_node();
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::Null);
    }
}
