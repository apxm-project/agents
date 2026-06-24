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
            Value::Number(Number::Integer(i64::try_from(count).unwrap_or(i64::MAX))),
        );
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

/// Refine shared-prefix token estimates using exact BPE tokenization.
///
/// After MLIR compilation, this post-pass replaces shared-prefix estimates
/// with BPE token counts, selecting the tokenizer based on each node's `model`
/// attribute.
pub fn refine_token_estimates(dags: &mut [ExecutionDag]) {
    for dag in dags.iter_mut() {
        for node in &mut dag.nodes {
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
                Value::Number(apxm_core::types::Number::Integer(i64::try_from(count).unwrap_or(i64::MAX))),
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
