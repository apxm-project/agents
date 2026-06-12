//! INV_TOOL operation - Capability invocation with validation and timeout
//!
//! Invokes registered capabilities/tools through the capability system.
//! Provides automatic input validation, timeout enforcement, and error handling.

use super::{
    ExecutionContext, Node, Result, Value, get_optional_u64_attribute, get_string_attribute,
    template::{input_names_from_node, render_named},
};
use crate::capability::CapabilitySandboxPreflight;
use crate::metadata_keys;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

/// Returns true if the effective grant policy (wire form of
/// `apxm_skill::CapabilityPolicy`, e.g. `broader[a,b]`) admits a write to
/// `cap`. `read_only`/`sandboxed`/absent never admit a Direct write.
fn grant_admits_write(policy: Option<&str>, cap: &str) -> bool {
    match policy {
        Some(p) if p.starts_with("broader[") => p
            .strip_prefix("broader[")
            .and_then(|s| s.strip_suffix(']'))
            .map(|inner| inner.split(',').map(str::trim).any(|t| t == cap))
            .unwrap_or(false),
        _ => false,
    }
}

/// Invoke-site write boundary — enforced for EVERY tool call regardless of how
/// the execution was launched (raw /v1/execute, a CALL_SKILL child DAG, a
/// dispatched graph, SPAWN_AGENT). A Direct (write) capability runs only if this
/// execution's effective grant (`SIDE_EFFECT_POLICY`, seeded from
/// admit_capabilities at the top level and propagated to children with no-widen)
/// admits it. Read-only and sandboxed capabilities are always allowed. This
/// closes the gap where the write boundary was previously enforced only by the
/// server's static pre-flight at /v1/execute (bypassable by nested executions).
fn enforce_write_boundary(
    ctx: &ExecutionContext,
    name: &str,
    args: &HashMap<String, Value>,
) -> Result<()> {
    let caps = &ctx.capability_system;
    if !caps.has_capability(name) || caps.is_read_only(name) {
        return Ok(());
    }
    match caps.sandbox_preflight(name, args) {
        Ok(CapabilitySandboxPreflight::Sandboxed { .. }) => return Ok(()),
        Ok(CapabilitySandboxPreflight::Direct) => {}
        // Pre-flight error -> fail-closed: treat as a write needing admission.
        Err(_) => {}
    }
    let policy = ctx
        .metadata
        .get(metadata_keys::SIDE_EFFECT_POLICY)
        .map(String::as_str);
    if grant_admits_write(policy, name) {
        Ok(())
    } else {
        Err(RuntimeError::Capability {
            capability: name.to_string(),
            message: format!(
                "write capability '{name}' is not admitted by this execution's grant; \
                 add it to admit_capabilities to authorize this execution"
            ),
        })
    }
}

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

    // Convert inputs to HashMap<String, Value> strictly from params_json.
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
                // Pass ALL values through, including nested objects/arrays — e.g.
                // an `http_get` `headers` object (and apxm-auth-injected
                // `Authorization`) must reach the capability. Previously nested
                // values were dropped, so structured args silently vanished.
                let Ok(value) = Value::try_from(v.clone()) else {
                    continue;
                };
                args.insert(k.clone(), value);
            }
        }
    }

    // Substitute `{name}` placeholders in every string-valued arg with the
    // corresponding upstream input, looked up via the node's `input_names`
    // parallel array. Example:
    //   input_names: ["query"]
    //   params_json: {"agent": "mock-agent", "prompt": "{query}"}
    if !inputs.is_empty() {
        let input_names = input_names_from_node(node);
        for val in args.values_mut() {
            if let Value::String(s) = val {
                *s = render_named(s, &inputs, &input_names)?;
            }
        }
    }

    if !node.attributes.contains_key(graph_attrs::PARAMS_JSON) && !inputs.is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "INV_TOOL inputs require a params_json object with named placeholders"
                .to_string(),
        });
    }

    // Check cancellation before expensive capability invocation
    if ctx.cancellation_token.is_cancelled() {
        return Err(RuntimeError::SchedulerCancelled);
    }

    let timeout = std::time::Duration::from_millis(timeout_ms);

    // Python branch is taken iff `bind-tool-handlers` stamped a handler id.
    let result = if let Some(handler_id) = python_handler_id {
        execute_python_tool(ctx, &capability_name, &handler_id, &args, timeout).await?
    } else {
        // Invoke-site write boundary: enforce the no-widen grant for EVERY tool
        // call, closing the bypass where nested executions skipped the server's
        // static pre-flight.
        enforce_write_boundary(ctx, &capability_name, &args)?;
        // Write serialization on the GRAPH path: the dataflow scheduler runs
        // independent inv_tool nodes concurrently and serializes only by data
        // dependency, never by tool identity — so a graph with two same-name
        // write nodes could race. Acquire the same per-name write lock the ASK
        // loop uses (shared map) so same-capability writes serialize across both
        // engines; read-only/sandboxed capabilities never lock.
        let write_guard = if ctx.capability_system.has_capability(&capability_name)
            && !ctx.capability_system.is_read_only(&capability_name)
        {
            let lock = crate::capability::tool_write_lock::write_lock_for_tool(&capability_name);
            let guard = lock.clone().write_owned().await;
            Some((lock, guard))
        } else {
            None
        };
        let outcome = ctx
            .invoke_tool_with_timeout(&capability_name, args, timeout)
            .await
            .map_err(|e| {
                tracing::error!(
                    capability = %capability_name,
                    error = %e,
                    "Capability invocation failed"
                );
                e
            })?;
        if let Some((lock, guard)) = write_guard {
            drop(guard);
            crate::capability::tool_write_lock::release_write_lock_if_idle(&capability_name, &lock);
        }
        outcome
    };

    tracing::info!(
        capability = %capability_name,
        "Capability invocation successful"
    );

    // Record capability invocation in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
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

    let json_result = bridge
        .call(capability_name, json_args, timeout)
        .await
        .map_err(|e| {
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

    #[test]
    fn grant_admits_write_only_for_listed_broader_caps() {
        // read_only / sandboxed / absent never admit a Direct write.
        assert!(!grant_admits_write(None, "fs.write"));
        assert!(!grant_admits_write(Some("read_only"), "fs.write"));
        assert!(!grant_admits_write(Some("sandboxed"), "fs.write"));
        // broader[...] admits only the listed capabilities.
        assert!(grant_admits_write(
            Some("broader[fs.write,provider.write]"),
            "fs.write"
        ));
        assert!(grant_admits_write(
            Some("broader[fs.write, provider.write]"),
            "provider.write"
        ));
        assert!(!grant_admits_write(
            Some("broader[provider.write]"),
            "fs.write"
        ));
        assert!(!grant_admits_write(Some("broader[]"), "fs.write"));
    }
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
    async fn inv_tool_inside_agent_scope_emits_tool_call_begin_and_end_with_call_id_pairing() {
        use crate::executor::OperationDispatcher;
        use crate::executor::agent_scope::AgentScope;
        use crate::executor::events::ExecutionEventEmitter;
        use std::sync::Mutex as StdMutex;

        #[derive(Default)]
        struct Recorder {
            tool_begin: StdMutex<Vec<(String, String, Vec<String>)>>,
            tool_end: StdMutex<Vec<(String, String, Vec<String>, String)>>,
        }
        impl ExecutionEventEmitter for Recorder {
            fn emit_llm_token(&self, _content: &str) {}
            fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}
            fn emit_tool_end(&self, _name: &str, _result: &Value) {}
            fn emit_tool_call_begin(
                &self,
                agent_code: &str,
                tool_name: &str,
                argument_keys: &[String],
            ) {
                self.tool_begin.lock().unwrap().push((
                    agent_code.to_string(),
                    tool_name.to_string(),
                    argument_keys.to_vec(),
                ));
            }
            fn emit_tool_call_end(
                &self,
                agent_code: &str,
                tool_name: &str,
                result_keys: &[String],
                status: &str,
                _latency_ms: u64,
            ) {
                self.tool_end.lock().unwrap().push((
                    agent_code.to_string(),
                    tool_name.to_string(),
                    result_keys.to_vec(),
                    status.to_string(),
                ));
            }
        }

        let mut ctx = create_test_context_with_capability().await;
        let recorder: Arc<Recorder> = Arc::new(Recorder::default());
        let emitter: Arc<dyn ExecutionEventEmitter> = recorder.clone();
        ctx.event_emitter = Some(emitter);
        ctx.agent_scope_stack
            .push(AgentScope::new("crm", "span-crm", None, None));

        let mut node = Node {
            id: 7,
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
            graph_attrs::PARAMS_JSON.to_string(),
            Value::String(r#"{"message":"hi"}"#.to_string()),
        );

        let _ = OperationDispatcher::dispatch(&ctx, &node, vec![])
            .await
            .unwrap();

        let begins = recorder.tool_begin.lock().unwrap();
        let ends = recorder.tool_end.lock().unwrap();
        assert_eq!(begins.len(), 1);
        assert_eq!(begins[0].0, "crm");
        assert_eq!(begins[0].1, "echo");
        assert_eq!(begins[0].2, vec!["message".to_string()]);
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].0, "crm");
        assert_eq!(ends[0].1, "echo");
        assert_eq!(ends[0].3, "ok");
    }

    #[tokio::test]
    async fn inv_tool_at_top_level_does_not_emit_tool_call() {
        use crate::executor::OperationDispatcher;
        use crate::executor::events::ExecutionEventEmitter;
        use std::sync::Mutex as StdMutex;

        #[derive(Default)]
        struct Recorder {
            tool_begin: StdMutex<usize>,
            tool_end: StdMutex<usize>,
        }
        impl ExecutionEventEmitter for Recorder {
            fn emit_llm_token(&self, _content: &str) {}
            fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}
            fn emit_tool_end(&self, _name: &str, _result: &Value) {}
            fn emit_tool_call_begin(
                &self,
                _agent_code: &str,
                _tool_name: &str,
                _argument_keys: &[String],
            ) {
                *self.tool_begin.lock().unwrap() += 1;
            }
            fn emit_tool_call_end(
                &self,
                _agent_code: &str,
                _tool_name: &str,
                _result_keys: &[String],
                _status: &str,
                _latency_ms: u64,
            ) {
                *self.tool_end.lock().unwrap() += 1;
            }
        }

        let mut ctx = create_test_context_with_capability().await;
        let recorder: Arc<Recorder> = Arc::new(Recorder::default());
        let emitter: Arc<dyn ExecutionEventEmitter> = recorder.clone();
        ctx.event_emitter = Some(emitter);
        assert!(ctx.agent_scope_stack.is_empty());

        let mut node = Node {
            id: 7,
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
            graph_attrs::PARAMS_JSON.to_string(),
            Value::String(r#"{"message":"hi"}"#.to_string()),
        );

        let _ = OperationDispatcher::dispatch(&ctx, &node, vec![])
            .await
            .unwrap();

        assert_eq!(*recorder.tool_begin.lock().unwrap(), 0);
        assert_eq!(*recorder.tool_end.lock().unwrap(), 0);
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
            graph_attrs::PARAMS_JSON.to_string(),
            Value::String(r#"{"message":"Hello World"}"#.to_string()),
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
        node.attributes.insert(
            graph_attrs::PARAMS_JSON.to_string(),
            Value::String(r#"{"message":"Test"}"#.to_string()),
        );

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
        node.attributes.insert(
            graph_attrs::PARAMS_JSON.to_string(),
            Value::String(r#"{"message":"via Rust"}"#.to_string()),
        );

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
