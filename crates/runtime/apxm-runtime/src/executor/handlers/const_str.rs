//! CONST_STR operation - String constant

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;

pub async fn execute(_ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let mut value = get_string_attribute(node, graph_attrs::VALUE)?;
    for (i, input) in inputs.iter().enumerate() {
        if let Some(s) = input.as_string() {
            value = value.replace(&format!("{{{i}}}"), s);
        }
    }
    Ok(Value::String(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    fn make_node(value: &str) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::VALUE.to_string(),
            Value::String(value.to_string()),
        );
        node
    }

    #[tokio::test]
    async fn test_const_str_returns_configured_value() {
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

        let node = make_node("hello world");
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("hello world".to_string()));
    }

    #[tokio::test]
    async fn test_const_str_ignores_inputs_without_placeholders() {
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

        let node = make_node("constant");
        let result = execute(&ctx, &node, vec![Value::String("ignored".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("constant".to_string()));
    }

    #[tokio::test]
    async fn test_const_str_substitutes_placeholders() {
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

        let node = make_node("hello {0}, meet {1}");
        let result = execute(
            &ctx,
            &node,
            vec![
                Value::String("world".to_string()),
                Value::String("rust".to_string()),
            ],
        )
        .await
        .unwrap();
        assert_eq!(result, Value::String("hello world, meet rust".to_string()));
    }

    #[tokio::test]
    async fn test_const_str_missing_value_attribute() {
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

        let node = Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }
}
