use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::execution::{ExecutionDag, Node};
use apxm_core::types::{AISOperationType, Number, Value};
use std::collections::HashMap;

use crate::air_builder::AirModule;

mod tokenizer_model_patterns {
    pub const GPT_4_PREFIX: &str = "gpt-4";
    pub const GPT_4O_PREFIX: &str = "gpt-4o";
    pub const GPT_35_PREFIX: &str = "gpt-3.5";
    pub const TEXT_EMBEDDING_ADA: &str = "text-embedding-ada";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenizerFamily {
    Cl100kBase,
    O200kBase,
}

impl TokenizerFamily {
    fn for_model(model: Option<&str>) -> Self {
        match model {
            Some(model) if is_cl100k_model(model) => Self::Cl100kBase,
            _ => Self::O200kBase,
        }
    }

    fn tokenizer(self) -> &'static bpe_openai::Tokenizer {
        match self {
            Self::Cl100kBase => bpe_openai::cl100k_base(),
            Self::O200kBase => bpe_openai::o200k_base(),
        }
    }
}

/// Select the BPE estimator used for compiler scheduling hints.
///
/// Exact provider billing is collected from runtime responses. These estimates
/// are compiler-side cost/scheduling hints for MLIR passes and graph-aware
/// backends, so unknown model families use the project-wide default estimator.
fn tokenizer_for_model(model: Option<&str>) -> &'static bpe_openai::Tokenizer {
    TokenizerFamily::for_model(model).tokenizer()
}

/// Return the compiler tokenizer family selected for a model.
pub fn tokenizer_name_for_model(model: Option<&str>) -> &'static str {
    match TokenizerFamily::for_model(model) {
        TokenizerFamily::Cl100kBase => "cl100k_base",
        TokenizerFamily::O200kBase => "o200k_base",
    }
}

/// Count text tokens with APXM's compiler-side BPE tokenizer.
///
/// Runtime responses remain the source of truth for provider billing. This
/// helper exposes the same compiler-side estimator used for budgeting,
/// scheduling, and graph metadata diagnostics.
pub fn count_text_tokens(model: Option<&str>, text: &str) -> usize {
    tokenizer_for_model(model).count(text)
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

/// Estimate the identical static leading prefix of one template for a model.
///
/// The caller supplies a template message channel that is eligible for prefix
/// reuse. Empty prefixes have no reusable static leading segment and produce
/// no estimate.
pub fn shared_prefix_token_estimate(model: Option<&str>, template: &str) -> Option<u32> {
    let prefix = static_leading_template_prefix(template);
    if prefix.is_empty() {
        return None;
    }

    Some(u32::try_from(tokenizer_for_model(model).count(prefix)).unwrap_or(u32::MAX))
}

/// Returns true for models that use the cl100k_base tokenizer.
fn is_cl100k_model(model: &str) -> bool {
    use tokenizer_model_patterns as patterns;

    let m = model.to_ascii_lowercase();
    (m.starts_with(patterns::GPT_4_PREFIX) && !m.starts_with(patterns::GPT_4O_PREFIX))
        || m.starts_with(patterns::GPT_35_PREFIX)
        || m.contains(patterns::TEXT_EMBEDDING_ADA)
}

/// Annotate pre-lowering LLM nodes with BPE token counts.
///
/// Sets whole-template and canonical shared-prefix estimates on each
/// ASK/THINK/REASON node so MLIR passes can consume tokenizer-backed values.
pub fn annotate_token_estimates(module: &mut AirModule) {
    annotate_shared_prefix_token_estimates(module);

    let mlir_key = mlir_attr_key(graph_attrs::EST_TEMPLATE_TOKENS);
    for node in &mut module.nodes {
        if !is_llm_template_op(node.op) {
            continue;
        }
        let template = match node.attributes.get(graph_attrs::TEMPLATE_STR) {
            Some(Value::String(value)) => value.clone(),
            _ => continue,
        };
        let static_text = strip_placeholders(&template);
        if static_text.is_empty() {
            continue;
        }
        let model = node
            .attributes
            .get(graph_attrs::MODEL)
            .and_then(|v| v.as_str());
        let count = tokenizer_for_model(model).count(&static_text);
        node.attributes.insert(
            mlir_key.clone(),
            Value::Number(Number::Integer(i64::try_from(count).unwrap_or(i64::MAX))),
        );
    }
}

/// Stamp pre-lowering AirModules with canonical shared-prefix estimates.
///
/// Raw AIR text bypasses this AirModule lowering hook. Artifact finalization
/// still materializes its canonical estimate, but does not retroactively
/// nominate a pre-dispatch warmup candidate.
pub fn annotate_shared_prefix_token_estimates(module: &mut AirModule) {
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
                let model = node
                    .attributes
                    .get(graph_attrs::MODEL)
                    .and_then(Value::as_str);
                shared_prefix_token_estimate(model, template)
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
/// A reuse group receives an estimate only when every member has the same
/// declared model and identical static leading template segment. This keeps
/// the estimate tied to one tokenizer and prevents an invalid group from
/// producing a warmup hint.
pub fn refine_token_estimates(dags: &mut [ExecutionDag]) {
    for dag in dags.iter_mut() {
        let mut groups = HashMap::<String, Vec<usize>>::new();
        for (index, node) in dag.nodes.iter_mut().enumerate() {
            match node.get_attribute(graph_attrs::REUSE_GROUP) {
                Some(Value::String(group)) => {
                    groups.entry(group.clone()).or_default().push(index);
                }
                _ => clear_shared_prefix_hints(node),
            }
        }

        for members in groups.values() {
            let estimate = shared_prefix_group_estimate(&dag.nodes, members);
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

fn shared_prefix_group_estimate(nodes: &[Node], members: &[usize]) -> Option<u32> {
    let first = nodes.get(*members.first()?)?;
    let model = node_model(first);
    let prefix = static_leading_template_prefix(node_template(first)?);
    if prefix.is_empty() {
        return None;
    }

    for &index in members.iter().skip(1) {
        let node = nodes.get(index)?;
        if node_model(node) != model
            || static_leading_template_prefix(node_template(node)?) != prefix
        {
            return None;
        }
    }

    shared_prefix_token_estimate(model, prefix)
}

fn node_template(node: &Node) -> Option<&str> {
    node.get_attribute(graph_attrs::TEMPLATE_STR)
        .and_then(Value::as_str)
}

fn node_model(node: &Node) -> Option<&str> {
    node.get_attribute(graph_attrs::MODEL)
        .and_then(Value::as_str)
}

fn clear_shared_prefix_hints(node: &mut Node) {
    node.attributes
        .remove(graph_attrs::SHARED_PREFIX_EST_TOKENS);
    node.attributes.remove(graph_attrs::WARMUP_CANDIDATE);
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
    use apxm_core::types::execution::Node;
    use std::collections::HashMap;

    fn llm_air_node(template: &str) -> AirNode {
        let mut attributes = HashMap::new();
        attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            Value::String(template.to_string()),
        );
        attributes.insert(
            graph_attrs::MODEL.to_string(),
            Value::String("gpt-4o-mini".to_string()),
        );
        AirNode {
            id: 1,
            name: "ask".to_string(),
            op: AISOperationType::Ask,
            attributes,
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
    fn direct_air_estimator_stamps_the_mlir_prefix_attribute() {
        let template = "System policy: {question}\nTail: static";
        let mut module = AirModule {
            name: "prefix".to_string(),
            nodes: vec![llm_air_node(template)],
            edges: Vec::new(),
            parameters: Vec::new(),
            metadata: HashMap::new(),
        };

        annotate_shared_prefix_token_estimates(&mut module);

        let estimate = module.nodes[0]
            .attributes
            .get(&mlir_attr_key(graph_attrs::SHARED_PREFIX_EST_TOKENS))
            .and_then(Value::as_u64);
        assert_eq!(
            estimate,
            shared_prefix_token_estimate(Some("gpt-4o-mini"), template).map(u64::from)
        );
    }

    #[test]
    fn refinement_repairs_legacy_full_template_static_estimates() {
        let template = "System policy: {question}\nTail: static";
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
            Value::String("gpt-4o-mini".to_string()),
        );
        node.set_attribute(
            graph_attrs::SHARED_PREFIX_EST_TOKENS.to_string(),
            Value::Number(Number::Integer(
                i64::try_from(count_text_tokens(
                    Some("gpt-4o-mini"),
                    &strip_placeholders(template),
                ))
                .unwrap(),
            )),
        );

        let mut dag = ExecutionDag::new();
        dag.nodes.push(node);
        refine_token_estimates(std::slice::from_mut(&mut dag));

        assert_eq!(
            dag.nodes[0]
                .get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                .and_then(Value::as_u64),
            shared_prefix_token_estimate(Some("gpt-4o-mini"), template).map(u64::from)
        );
    }

    #[test]
    fn refinement_preserves_a_matching_canonical_estimate() {
        let template = "Static policy: {question}";
        let expected = shared_prefix_token_estimate(Some("gpt-4o-mini"), template).unwrap();
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
            Value::String("gpt-4o-mini".to_string()),
        );
        node.set_attribute(
            graph_attrs::SHARED_PREFIX_EST_TOKENS.to_string(),
            Value::Number(Number::Integer(i64::from(expected))),
        );

        let mut dag = ExecutionDag::new();
        dag.nodes.push(node);
        refine_token_estimates(std::slice::from_mut(&mut dag));

        assert_eq!(
            dag.nodes[0]
                .get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                .and_then(Value::as_u64),
            Some(u64::from(expected))
        );
    }

    #[test]
    fn refinement_withholds_hints_when_group_members_disagree() {
        let mut first = Node::new(1, AISOperationType::Ask);
        let mut second = Node::new(2, AISOperationType::Ask);
        for (node, template, model) in [
            (&mut first, "Static policy: {question}", "gpt-4o-mini"),
            (&mut second, "Different policy: {question}", "gpt-4"),
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
        refine_token_estimates(std::slice::from_mut(&mut dag));

        for node in dag.nodes {
            assert!(
                node.get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                    .is_none()
            );
            assert!(node.get_attribute(graph_attrs::WARMUP_CANDIDATE).is_none());
        }
    }
}
