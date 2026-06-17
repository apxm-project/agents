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
