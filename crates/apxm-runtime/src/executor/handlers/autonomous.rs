//! AUTONOMOUS operation - Autonomous execution stub
//!
//! Placeholder for future autonomous agent execution mode.
//! Currently passes through the first input and records an AAM transition.

use super::{ExecutionContext, Node, Result, Value, get_input};
use crate::aam::TransitionLabel;
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let value = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        Value::Null
    };

    // Record autonomous transition in AAM
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::AUTONOMOUS_NODE_PREFIX, node.id),
        value.clone(),
        TransitionLabel::operation(node.id, node.op_type.to_string()),
    );

    tracing::info!(
        execution_id = %ctx.execution_id,
        node_id = node.id,
        "AUTONOMOUS operation executed (stub)"
    );

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::runtime::belief_keys;
    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::operations::AISOperationType;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_autonomous_node(id: u64) -> apxm_core::types::execution::Node {
        apxm_core::types::execution::Node {
            id,
            op_type: AISOperationType::Autonomous,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        }
    }

    #[tokio::test]
    async fn test_autonomous_passthrough_with_input() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = make_autonomous_node(42);
        let input_value = Value::String("test data".to_string());
        let result = execute(&ctx, &node, vec![input_value.clone()])
            .await
            .unwrap();

        assert_eq!(result, input_value, "Should pass through the first input");
    }

    #[tokio::test]
    async fn test_autonomous_null_without_input() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = make_autonomous_node(1);
        let result = execute(&ctx, &node, vec![]).await.unwrap();

        assert_eq!(result, Value::Null, "Should return Null when no inputs");
    }

    #[tokio::test]
    async fn test_autonomous_records_aam_transition() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_autonomous_node(7);
        let input = Value::String("autonomy data".to_string());
        let _ = execute(&ctx, &node, vec![input.clone()]).await.unwrap();

        // Check AAM has the autonomous node belief recorded
        let beliefs = ctx.aam.beliefs();
        let key = format!("{}7", belief_keys::AUTONOMOUS_NODE_PREFIX);
        assert_eq!(
            beliefs.get(&key),
            Some(&input),
            "AAM should record the autonomous transition with the input value"
        );
    }

    #[tokio::test]
    async fn test_autonomous_records_null_transition_for_empty_input() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_autonomous_node(3);
        let _ = execute(&ctx, &node, vec![]).await.unwrap();

        let beliefs = ctx.aam.beliefs();
        let key = format!("{}3", belief_keys::AUTONOMOUS_NODE_PREFIX);
        assert_eq!(
            beliefs.get(&key),
            Some(&Value::Null),
            "AAM should record Null for autonomous node with no inputs"
        );
    }

    #[tokio::test]
    async fn test_autonomous_uses_first_input_only() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = make_autonomous_node(1);
        let inputs = vec![
            Value::String("first".to_string()),
            Value::String("second".to_string()),
        ];
        let result = execute(&ctx, &node, inputs).await.unwrap();

        assert_eq!(
            result,
            Value::String("first".to_string()),
            "Should use only the first input"
        );
    }
}
