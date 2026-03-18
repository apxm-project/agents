//! WAITALL operation - Wait for all dependencies

use super::{ExecutionContext, Node, Result, Value};

pub async fn execute(_ctx: &ExecutionContext, _node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // WAITALL simply passes through all inputs as an array
    // The actual synchronization is handled by the scheduler
    Ok(Value::Array(inputs))
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
            op_type: AISOperationType::WaitAll,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        }
    }

    #[tokio::test]
    async fn test_wait_all_collects_inputs() {
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
        let inputs = vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
            Value::String("c".to_string()),
        ];
        let result = execute(&ctx, &node, inputs.clone()).await.unwrap();
        assert_eq!(result, Value::Array(inputs));
    }

    #[tokio::test]
    async fn test_wait_all_empty_inputs() {
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
        assert_eq!(result, Value::Array(vec![]));
    }
}
