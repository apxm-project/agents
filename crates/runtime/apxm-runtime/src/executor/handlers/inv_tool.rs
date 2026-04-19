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

    // Check if this capability is backed by a Python handler.
    let python_handler_id = node
        .attributes
        .get(graph_attrs::PYTHON_HANDLER_ID)
        .and_then(|v| v.as_string())
        .map(|s| s.to_string());

    tracing::debug!(
        capability = %capability_name,
        inputs = inputs.len(),
        timeout_ms = timeout_ms,
        python_handler = ?python_handler_id,
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

    let timeout = std::time::Duration::from_millis(timeout_ms);

    // Dispatch: Python handler branch vs. Rust capability branch.
    let result = if let Some(handler_id) = python_handler_id {
        execute_python_tool(ctx, &capability_name, &handler_id, &args, timeout).await?
    } else {
        ctx.capability_system
            .invoke_with_timeout(&capability_name, args, timeout)
            .await
            .map_err(|e| {
                tracing::error!(
                    capability = %capability_name,
                    error = %e,
                    "Capability invocation failed"
                );
                e
            })?
    };

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

/// Dispatch an INV_TOOL call to the Python tool worker bridge.
///
/// Converts the `HashMap<String, Value>` args to `serde_json::Value`,
/// calls into `PythonToolBridge::call`, and converts the result back.
async fn execute_python_tool(
    ctx: &ExecutionContext,
    capability_name: &str,
    _handler_id: &str,
    args: &HashMap<String, Value>,
    timeout: std::time::Duration,
) -> Result<Value> {
    let bridge = ctx
        .python_tool_bridge
        .as_ref()
        .ok_or_else(|| RuntimeError::Capability {
            capability: capability_name.to_string(),
            message: "INV_TOOL has python_handler_id but no PythonToolBridge is configured"
                .to_string(),
        })?;

    // Convert Value args to serde_json::Value for the wire protocol.
    let json_args =
        serde_json::to_value(args).map_err(|e| RuntimeError::Serialization(e.to_string()))?;

    tracing::debug!(
        capability = %capability_name,
        timeout_ms = timeout.as_millis() as u64,
        "Dispatching to Python tool worker"
    );

    let json_result = bridge.call(capability_name, json_args, timeout).await.map_err(|e| {
        tracing::error!(
            capability = %capability_name,
            error = %e,
            "Python tool invocation failed"
        );
        RuntimeError::Capability {
            capability: capability_name.to_string(),
            message: format!("Python tool failed: {}", e),
        }
    })?;

    // Convert serde_json::Value back to Value.
    json_to_value(json_result)
}

/// Convert a `serde_json::Value` to the runtime `Value` type.
fn json_to_value(v: serde_json::Value) -> Result<Value> {
    match v {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Number(apxm_core::types::values::Number::Integer(i)))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(apxm_core::types::values::Number::Float(f)))
            } else {
                Err(RuntimeError::Serialization(format!(
                    "Unsupported JSON number: {}",
                    n
                )))
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s)),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<Value>> = arr.into_iter().map(json_to_value).collect();
            Ok(Value::Array(items?))
        }
        serde_json::Value::Object(map) => {
            let mut result = HashMap::new();
            for (k, v) in map {
                result.insert(k, json_to_value(v)?);
            }
            Ok(Value::Object(result))
        }
    }
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

    #[tokio::test]
    async fn test_inv_python_handler_no_bridge_errors() {
        let ctx = create_test_context_with_capability().await;
        // python_tool_bridge is None by default

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
            Value::String("add".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PYTHON_HANDLER_ID.to_string(),
            Value::String("sha256:abc123".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("PythonToolBridge"),
            "Expected PythonToolBridge error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_inv_without_python_handler_uses_capability_system() {
        // When python_handler_id is absent, the existing capability path is used.
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
        node.attributes
            .insert("arg_message".to_string(), Value::String("via Rust".to_string()));

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(
            result.as_string().map(|s| s.as_str()),
            Some("Echo: via Rust")
        );
    }

    #[test]
    fn test_json_to_value_primitives() {
        assert_eq!(json_to_value(serde_json::Value::Null).unwrap(), Value::Null);
        assert_eq!(
            json_to_value(serde_json::json!(true)).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            json_to_value(serde_json::json!(42)).unwrap(),
            Value::Number(apxm_core::types::values::Number::Integer(42))
        );
        assert_eq!(
            json_to_value(serde_json::json!(3.14)).unwrap(),
            Value::Number(apxm_core::types::values::Number::Float(3.14))
        );
        assert_eq!(
            json_to_value(serde_json::json!("hello")).unwrap(),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn test_json_to_value_nested() {
        let json = serde_json::json!({"nums": [1, 2, 3], "flag": true});
        let val = json_to_value(json).unwrap();
        if let Value::Object(map) = &val {
            assert!(map.contains_key("nums"));
            assert!(map.contains_key("flag"));
            if let Value::Array(arr) = &map["nums"] {
                assert_eq!(arr.len(), 3);
            } else {
                panic!("Expected Array for 'nums'");
            }
        } else {
            panic!("Expected Object");
        }
    }
}
