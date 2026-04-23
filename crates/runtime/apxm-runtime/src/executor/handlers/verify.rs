//! VERIFY operation - Verification with LLM

use super::{
    ExecutionContext, Node, Result, Value, apply_llm_request_routing_from_node,
    execute_llm_request, get_input, get_optional_string_attribute, get_string_attribute,
};
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Try claim first (new schema), fall back to condition (legacy)
    let claim = match get_optional_string_attribute(node, graph_attrs::CLAIM_TEXT)? {
        Some(c) => c,
        None => get_string_attribute(node, graph_attrs::CONDITION)?,
    };

    // Get optional evidence attribute, otherwise use input token 0
    let evidence = match get_optional_string_attribute(node, graph_attrs::EVIDENCE)? {
        Some(e) => e,
        None => {
            if !inputs.is_empty() {
                format!("{:?}", get_input(node, &inputs, 0)?)
            } else {
                "No evidence provided.".to_string()
            }
        }
    };

    // Build a richer prompt when evidence is provided
    let verification_prompt = format!(
        "Verify whether the following claim is true given the evidence.\n\nClaim: {}\n\nEvidence:\n{}\n\nRespond with 'true' or 'false' and a one-sentence explanation.",
        claim, evidence
    );

    // Priority: 1) node attribute (agent context), 2) config instruction, 3) template, 4) hardcoded fallback
    let system_prompt = get_optional_string_attribute(node, graph_attrs::SYSTEM_PROMPT)?
        .or_else(|| ctx.instruction_config.verify.clone())
        .or_else(|| apxm_backends::render_prompt("verify_system", &serde_json::json!({})).ok())
        .unwrap_or_else(|| {
            "You are a verification expert. Analyze claims and evidence precisely. Respond with only 'true' or 'false' followed by a brief explanation.".to_string()
        });

    let request = apply_llm_request_routing_from_node(
        LLMRequest::new(verification_prompt).with_system_prompt(system_prompt),
        node,
    )?;
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

    fn make_verify_node_with_claim(claim: &str, evidence: Option<&str>) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::Verify,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::CLAIM_TEXT.to_string(),
            Value::String(claim.to_string()),
        );
        if let Some(ev) = evidence {
            node.attributes.insert(
                graph_attrs::EVIDENCE.to_string(),
                Value::String(ev.to_string()),
            );
        }
        node
    }

    fn make_verify_node_legacy(condition: &str) -> Node {
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
    async fn test_verify_with_claim_and_evidence() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let mock = MockLLMBackend::static_response("true - the value is positive");
        llm_registry.register("mock", mock).unwrap();
        llm_registry.set_default("mock").unwrap();

        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_verify_node_with_claim("value is positive", Some("The value is 42"));
        let result = execute(&ctx, &node, vec![]).await.unwrap();
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
    async fn test_verify_legacy_condition() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let mock = MockLLMBackend::static_response("true");
        llm_registry.register("mock", mock).unwrap();
        llm_registry.set_default("mock").unwrap();

        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_verify_node_legacy("value > 0");
        let result = execute(&ctx, &node, vec![Value::String("42".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[tokio::test]
    async fn test_verify_failing() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let mock = MockLLMBackend::static_response("false - the claim is incorrect");
        llm_registry.register("mock", mock).unwrap();
        llm_registry.set_default("mock").unwrap();

        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_verify_node_with_claim("value is negative", Some("The value is 42"));
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_verify_missing_claim_and_condition() {
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

    #[tokio::test]
    async fn test_verify_with_input_as_evidence() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let mock = MockLLMBackend::static_response("true - evidence supports the claim");
        llm_registry.register("mock", mock).unwrap();
        llm_registry.set_default("mock").unwrap();

        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        // No evidence attribute, should use input token 0 as evidence
        let node = make_verify_node_with_claim("data contains important information", None);
        let result = execute(
            &ctx,
            &node,
            vec![Value::String("critical data here".to_string())],
        )
        .await
        .unwrap();
        assert_eq!(result, Value::Bool(true));
    }
}
