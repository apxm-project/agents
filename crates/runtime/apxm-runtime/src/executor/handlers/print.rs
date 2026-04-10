//! PRINT operation - Print output to stdout with markdown rendering (void operation)

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use termimad::MadSkin;

/// Format a Value as a human-readable string (recursive for arrays).
fn format_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        Value::Array(arr) => arr.iter().map(format_value).collect::<Vec<_>>().join(""),
        Value::Object(obj) => serde_json::to_string(obj).unwrap_or_else(|_| format!("{:?}", obj)),
        Value::Token(id) => format!("<token:{}>", id),
    }
}

/// Resolve template variables like {{node_N}} in a message string.
/// Replaces {{node_N}} with the value from the input that came from node N.
/// If the specific node source cannot be determined, replaces with all inputs concatenated.
fn resolve_template_vars(message: &str, inputs: &[Value]) -> String {
    // Simple regex replacement: {{node_\d+}} -> concatenated inputs
    // TODO: This is a minimal fix. Full implementation would track token sources
    // to map each {{node_N}} to the specific input from node N.

    let re = regex::Regex::new(r"\{\{node_\d+\}\}").unwrap();
    let mut result = message.to_string();

    // Replace all {{node_*}} placeholders with concatenated input values
    if re.is_match(message) && !inputs.is_empty() {
        let replacement = inputs.iter().map(format_value).collect::<Vec<_>>().join("");
        result = re.replace_all(message, replacement.as_str()).to_string();
    }

    result
}

/// Execute a print operation. This is a void operation (no output tokens).
/// Output is rendered as markdown for terminal display.
pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let message = get_string_attribute(node, graph_attrs::MESSAGE).unwrap_or_default();

    // Resolve template variables in the message
    let resolved_message = resolve_template_vars(&message, &inputs);

    // Build output: resolved message followed by any remaining input values not already included
    let output = if message.contains("{{") {
        // If we resolved templates, the inputs are already included in the message
        resolved_message
    } else {
        // Original behavior: message followed by inputs
        let mut output = resolved_message.clone();
        for (i, input) in inputs.iter().enumerate() {
            if i == 0 && !resolved_message.is_empty() {
                output.push(' ');
            }
            output.push_str(&format_value(input));
            if i < inputs.len() - 1 {
                output.push(' ');
            }
        }
        output
    };

    // Render markdown to terminal
    let skin = MadSkin::default();
    skin.print_text(&output);

    // Record print in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::PRINT_PREFIX,
            ctx.execution_id,
            node.id
        ),
        Value::String(output.chars().take(200).collect::<String>()),
        label,
    );

    // Void operation - return Null (no output tokens in artifact)
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    fn make_node(message: &str) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::Print,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::MESSAGE.to_string(),
            Value::String(message.to_string()),
        );
        node
    }

    #[tokio::test]
    async fn test_print_returns_null() {
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

        let node = make_node("Hello");
        let result = execute(&ctx, &node, vec![Value::String("World".to_string())])
            .await
            .unwrap();
        // Print is a void operation, always returns Null
        assert_eq!(result, Value::Null);
    }

    #[tokio::test]
    async fn test_print_records_aam_belief() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_node("Status:");
        let _result = execute(&ctx, &node, vec![Value::String("OK".to_string())])
            .await
            .unwrap();

        let beliefs = aam.beliefs();
        let key = format!(
            "{}{}:{}",
            belief_keys::PRINT_PREFIX,
            ctx.execution_id,
            node.id
        );
        assert!(beliefs.contains_key(&key));
    }

    #[test]
    fn test_format_value_string() {
        assert_eq!(format_value(&Value::String("hello".into())), "hello");
    }

    #[test]
    fn test_format_value_null() {
        assert_eq!(format_value(&Value::Null), "null");
    }

    #[test]
    fn test_format_value_bool() {
        assert_eq!(format_value(&Value::Bool(true)), "true");
    }

    #[test]
    fn test_format_value_array() {
        let arr = Value::Array(vec![Value::String("a".into()), Value::String("b".into())]);
        assert_eq!(format_value(&arr), "ab");
    }
}
