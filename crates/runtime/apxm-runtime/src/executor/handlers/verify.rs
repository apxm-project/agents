//! VERIFY operation - Verification with LLM

use super::{
    ExecutionContext, Node, Result, Value, apply_llm_request_routing_from_node,
    execute_llm_request_for_node, get_input, get_optional_string_attribute, get_string_attribute,
    llm::attach_graph_hints,
    template::{input_names_from_node, render_named},
};
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let mut claim = get_string_attribute(node, graph_attrs::CLAIM_TEXT)?;
    let input_names = input_names_from_node(node);
    if !inputs.is_empty() && !input_names.is_empty() {
        claim = render_named(&claim, &inputs, &input_names)?;
    }

    // Get optional evidence attribute, otherwise use input token 0.
    let mut evidence = match get_optional_string_attribute(node, graph_attrs::EVIDENCE)? {
        Some(e) => e,
        None => {
            if !inputs.is_empty() {
                format!("{:?}", get_input(node, &inputs, 0)?)
            } else {
                return Err(apxm_core::error::RuntimeError::Operation {
                    op_type: node.op_type,
                    message: "VERIFY requires either an evidence attribute or one input value"
                        .to_string(),
                });
            }
        }
    };
    if !inputs.is_empty() && !input_names.is_empty() {
        evidence = render_named(&evidence, &inputs, &input_names)?;
    }

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
    let request = attach_graph_hints(ctx, node, request);
    let response = execute_llm_request_for_node(ctx, node, "VERIFY", &request).await?;

    let is_verified = response.content.to_lowercase().contains("true");

    // Record verification result in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
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
    async fn test_verify_missing_claim_is_error() {
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
    async fn test_verify_missing_evidence_is_error() {
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

        let node = make_verify_node_with_claim("value is positive", None);
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
