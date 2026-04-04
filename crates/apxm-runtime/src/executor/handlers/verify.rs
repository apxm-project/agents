//! VERIFY operation - Verification with LLM

use super::{
    ExecutionContext, Node, Result, Value, execute_llm_request, get_input,
    get_optional_string_attribute, get_string_attribute,
};
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let condition = get_string_attribute(node, graph_attrs::CONDITION)?;
    let value_to_verify = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        Value::Null
    };

    let verification_prompt = format!(
        "Verify if the following condition is met:\n\nCondition: {}\nValue: {:?}\n\nRespond with 'true' or 'false'.",
        condition, value_to_verify
    );

    // Priority: 1) node attribute (agent context), 2) config instruction, 3) template, 4) hardcoded fallback
    let system_prompt = get_optional_string_attribute(node, graph_attrs::SYSTEM_PROMPT)?
        .or_else(|| ctx.instruction_config.verify.clone())
        .or_else(|| apxm_backends::render_prompt("verify_system", &serde_json::json!({})).ok())
        .unwrap_or_else(|| {
            "You are a verification expert. Analyze conditions precisely. Respond with only 'true' or 'false'.".to_string()
        });

    let request = LLMRequest::new(verification_prompt).with_system_prompt(system_prompt);
    let response = execute_llm_request(ctx, node.id, "VERIFY", &request).await?;

    let is_verified = response.content.to_lowercase().contains("true");

    // Record verification result in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::VERIFY_PREFIX,
            ctx.execution_id,
            node.id
        ),
        Value::Bool(is_verified),
        label,
    );

    Ok(Value::Bool(is_verified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::llm::backends::mock::MockLLMBackend;
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    fn make_verify_node(condition: &str) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::Verify,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::CONDITION.to_string(),
            Value::String(condition.to_string()),
        );
        node
    }

    #[tokio::test]
    async fn test_verify_passing() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        // Register mock backend that always returns "true"
        let mock = MockLLMBackend::static_response("true");
        llm_registry.register("mock", mock).unwrap();
        llm_registry.set_default("mock").unwrap();

        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_verify_node("value > 0");
        let result = execute(&ctx, &node, vec![Value::String("42".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(true));

        // Verify AAM recorded the result
        let beliefs = aam.beliefs();
        let key = format!(
            "{}{}:{}",
            belief_keys::VERIFY_PREFIX,
            ctx.execution_id,
            node.id
        );
        assert_eq!(beliefs.get(&key), Some(&Value::Bool(true)));
    }

    #[tokio::test]
    async fn test_verify_failing() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        // Register mock backend that always returns "false"
        let mock = MockLLMBackend::static_response("false");
        llm_registry.register("mock", mock).unwrap();
        llm_registry.set_default("mock").unwrap();

        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_verify_node("value is negative");
        let result = execute(&ctx, &node, vec![Value::String("42".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_verify_missing_condition() {
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
            op_type: AISOperationType::Verify,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }
}
