//! ERR operation - Create error value

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let message = get_string_attribute(node, graph_attrs::MESSAGE)?;

    // Record error creation in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::ERR_PREFIX,
            ctx.execution_id,
            node.id
        ),
        Value::String(message.clone()),
        label,
    );

    // Return error message as string value
    // Actual error propagation handled by scheduler
    Ok(Value::String(format!("Error: {}", message)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    fn make_node(message: &str) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::Err,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::MESSAGE.to_string(),
            Value::String(message.to_string()),
        );
        node
    }

    #[tokio::test]
    async fn test_err_creates_error_value() {
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

        let node = make_node("something went wrong");
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(
            result,
            Value::String("Error: something went wrong".to_string())
        );
    }

    #[tokio::test]
    async fn test_err_records_aam_belief() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_node("test error");
        let _result = execute(&ctx, &node, vec![]).await.unwrap();

        // Verify the error was recorded in AAM
        let beliefs = aam.beliefs();
        let key = format!(
            "{}{}:{}",
            belief_keys::ERR_PREFIX,
            ctx.execution_id,
            node.id
        );
        assert_eq!(
            beliefs.get(&key),
            Some(&Value::String("test error".to_string()))
        );
    }

    #[tokio::test]
    async fn test_err_missing_message_attribute() {
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
            op_type: AISOperationType::Err,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }
}
