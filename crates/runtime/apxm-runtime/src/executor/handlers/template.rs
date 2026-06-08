//! Named-placeholder substitution shared by all template-bearing handlers.
//!
//! Templates use `{name}` placeholders; runtime resolution maps each `name`
//! against the node's `input_names` parallel array (matching incoming Data
//! operands by index) or — for compile parameters — against runtime args
//! (handled at the entry point, not here).

use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use apxm_core::utils::template::parse_placeholder_names;
use std::collections::HashMap;

/// Render a template by substituting every `{name}` placeholder with the
/// corresponding entry from `inputs`, where `name` is looked up in
/// `input_names` (the parallel name-by-position array attached to the node).
///
/// Returns `RuntimeError::Operation` (with a generic op_type marker) when:
/// - `input_names.len() != inputs.len()` (graph contract violation)
/// - the template references a name not present in `input_names`
///   (the validator should have caught this; treated as defense-in-depth)
pub fn render_named(
    template: &str,
    inputs: &[Value],
    input_names: &[String],
) -> Result<String, RuntimeError> {
    if input_names.len() != inputs.len() {
        return Err(RuntimeError::Executor(format!(
            "input_names length ({}) does not match inputs length ({})",
            input_names.len(),
            inputs.len()
        )));
    }

    // Build name → index map once.
    let mut index: HashMap<&str, usize> = HashMap::with_capacity(input_names.len());
    for (i, name) in input_names.iter().enumerate() {
        index.insert(name.as_str(), i);
    }

    let names = parse_placeholder_names(template);
    if names.is_empty() && !template.contains("{{") && !template.contains("}}") {
        return Ok(template.to_string());
    }

    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            out.push('{');
            i += 2;
            continue;
        }
        if bytes[i] == b'}' && i + 1 < bytes.len() && bytes[i + 1] == b'}' {
            out.push('}');
            i += 2;
            continue;
        }
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b'}' {
                let name = &template[start..end];
                let Some(&idx) = index.get(name) else {
                    return Err(RuntimeError::Executor(format!(
                        "template references unknown placeholder '{{{}}}': not in input_names",
                        name
                    )));
                };
                let value = &inputs[idx];
                let rendered = value
                    .as_string()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| value.to_string());
                out.push_str(&rendered);
                i = end + 1;
                continue;
            }
        }
        // Pass through any byte that is not part of a recognized placeholder.
        out.push(bytes[i] as char);
        i += 1;
    }
    Ok(out)
}

/// Extract the `input_names` parallel string array from a node's attribute map.
///
/// Returns an empty vec if the attribute is missing (nodes with zero data
/// inputs do not need the attribute).
pub fn input_names_from_node(node: &apxm_core::types::execution::Node) -> Vec<String> {
    use apxm_core::constants::graph::attrs as graph_attrs;

    node.attributes
        .get(graph_attrs::INPUT_NAMES)
        .and_then(|v| match v {
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|item| item.as_string().map(|s| s.to_string()))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_substitutes_named_placeholders() {
        let names = vec!["topic".to_string(), "audience".to_string()];
        let inputs = vec![
            Value::String("rust".to_string()),
            Value::String("kids".to_string()),
        ];
        let out = render_named("Explain {topic} to {audience}", &inputs, &names).unwrap();
        assert_eq!(out, "Explain rust to kids");
    }

    #[test]
    fn no_placeholders_returns_template_verbatim() {
        let out = render_named("nothing to see", &[], &[]).unwrap();
        assert_eq!(out, "nothing to see");
    }

    #[test]
    fn length_mismatch_errors() {
        let names = vec!["a".to_string()];
        let inputs: Vec<Value> = vec![];
        let err = render_named("{a}", &inputs, &names).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("input_names length"), "got: {msg}");
    }

    #[test]
    fn unknown_name_errors() {
        let names = vec!["a".to_string()];
        let inputs = vec![Value::String("x".to_string())];
        let err = render_named("{b}", &inputs, &names).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown placeholder"), "got: {msg}");
    }

    #[test]
    fn duplicate_occurrences_substitute_each_time() {
        let names = vec!["x".to_string()];
        let inputs = vec![Value::String("foo".to_string())];
        let out = render_named("{x}-{x}-{x}", &inputs, &names).unwrap();
        assert_eq!(out, "foo-foo-foo");
    }

    #[test]
    fn malformed_braces_pass_through() {
        // `{` not followed by a name+`}` is preserved.
        let out = render_named("a {} b { c", &[], &[]).unwrap();
        assert_eq!(out, "a {} b { c");
    }

    #[test]
    fn escaped_double_braces_render_as_literal_braces() {
        let names = vec!["topic".to_string()];
        let inputs = vec![Value::String("rust".to_string())];
        let out = render_named("literal {{topic}} and {topic}", &inputs, &names).unwrap();
        assert_eq!(out, "literal {topic} and rust");
    }

    #[test]
    fn escaped_double_braces_render_without_placeholders() {
        let out = render_named("literal {{topic}}", &[], &[]).unwrap();
        assert_eq!(out, "literal {topic}");
    }

    #[test]
    fn non_string_value_renders_via_to_string() {
        use apxm_core::types::values::Number;
        let names = vec!["n".to_string()];
        let inputs = vec![Value::Number(Number::Integer(42))];
        let out = render_named("count={n}", &inputs, &names).unwrap();
        assert_eq!(out, "count=42");
    }
}
