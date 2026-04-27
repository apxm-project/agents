use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::{AISOperationType, Number, Value};

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

/// Returns true for models that use the cl100k_base tokenizer.
fn is_cl100k_model(model: &str) -> bool {
    use tokenizer_model_patterns as patterns;

    let m = model.to_ascii_lowercase();
    (m.starts_with(patterns::GPT_4_PREFIX) && !m.starts_with(patterns::GPT_4O_PREFIX))
        || m.starts_with(patterns::GPT_35_PREFIX)
        || m.contains(patterns::TEXT_EMBEDDING_ADA)
}

/// Annotate LLM nodes with BPE token counts before lowering to MLIR.
///
/// Sets `ais.est_template_tokens` on each ASK/THINK/REASON node so MLIR passes
/// can read pre-computed tokenizer values.
pub fn annotate_token_estimates(module: &mut AirModule) {
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
            Value::Number(Number::Integer(count as i64)),
        );
    }
}

/// Strip `{name}` placeholders, returning only static text.
///
/// Templates reference inputs by name (`{topic}`, `{question}`, …); the
/// shared `template::parse_placeholder_names` helper defines the exact
/// grammar (`\{(\w+)\}`). Anything that matches that grammar is removed;
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
    if b < 0x80 {
        1
    } else if b < 0xC0 {
        1 // continuation byte — treat as one (input is assumed valid)
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

/// Refine shared-prefix token estimates using exact BPE tokenization.
///
/// After MLIR compilation, this post-pass replaces shared-prefix estimates
/// with BPE token counts, selecting the tokenizer based on each node's `model`
/// attribute.
pub fn refine_token_estimates(dags: &mut [ExecutionDag]) {
    for dag in dags.iter_mut() {
        for node in dag.nodes.iter_mut() {
            // Only process nodes in a shared prefix group
            if node.get_attribute(graph_attrs::REUSE_GROUP).is_none() {
                continue;
            }
            // Get the template string
            let template = match node.get_attribute(graph_attrs::TEMPLATE_STR) {
                Some(Value::String(s)) => s.clone(),
                _ => continue,
            };
            let static_text = strip_placeholders(&template);
            if static_text.is_empty() {
                continue;
            }
            // Select tokenizer based on the node's model attribute
            let model = node
                .get_attribute(graph_attrs::MODEL)
                .and_then(|v| match v {
                    Value::String(s) => Some(s.as_str()),
                    _ => None,
                });
            let count = tokenizer_for_model(model).count(&static_text);
            node.set_attribute(
                graph_attrs::SHARED_PREFIX_EST_TOKENS.to_string(),
                Value::Number(apxm_core::types::Number::Integer(count as i64)),
            );
        }
    }
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
    use crate::air_builder::AirNode;
    use std::collections::HashMap;

    fn make_llm_node(id: u64, op: AISOperationType, template: &str) -> AirNode {
        let mut attributes = HashMap::new();
        attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            Value::String(template.to_string()),
        );
        AirNode {
            id,
            name: format!("node_{id}"),
            op,
            attributes,
        }
    }

    fn make_module(nodes: Vec<AirNode>) -> AirModule {
        AirModule {
            name: "test".to_string(),
            nodes,
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        }
    }

    fn get_est_tokens(node: &AirNode) -> Option<i64> {
        let key = mlir_attr_key(graph_attrs::EST_TEMPLATE_TOKENS);
        match node.attributes.get(&key) {
            Some(Value::Number(Number::Integer(n))) => Some(*n),
            _ => None,
        }
    }

    #[test]
    fn annotate_sets_token_count_on_ask() {
        let mut module = make_module(vec![make_llm_node(
            1,
            AISOperationType::Ask,
            "What is the meaning of life?",
        )]);
        annotate_token_estimates(&mut module);
        let count = get_est_tokens(&module.nodes[0]).expect("should have est_template_tokens");
        assert!(count > 0);
        assert!(count < 20); // ~7 tokens for this short sentence
    }

    #[test]
    fn annotate_sets_token_count_on_think_and_reason() {
        let mut module = make_module(vec![
            make_llm_node(
                1,
                AISOperationType::Think,
                "Analyze this problem step by step.",
            ),
            make_llm_node(2, AISOperationType::Reason, "Given the evidence, conclude."),
        ]);
        annotate_token_estimates(&mut module);
        assert!(get_est_tokens(&module.nodes[0]).unwrap() > 0);
        assert!(get_est_tokens(&module.nodes[1]).unwrap() > 0);
    }

    #[test]
    fn annotate_skips_non_llm_ops() {
        let mut module = make_module(vec![AirNode {
            id: 1,
            name: "spawn".to_string(),
            op: AISOperationType::SpawnAgent,
            attributes: HashMap::from([(
                graph_attrs::TEMPLATE_STR.to_string(),
                Value::String("some text".to_string()),
            )]),
        }]);
        annotate_token_estimates(&mut module);
        assert!(get_est_tokens(&module.nodes[0]).is_none());
    }

    #[test]
    fn annotate_skips_nodes_without_template() {
        let mut module = make_module(vec![AirNode {
            id: 1,
            name: "bare_ask".to_string(),
            op: AISOperationType::Ask,
            attributes: HashMap::new(),
        }]);
        annotate_token_estimates(&mut module);
        assert!(get_est_tokens(&module.nodes[0]).is_none());
    }

    #[test]
    fn annotate_strips_placeholders_before_counting() {
        let mut module = make_module(vec![make_llm_node(
            1,
            AISOperationType::Ask,
            "Explain {topic} in detail",
        )]);
        annotate_token_estimates(&mut module);
        let count = get_est_tokens(&module.nodes[0]).unwrap();
        // "Explain  in detail" — placeholders stripped
        let expected = bpe_openai::o200k_base().count("Explain  in detail");
        assert_eq!(count, expected as i64);
    }

    #[test]
    fn annotate_placeholder_only_template_gets_no_annotation() {
        let mut module = make_module(vec![make_llm_node(1, AISOperationType::Ask, "{question}")]);
        annotate_token_estimates(&mut module);
        // Static text is empty after stripping → no annotation
        assert!(get_est_tokens(&module.nodes[0]).is_none());
    }

    #[test]
    fn annotate_respects_model_attribute() {
        let mut node = make_llm_node(1, AISOperationType::Ask, "Hello world, this is a test.");
        node.attributes.insert(
            graph_attrs::MODEL.to_string(),
            Value::String("gpt-4".to_string()),
        );
        let mut module = make_module(vec![node]);
        annotate_token_estimates(&mut module);
        let count_cl100k = get_est_tokens(&module.nodes[0]).unwrap();

        let mut node2 = make_llm_node(2, AISOperationType::Ask, "Hello world, this is a test.");
        node2.attributes.insert(
            graph_attrs::MODEL.to_string(),
            Value::String("gpt-4o".to_string()),
        );
        let mut module2 = make_module(vec![node2]);
        annotate_token_estimates(&mut module2);
        let count_o200k = get_est_tokens(&module2.nodes[0]).unwrap();

        // Both should be positive; may differ due to different tokenizers
        assert!(count_cl100k > 0);
        assert!(count_o200k > 0);
    }

    #[test]
    fn annotate_matches_bpe_tokenizer() {
        // Code-heavy text exercises the actual BPE tokenizer rather than a
        // non-tokenizer estimate.
        let code_template = "```rust\nfn main() {\n    println!(\"Hello, world!\");\n    let x = vec![1, 2, 3];\n    for i in &x {\n        println!(\"{}\", i);\n    }\n}\n```";
        let mut module = make_module(vec![make_llm_node(1, AISOperationType::Ask, code_template)]);
        annotate_token_estimates(&mut module);
        let bpe_count = get_est_tokens(&module.nodes[0]).unwrap();
        let expected = bpe_openai::o200k_base().count(code_template);
        assert_eq!(bpe_count as usize, expected);
    }

    #[test]
    fn strip_simple_placeholders() {
        assert_eq!(strip_placeholders("Hello {topic} world"), "Hello  world");
        assert_eq!(strip_placeholders("{a} {b}"), " ");
        assert_eq!(strip_placeholders("no placeholders"), "no placeholders");
    }

    #[test]
    fn strip_preserves_non_word_braces() {
        // `{a-b}` is not a valid placeholder (regex `\{(\w+)\}`) so it
        // remains in the static text.
        assert_eq!(strip_placeholders("{a-b} is {x}"), "{a-b} is ");
    }

    #[test]
    fn strip_empty_input() {
        assert_eq!(strip_placeholders(""), "");
    }

    #[test]
    fn tokenizer_selection_defaults_to_o200k() {
        let tok = tokenizer_for_model(None);
        assert!(tok.count("hello") > 0);
    }

    #[test]
    fn tokenizer_selection_cl100k_for_gpt4() {
        let tok_cl = tokenizer_for_model(Some("gpt-4"));
        let tok_o2 = tokenizer_for_model(Some("gpt-4o"));
        // Both should produce valid counts (different tokenizers)
        assert!(tok_cl.count("hello world") > 0);
        assert!(tok_o2.count("hello world") > 0);
    }

    #[test]
    fn tokenizer_selection_o200k_for_claude() {
        let tok = tokenizer_for_model(Some("claude-sonnet-4-20250514"));
        assert!(tok.count("hello") > 0);
    }

    #[test]
    fn tokenizer_selection_o200k_for_unknown_model() {
        // Llama, Mistral, etc. fall back to o200k
        let tok = tokenizer_for_model(Some("meta-llama/Llama-3.1-8B-Instruct"));
        assert!(tok.count("hello") > 0);
    }

    #[test]
    fn cl100k_model_detection() {
        assert!(is_cl100k_model("gpt-4"));
        assert!(is_cl100k_model("gpt-4-turbo"));
        assert!(is_cl100k_model("gpt-3.5-turbo"));
        assert!(!is_cl100k_model("gpt-4o"));
        assert!(!is_cl100k_model("gpt-4o-mini"));
        assert!(!is_cl100k_model("claude-sonnet-4-20250514"));
        assert!(!is_cl100k_model("meta-llama/Llama-3.1-8B"));
    }
}
