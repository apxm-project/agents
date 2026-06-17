//! Named-placeholder substitution shared by all template-bearing handlers.
//!
//! Templates use `{name}` placeholders; runtime resolution maps each `name`
//! against the node's `input_names` parallel array (matching incoming Data
//! operands by index) or — for compile parameters — against runtime args
//! (handled at the entry point, not here).
//!
//! Dotted paths such as `{data.event.subject}` resolve the first segment against
//! `input_names` and navigate JSON for the remainder.

use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use apxm_core::utils::template::parse_placeholder_names;
use std::collections::HashMap;

/// Render a template by substituting every `{name}` placeholder with the
/// corresponding entry from `inputs`, where `name` is looked up in
/// `input_names` (the parallel name-by-position array attached to the node).
///
/// Supports dotted paths (`{data.event.subject}`): the first segment must
/// appear in `input_names`; remaining segments navigate JSON object fields.
///
/// Returns `RuntimeError::Executor` when:
/// - `input_names.len() != inputs.len()` (graph contract violation)
/// - the template references a name not present in `input_names`
///   (the validator should have caught this; treated as defense-in-depth)
/// - a dotted path cannot be resolved
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
    if names.is_empty() && !template.contains('{') {
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
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'.')
            {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b'}' {
                let name = &template[start..end];
                let rendered = resolve_placeholder(name, inputs, &index)?;
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

fn resolve_placeholder(
    name: &str,
    inputs: &[Value],
    index: &HashMap<&str, usize>,
) -> Result<String, RuntimeError> {
    let (root, rest) = match name.split_once('.') {
        Some((root, rest)) => (root, Some(rest)),
        None => (name, None),
    };
    let Some(&idx) = index.get(root) else {
        return Err(RuntimeError::Executor(format!(
            "template references unknown placeholder '{{{name}}}': not in input_names"
        )));
    };
    let mut value = inputs[idx].clone();
    if let Some(path) = rest {
        value = navigate_json_value(&value, path).ok_or_else(|| {
            RuntimeError::Executor(format!(
                "template placeholder '{{{name}}}' could not be resolved"
            ))
        })?;
    }
    Ok(value
        .as_string()
        .map(|s| s.to_string())
        .unwrap_or_else(|| value.to_string()))
}

fn navigate_json_value(value: &Value, path: &str) -> Option<Value> {
    let json = value.to_json().ok()?;
    let mut cur = json;
    for seg in path.split('.') {
        cur = match cur {
            serde_json::Value::Object(map) => map.get(seg)?.clone(),
            serde_json::Value::Array(arr) => arr.get(seg.parse::<usize>().ok()?)?.clone(),
            _ => return None,
        };
    }
    json_to_value(cur).ok()
}

fn json_to_value(v: serde_json::Value) -> Result<Value, RuntimeError> {
    use apxm_core::types::values::Number;
    Ok(match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(Number::Integer(i))
            } else if let Some(u) = n.as_u64() {
                Value::Number(Number::Integer(u as i64))
            } else {
                Value::Number(Number::Float(n.as_f64().unwrap_or(0.0)))
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(a) => {
            Value::Array(a.into_iter().map(json_to_value).collect::<Result<_, _>>()?)
        }
        serde_json::Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| json_to_value(v).map(|val| (k, val)))
                .collect::<Result<_, _>>()?,
        ),
    })
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
    use apxm_core::types::values::Value;

    #[test]
    fn dotted_event_field_resolves_from_json_param() {
        let data = json_to_value(serde_json::json!({
            "event": { "subject": "chat-42", "payload": { "text": "hi" } }
        }))
        .unwrap();
        let inputs = vec![data];
        let names = vec!["data".to_string()];
        let out = render_named("{data.event.subject}", &inputs, &names).unwrap();
        assert_eq!(out, "chat-42");
    }

    #[test]
    fn step_output_ref_resolves() {
        let agent_out = Value::String("hello".into());
        let inputs = vec![agent_out];
        let names = vec!["agent".to_string()];
        let out = render_named("{agent}", &inputs, &names).unwrap();
        assert_eq!(out, "hello");
    }

    #[test]
    fn unresolvable_dotted_path_errors() {
        let data = json_to_value(serde_json::json!({ "event": {} })).unwrap();
        let inputs = vec![data];
        let names = vec!["data".to_string()];
        assert!(render_named("{data.event.missing}", &inputs, &names).is_err());
    }
}
