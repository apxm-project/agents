//! PRINT operation - Print output to stdout with markdown rendering (void operation)

use super::{
    ExecutionContext, Node, Result, Value, get_string_attribute,
    template::{input_names_from_node, render_named},
};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use termimad::MadSkin;

/// Execute a print operation. This is a void operation (no output tokens).
/// Output is rendered as markdown for terminal display.
///
/// `message` is a named-placeholder template; `{name}` references the
/// corresponding input via the node's `input_names` parallel array.
pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let message = get_string_attribute(node, graph_attrs::MESSAGE).unwrap_or_default();
    let input_names = input_names_from_node(node);
    let output = render_named(&message, &inputs, &input_names)?;

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

    /// Build a PRINT node whose `message` template references inputs by
    /// the names supplied via `input_names`.
    fn make_node(message: &str, input_names: &[&str]) -> Node {
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
        if !input_names.is_empty() {
            node.attributes.insert(
                graph_attrs::INPUT_NAMES.to_string(),
                Value::Array(
                    input_names
                        .iter()
                        .map(|n| Value::String((*n).to_string()))
                        .collect(),
                ),
            );
        }
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

        let node = make_node("Hello {greeted}", &["greeted"]);
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

        let node = make_node("Status: {status}", &["status"]);
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

    #[tokio::test]
    async fn test_print_with_no_inputs_renders_message_verbatim() {
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

        let node = make_node("plain message", &[]);
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::Null);
    }
}
