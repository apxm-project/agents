//! IDENTITY operation - Identity passthrough with AAM transition recorded

use super::{ExecutionContext, Node, Result, Value, get_input};
use crate::aam::TransitionLabel;
use apxm_core::constants::runtime::{belief_keys, transition_labels};

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let value = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        Value::Null
    };

    // Record an identity transition in the AAM (state unchanged but recorded)
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::IDENTITY_NODE_PREFIX, node.id),
        value.clone(),
        TransitionLabel::Custom(transition_labels::IDENTITY.to_string()),
    );

    Ok(value)
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
            id: 42,
            op_type: AISOperationType::Identity,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        }
    }

    #[tokio::test]
    async fn test_identity_passthrough() {
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
        let input = Value::String("pass_through".to_string());
        let result = execute(&ctx, &node, vec![input.clone()]).await.unwrap();
        assert_eq!(result, input);
    }

    #[tokio::test]
    async fn test_identity_no_input_returns_null() {
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

    #[tokio::test]
    async fn test_identity_records_aam_transition() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_node();
        let input = Value::String("tracked".to_string());
        let _result = execute(&ctx, &node, vec![input.clone()]).await.unwrap();

        // Verify belief was recorded in AAM
        let beliefs = aam.beliefs();
        let key = format!("{}{}", belief_keys::IDENTITY_NODE_PREFIX, node.id);
        assert_eq!(beliefs.get(&key), Some(&input));
    }
}
