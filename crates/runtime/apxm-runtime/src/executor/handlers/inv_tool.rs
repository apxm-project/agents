//! INV_TOOL operation - Capability invocation with validation and timeout
//!
//! Invokes registered capabilities/tools through the capability system.
//! Provides automatic input validation, timeout enforcement, and error handling.

use super::{
    ExecutionContext, Node, Result, Value, get_optional_u64_attribute, get_string_attribute,
    template::{input_names_from_node, render_named},
};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

/// Execute INV_TOOL operation - Invoke a registered capability
///
/// # Attributes
///
/// - `capability` (required): Name of the capability to invoke
/// - `timeout_ms` (optional): Custom timeout in milliseconds (default: 30000)
///
/// # Inputs
///
/// Input values are passed as capability arguments. The number and types
/// of inputs depend on the capability's schema.
///
/// # Returns
///
/// Result value from capability execution
///
/// # Errors
///
/// Returns error if:
/// - Capability not found
/// - Input validation fails against capability schema
/// - Execution times out
/// - Capability execution fails
///
/// # Example
///
/// ```text
/// INV_TOOL(capability="echo", timeout_ms=5000) -> result
/// ```
pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let capability_name = get_string_attribute(node, graph_attrs::CAPABILITY)?;
    let timeout_ms = get_optional_u64_attribute(node, graph_attrs::TIMEOUT_MS)?
        .unwrap_or(apxm_core::constants::defaults::DEFAULT_TIMEOUT_MS);

    tracing::debug!(
        capability = %capability_name,
        inputs = inputs.len(),
        timeout_ms = timeout_ms,
        "Executing INV_TOOL operation"
    );

    // Convert inputs to HashMap<String, Value>
    // Priority: 1) params_json attribute, 2) arg_* attributes, 3) positional inputs
    let mut args = HashMap::new();

    // First, check for params_json attribute (from InvToolOp MLIR)
    if let Some(params_json) = node
        .attributes
        .get(graph_attrs::PARAMS_JSON)
        .and_then(|v| v.as_string())
    {
        // Parse JSON and extract key-value pairs
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(params_json)
            && let Some(obj) = parsed.as_object()
        {
            for (k, v) in obj {
                let value = match v {
                    serde_json::Value::String(s) => Value::String(s.clone()),
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Value::Number(apxm_core::types::values::Number::Integer(i))
                        } else if let Some(f) = n.as_f64() {
                            Value::Number(apxm_core::types::values::Number::Float(f))
                        } else {
                            continue;
                        }
                    }
                    serde_json::Value::Bool(b) => Value::Bool(*b),
                    _ => continue, // Skip complex nested values
                };
                args.insert(k.clone(), value);
            }
        }
    }

    // Substitute `{name}` placeholders in every string-valued arg with the
    // corresponding upstream input, looked up via the node's `input_names`
    // parallel array. Example:
    //   input_names: ["query"]
    //   params_json: {"agent": "claude", "prompt": "{query}"}
    if !inputs.is_empty() {
        let input_names = input_names_from_node(node);
        for val in args.values_mut() {
            if let Value::String(s) = val {
                *s = render_named(s, &inputs, &input_names)?;
            }
        }
    }

    // If no args from params_json, check for arg_* attributes (named arguments)
    if args.is_empty() {
        let args_from_attrs = node
            .attributes
            .iter()
            .filter(|(k, _)| k.starts_with("arg_"))
            .map(|(k, v)| (k.trim_start_matches("arg_").to_string(), v.clone()))
            .collect::<HashMap<String, Value>>();

        if !args_from_attrs.is_empty() {
            args = args_from_attrs;
        } else {
            // Fall back to positional arguments from inputs
            for (i, input_value) in inputs.iter().enumerate() {
                args.insert(format!("arg{}", i), input_value.clone());
            }
        }
    }

    // Check cancellation before expensive capability invocation
    if ctx.cancellation_token.is_cancelled() {
        return Err(RuntimeError::SchedulerCancelled);
    }

    // Invoke capability with timeout
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let result = ctx
        .capability_system
        .invoke_with_timeout(&capability_name, args, timeout)
        .await
        .map_err(|e| {
            tracing::error!(
                capability = %capability_name,
                error = %e,
                "Capability invocation failed"
            );
            e
        })?;

    tracing::info!(
        capability = %capability_name,
        "Capability invocation successful"
    );

    // Record capability invocation in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!("{}{}:{}", belief_keys::INV_PREFIX, capability_name, node.id),
        Value::String(format!("invoked:{}", capability_name)),
        label,
    );

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        capability::{CapabilitySystem, executor::EchoCapability},
        memory::{MemoryConfig, MemorySystem},
    };
    use apxm_backends::LLMRegistry;
    use apxm_core::types::{execution::NodeMetadata, operations::AISOperationType};
    use std::sync::Arc;

    async fn create_test_context_with_capability() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        // Register echo capability
        capability_system
            .register(Arc::new(EchoCapability::new()))
            .unwrap();

        ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
    }

    #[tokio::test]
    async fn test_inv_with_named_args() {
        let ctx = create_test_context_with_capability().await;

        let mut node = Node {
            id: 1,
            op_type: AISOperationType::InvTool,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };

        node.attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            Value::String("echo".to_string()),
        );
        node.attributes.insert(
            "arg_message".to_string(),
            Value::String("Hello World".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(
            result.as_string().map(|s| s.as_str()),
            Some("Echo: Hello World")
        );
    }

    #[tokio::test]
    async fn test_inv_capability_not_found() {
        let ctx = create_test_context_with_capability().await;

        let mut node = Node {
            id: 1,
            op_type: AISOperationType::InvTool,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };

        node.attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            Value::String("nonexistent".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_inv_with_custom_timeout() {
        let ctx = create_test_context_with_capability().await;

        let mut node = Node {
            id: 1,
            op_type: AISOperationType::InvTool,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };

        node.attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            Value::String("echo".to_string()),
        );
        node.attributes.insert(
            "timeout_ms".to_string(),
            Value::Number(apxm_core::types::values::Number::Integer(5000)),
        );
        node.attributes
            .insert("arg_message".to_string(), Value::String("Test".to_string()));

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result.as_string().map(|s| s.as_str()), Some("Echo: Test"));
    }
}
