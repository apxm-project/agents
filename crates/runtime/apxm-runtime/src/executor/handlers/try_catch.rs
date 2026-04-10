//! TRYCATCH operation - Exception handling

use super::{ExecutionContext, Node, Result, Value};

pub async fn execute(_ctx: &ExecutionContext, _node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Pass through inputs - actual exception handling done by scheduler
    Ok(inputs.first().cloned().unwrap_or(Value::Null))
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
            op_type: AISOperationType::TryCatch,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        }
    }

    #[tokio::test]
    async fn test_try_catch_passes_through_no_error() {
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
        let input = Value::String("success_value".to_string());
        let result = execute(&ctx, &node, vec![input.clone()]).await.unwrap();
        assert_eq!(result, input);
    }

    #[tokio::test]
    async fn test_try_catch_with_error_value() {
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
        let error_val = Value::String("Error: something failed".to_string());
        let result = execute(&ctx, &node, vec![error_val.clone()]).await.unwrap();
        assert_eq!(result, error_val);
    }

    #[tokio::test]
    async fn test_try_catch_no_inputs_returns_null() {
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
