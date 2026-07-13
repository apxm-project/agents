use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::execution::{ExecutionDag, Node};
use apxm_core::types::{AISOperationType, Number, Value};
use std::collections::HashMap;

use crate::air_builder::AirModule;
use crate::analysis::{CompilerAnalysisInputs, TokenizerEvidence};

/// Return the BPE tokenizer selected by explicit route capability evidence.
fn tokenizer_for_evidence(evidence: TokenizerEvidence) -> Option<&'static bpe_openai::Tokenizer> {
    match evidence {
        TokenizerEvidence::Unknown => None,
        TokenizerEvidence::Cl100kBase => Some(bpe_openai::cl100k_base()),
        TokenizerEvidence::O200kBase => Some(bpe_openai::o200k_base()),
    }
}

/// Report that model spelling alone cannot select a compiler tokenizer.
///
/// This compatibility helper exists for callers outside the compiler pipeline.
/// Compiler token accounting requires [`TokenizerEvidence`] from a configured
/// route and never derives an encoding from a model or provider name.
pub fn tokenizer_name_for_model(_model: Option<&str>) -> &'static str {
    "unavailable"
}

/// Return a conservative count when no tokenizer capability was configured.
///
/// New compiler code must use [`count_text_tokens_with_evidence`] and handle
/// its typed unavailable result. `usize::MAX` keeps legacy numeric-only callers
/// from treating an unknown count as a permissive token budget.
pub fn count_text_tokens(_model: Option<&str>, _text: &str) -> usize {
    usize::MAX
}

/// Count text tokens with explicitly configured tokenizer evidence.
pub fn count_text_tokens_with_evidence(evidence: TokenizerEvidence, text: &str) -> Option<usize> {
    tokenizer_for_evidence(evidence).map(|tokenizer| tokenizer.count(text))
}

/// Return the static leading segment of a template before its first input placeholder.
///
/// A shared-prefix estimate represents only text that is identical before any
/// request-specific interpolation. Braces that do not contain a valid named
/// placeholder remain literal template text.
pub fn static_leading_template_prefix(template: &str) -> &str {
    let bytes = template.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'{' {
            let start = index + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b'}' {
                return &template[..index];
            }
        }
        index += utf8_len(bytes[index]);
    }

    template
}

/// Estimate the identical static leading prefix of one template with explicit evidence.
///
/// The caller supplies a template message channel that is eligible for prefix
/// reuse. Empty prefixes have no reusable static leading segment and produce
/// no estimate.
pub fn shared_prefix_token_estimate(evidence: TokenizerEvidence, template: &str) -> Option<u32> {
    let prefix = static_leading_template_prefix(template);
    if prefix.is_empty() {
        return None;
    }

    count_text_tokens_with_evidence(evidence, prefix).and_then(|count| u32::try_from(count).ok())
}

/// Annotate pre-lowering LLM nodes with BPE token counts.
///
/// Sets whole-template and canonical shared-prefix estimates on each
/// ASK/THINK/REASON node so MLIR passes can consume tokenizer-backed values.
pub fn annotate_token_estimates(module: &mut AirModule, inputs: &CompilerAnalysisInputs) {
    annotate_shared_prefix_token_estimates(module, inputs);

    let mlir_key = mlir_attr_key(graph_attrs::EST_TEMPLATE_TOKENS);
    for node in &mut module.nodes {
        if !is_llm_template_op(node.op) {
            continue;
        }
        let Some(tokenizer) = inputs.tokenizer_for_attributes(&node.attributes) else {
            node.attributes.remove(&mlir_key);
            continue;
        };
        let template = match node.attributes.get(graph_attrs::TEMPLATE_STR) {
            Some(Value::String(value)) => value.clone(),
            _ => {
                node.attributes.remove(&mlir_key);
                continue;
            }
        };
        let static_text = strip_placeholders(&template);
        if static_text.is_empty() {
            node.attributes.remove(&mlir_key);
            continue;
        }
        let estimate = count_text_tokens_with_evidence(tokenizer, &static_text)
            .and_then(|count| i64::try_from(count).ok());
        match estimate {
            Some(count) => {
                node.attributes
                    .insert(mlir_key.clone(), Value::Number(Number::Integer(count)));
            }
            None => {
                node.attributes.remove(&mlir_key);
            }
        }
    }
}

/// Stamp pre-lowering AirModules with canonical shared-prefix estimates.
///
/// Raw AIR text bypasses this AirModule lowering hook. Artifact finalization
/// still materializes its canonical estimate, but does not retroactively
/// nominate a pre-dispatch warmup candidate.
pub fn annotate_shared_prefix_token_estimates(
    module: &mut AirModule,
    inputs: &CompilerAnalysisInputs,
) {
    let mlir_key = mlir_attr_key(graph_attrs::SHARED_PREFIX_EST_TOKENS);

    for node in &mut module.nodes {
        if !is_llm_template_op(node.op) {
            continue;
        }

        let estimate = node
            .attributes
            .get(graph_attrs::TEMPLATE_STR)
            .and_then(Value::as_str)
            .and_then(|template| {
                inputs
                    .tokenizer_for_attributes(&node.attributes)
                    .and_then(|evidence| shared_prefix_token_estimate(evidence, template))
            });

        match estimate {
            Some(count) => {
                node.attributes.insert(
                    mlir_key.clone(),
                    Value::Number(Number::Integer(i64::from(count))),
                );
            }
            None => {
                node.attributes.remove(&mlir_key);
            }
        }
    }
}

/// Strip `{name}` placeholders, returning only static text.
///
/// Templates reference inputs by name (`{topic}`, `{question}`, …); the
/// shared `template::parse_placeholder_names` helper defines the exact
/// grammar (`\{(\w+)\}`). Anything that matches that grammar is stripped;
/// braces around non-word content (e.g. `{a-b}`) are preserved as text.
fn strip_placeholders(template: &str) -> String {
    let mut result = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b'}' {
                // Drop `{name}` entirely.
                i = end + 1;
                continue;
            }
        }
        // Pass through one UTF-8 character at the current position.
        let ch_end = i + utf8_len(bytes[i]);
        result.push_str(&template[i..ch_end]);
        i = ch_end;
    }
    result
}

/// UTF-8 byte length of the codepoint starting with `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0xC0 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

/// Materialize canonical shared-prefix estimates in emitted artifacts.
///
/// A reuse group receives an estimate only when every member resolves to the
/// same configured route and has an identical static leading template segment.
/// Missing tokenizer evidence leaves the accounting unavailable.
pub fn refine_token_estimates(dags: &mut [ExecutionDag], inputs: &CompilerAnalysisInputs) {
    for dag in dags.iter_mut() {
        let mut groups = HashMap::<String, Vec<usize>>::new();
        for (index, node) in dag.nodes.iter_mut().enumerate() {
            if is_llm_template_op(node.op_type) && inputs.tokenizer_for_node(node).is_none() {
                clear_token_estimate_hints(node);
            }
            match node.get_attribute(graph_attrs::REUSE_GROUP) {
                Some(Value::String(group)) => {
                    groups.entry(group.clone()).or_default().push(index);
                }
                _ => clear_shared_prefix_hints(node),
            }
        }

        for members in groups.values() {
            let estimate = shared_prefix_group_estimate(&dag.nodes, members, inputs);
            for &index in members {
                let node = &mut dag.nodes[index];
                if let Some(count) = estimate {
                    node.set_attribute(
                        graph_attrs::SHARED_PREFIX_EST_TOKENS.to_string(),
                        Value::Number(Number::Integer(i64::from(count))),
                    );
                } else {
                    clear_shared_prefix_hints(node);
                }
            }
        }
    }
}

fn shared_prefix_group_estimate(
    nodes: &[Node],
    members: &[usize],
    inputs: &CompilerAnalysisInputs,
) -> Option<u32> {
    let first = nodes.get(*members.first()?)?;
    let tokenizer = inputs.tokenizer_for_node(first)?;
    let prefix = static_leading_template_prefix(node_template(first)?);
    if prefix.is_empty() {
        return None;
    }

    for &index in members.iter().skip(1) {
        let node = nodes.get(index)?;
        if !inputs.same_configured_backend(first, node)
            || inputs.tokenizer_for_node(node) != Some(tokenizer)
            || static_leading_template_prefix(node_template(node)?) != prefix
        {
            return None;
        }
    }

    shared_prefix_token_estimate(tokenizer, prefix)
}

fn node_template(node: &Node) -> Option<&str> {
    node.get_attribute(graph_attrs::TEMPLATE_STR)
        .and_then(Value::as_str)
}

fn clear_shared_prefix_hints(node: &mut Node) {
    node.attributes
        .remove(graph_attrs::SHARED_PREFIX_EST_TOKENS);
    node.attributes.remove(graph_attrs::WARMUP_CANDIDATE);
}

/// Remove unproven whole-template and shared-prefix accounting hints.
fn clear_token_estimate_hints(node: &mut Node) {
    node.attributes.remove(graph_attrs::EST_TEMPLATE_TOKENS);
    clear_shared_prefix_hints(node);
}

fn mlir_attr_key(attr: &str) -> String {
    format!("{}{}", graph_attrs::MLIR_ATTR_PREFIX, attr)
}

fn is_llm_template_op(op: AISOperationType) -> bool {
    matches!(
        op,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_builder::{AirModule, AirNode};
    use crate::{BackendCapabilityEvidence, ConfiguredBackendEvidence};
    use apxm_core::types::execution::Node;
    use std::collections::HashMap;

    fn llm_air_node(template: &str, model: &str) -> AirNode {
        let mut attributes = HashMap::new();
        attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            Value::String(template.to_string()),
        );
        attributes.insert(
            graph_attrs::MODEL.to_string(),
            Value::String(model.to_string()),
        );
        AirNode {
            id: 1,
            name: "ask".to_string(),
            op: AISOperationType::Ask,
            attributes,
        }
    }

    fn configured_inputs(model: &str, tokenizer: TokenizerEvidence) -> CompilerAnalysisInputs {
        CompilerAnalysisInputs {
            configured_backends: vec![ConfiguredBackendEvidence {
                backend: "configured-backend".to_string(),
                model: model.to_string(),
                aliases: Default::default(),
                available: true,
                capabilities: BackendCapabilityEvidence {
                    tokenizer,
                    ..Default::default()
                },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn shared_prefix_stops_at_the_first_dynamic_placeholder() {
        let template = "System policy:\nAnswer carefully.\nQuestion: {question}\nTail: static";
        let prefix = static_leading_template_prefix(template);

        assert_eq!(prefix, "System policy:\nAnswer carefully.\nQuestion: ");
        assert_ne!(prefix, strip_placeholders(template));
    }

    #[test]
    fn configured_tokenizer_evidence_stamps_the_mlir_prefix_attribute() {
        let template = "System policy: {question}\nTail: static";
        let tokenizer = TokenizerEvidence::O200kBase;
        let inputs = configured_inputs("configured-model", tokenizer);
        let mut module = AirModule {
            name: "prefix".to_string(),
            nodes: vec![llm_air_node(template, "configured-model")],
            edges: Vec::new(),
            parameters: Vec::new(),
            metadata: HashMap::new(),
        };

        annotate_shared_prefix_token_estimates(&mut module, &inputs);

        let estimate = module.nodes[0]
            .attributes
            .get(&mlir_attr_key(graph_attrs::SHARED_PREFIX_EST_TOKENS))
            .and_then(Value::as_u64);
        assert_eq!(
            estimate,
            shared_prefix_token_estimate(tokenizer, template).map(u64::from)
        );
    }

    #[test]
    fn model_spellings_do_not_select_tokenizers_without_configured_evidence() {
        let template = "System policy: {question}";
        for model in ["gpt-4o-mini", "provider/custom-model", "model-v1"] {
            let mut module = AirModule {
                name: "unknown-tokenizer".to_string(),
                nodes: vec![llm_air_node(template, model)],
                edges: Vec::new(),
                parameters: Vec::new(),
                metadata: HashMap::new(),
            };

            annotate_token_estimates(&mut module, &CompilerAnalysisInputs::default());

            assert_eq!(count_text_tokens(Some(model), template), usize::MAX);
            assert!(
                module.nodes[0]
                    .attributes
                    .get(&mlir_attr_key(graph_attrs::EST_TEMPLATE_TOKENS))
                    .is_none()
            );
            assert!(
                module.nodes[0]
                    .attributes
                    .get(&mlir_attr_key(graph_attrs::SHARED_PREFIX_EST_TOKENS))
                    .is_none()
            );
        }
    }

    #[test]
    fn refinement_repairs_legacy_full_template_static_estimates() {
        let template = "System policy: {question}\nTail: static";
        let tokenizer = TokenizerEvidence::O200kBase;
        let inputs = configured_inputs("configured-model", tokenizer);
        let mut node = Node::new(1, AISOperationType::Ask);
        node.set_attribute(
            graph_attrs::REUSE_GROUP.to_string(),
            Value::String("shared".to_string()),
        );
        node.set_attribute(
            graph_attrs::TEMPLATE_STR.to_string(),
            Value::String(template.to_string()),
        );
        node.set_attribute(
            graph_attrs::MODEL.to_string(),
            Value::String("configured-model".to_string()),
        );
        node.set_attribute(
            graph_attrs::SHARED_PREFIX_EST_TOKENS.to_string(),
            Value::Number(Number::Integer(
                i64::try_from(
                    count_text_tokens_with_evidence(tokenizer, &strip_placeholders(template))
                        .expect("configured tokenizer"),
                )
                .unwrap(),
            )),
        );

        let mut dag = ExecutionDag::new();
        dag.nodes.push(node);
        refine_token_estimates(std::slice::from_mut(&mut dag), &inputs);

        assert_eq!(
            dag.nodes[0]
                .get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                .and_then(Value::as_u64),
            shared_prefix_token_estimate(tokenizer, template).map(u64::from)
        );
    }

    #[test]
    fn refinement_preserves_a_matching_canonical_estimate() {
        let template = "Static policy: {question}";
        let tokenizer = TokenizerEvidence::O200kBase;
        let inputs = configured_inputs("configured-model", tokenizer);
        let expected = shared_prefix_token_estimate(tokenizer, template).unwrap();
        let mut node = Node::new(1, AISOperationType::Ask);
        node.set_attribute(
            graph_attrs::REUSE_GROUP.to_string(),
            Value::String("shared".to_string()),
        );
        node.set_attribute(
            graph_attrs::TEMPLATE_STR.to_string(),
            Value::String(template.to_string()),
        );
        node.set_attribute(
            graph_attrs::MODEL.to_string(),
            Value::String("configured-model".to_string()),
        );
        node.set_attribute(
            graph_attrs::SHARED_PREFIX_EST_TOKENS.to_string(),
            Value::Number(Number::Integer(i64::from(expected))),
        );

        let mut dag = ExecutionDag::new();
        dag.nodes.push(node);
        refine_token_estimates(std::slice::from_mut(&mut dag), &inputs);

        assert_eq!(
            dag.nodes[0]
                .get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                .and_then(Value::as_u64),
            Some(u64::from(expected))
        );
    }

    #[test]
    fn refinement_withholds_hints_when_group_members_disagree() {
        let inputs = configured_inputs("configured-model", TokenizerEvidence::O200kBase);
        let mut first = Node::new(1, AISOperationType::Ask);
        let mut second = Node::new(2, AISOperationType::Ask);
        for (node, template, model) in [
            (&mut first, "Static policy: {question}", "configured-model"),
            (
                &mut second,
                "Different policy: {question}",
                "configured-model",
            ),
        ] {
            node.set_attribute(
                graph_attrs::REUSE_GROUP.to_string(),
                Value::String("shared".to_string()),
            );
            node.set_attribute(
                graph_attrs::TEMPLATE_STR.to_string(),
                Value::String(template.to_string()),
            );
            node.set_attribute(
                graph_attrs::MODEL.to_string(),
                Value::String(model.to_string()),
            );
            node.set_attribute(
                graph_attrs::SHARED_PREFIX_EST_TOKENS.to_string(),
                Value::Number(Number::Integer(99)),
            );
            node.set_attribute(graph_attrs::WARMUP_CANDIDATE.to_string(), Value::Bool(true));
        }

        let mut dag = ExecutionDag::new();
        dag.nodes = vec![first, second];
        refine_token_estimates(std::slice::from_mut(&mut dag), &inputs);

        for node in dag.nodes {
            assert!(
                node.get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                    .is_none()
            );
            assert!(node.get_attribute(graph_attrs::WARMUP_CANDIDATE).is_none());
        }
    }
}
