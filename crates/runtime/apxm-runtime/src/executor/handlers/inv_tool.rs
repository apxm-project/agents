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
use apxm_core::types::{AISOperationType, CapabilityOperation, CapabilityStatus};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct RuntimeDelegatedCapability {
    tool_binding: String,
    operations: Vec<CapabilityOperation>,
    #[serde(default)]
    expires_at: Option<String>,
    status: CapabilityStatus,
}

/// Returns true if the effective runtime-minted delegated capability set admits
/// a Direct write to `cap`. Absence, malformed metadata, expired grants, and
/// non-mutating grants all fail closed.
fn delegated_authority_admits_write(metadata: Option<&str>, cap: &str) -> bool {
    let Some(metadata) = metadata else {
        return false;
    };
    let Ok(capabilities) = serde_json::from_str::<Vec<RuntimeDelegatedCapability>>(metadata) else {
        return false;
    };
    capabilities.into_iter().any(|capability| {
        capability.status == CapabilityStatus::Active
            && capability.tool_binding == cap
            && capability
                .operations
                .iter()
                .copied()
                .any(CapabilityOperation::is_mutating)
            && !delegated_capability_expired(capability.expires_at.as_deref())
    })
}

fn delegated_capability_expired(expires_at: Option<&str>) -> bool {
    let Some(expires_at) = expires_at else {
        return false;
    };
    DateTime::parse_from_rfc3339(expires_at)
        .map(|expires_at| expires_at.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

/// Invoke-site write boundary — enforced for EVERY tool call regardless of how
/// the execution was launched (raw /v1/execute, a CALL_SKILL child DAG, a
/// dispatched graph, SPAWN_AGENT). A Direct (write) capability runs only if this
/// execution's effective runtime-minted delegated capabilities admit it.
/// Read-only and sandboxed capabilities are always allowed. This closes the gap
/// where the write boundary was previously enforced only by the server's static
/// pre-flight at /v1/execute (bypassable by nested executions).
fn enforce_write_boundary(
    ctx: &ExecutionContext,
    name: &str,
    args: &HashMap<String, Value>,
    unregistered_requires_delegation: bool,
) -> Result<()> {
    let caps = &ctx.capability_system;
    if caps.has_capability(name) {
        if caps.is_read_only(name) {
            return Ok(());
        }
        match caps.sandbox_preflight(name, args) {
            Ok(CapabilitySandboxPreflight::Sandboxed { .. }) => return Ok(()),
            Ok(CapabilitySandboxPreflight::Direct) => {}
            // Pre-flight error -> fail-closed: treat as a write needing admission.
            Err(_) => {}
        }
    } else if !unregistered_requires_delegation {
        return Ok(());
    }
    let delegated_capabilities = ctx
        .metadata
        .get(metadata_keys::DELEGATED_CAPABILITIES)
        .map(String::as_str);
    if delegated_authority_admits_write(delegated_capabilities, name) {
        Ok(())
    } else {
        Err(RuntimeError::Capability {
            capability: name.to_string(),
            message: format!(
                "write capability '{name}' is not delegated by this execution; \
                 present a runtime-minted cap_* id in delegated_capability_ids to authorize this execution"
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
        args = args_from_params_json(node.op_type, params_json)?;
    }

    // Substitute `{name}` placeholders in every string-valued arg with the
    // corresponding upstream input, looked up via the node's `input_names`
    // parallel array. Example:
    //   input_names: ["query"]
    //   params_json: {"agent": "mock-agent", "prompt": "{query}"}
    if !inputs.is_empty() {
        let input_names = input_names_from_node(node);
        for val in args.values_mut() {
            render_named_in_value(val, &inputs, &input_names)?;
        }
    }

    if !node.attributes.contains_key(graph_attrs::PARAMS_JSON) && !inputs.is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "INV_TOOL inputs require a params_json object with named placeholders"
                .to_string(),
        });
    }
    super::llm::tool_dispatch::inject_visible_skill_imports(
        &capability_name,
        &mut args,
        &ctx.metadata,
    );

    // Check cancellation before expensive capability invocation
    if ctx.cancellation_token.is_cancelled() {
        return Err(RuntimeError::SchedulerCancelled);
    }

    let timeout = std::time::Duration::from_millis(timeout_ms);

    // pre_tool hooks run for EVERY tool — Python-bridge AND native/builtin
    // capabilities (FR-004, constitution #5). A deny / gate failure does NOT
    // fail the node: the turn continues gracefully with a denial message (m4,
    // matching the LLM tool-loop's graceful `ToolResult::error`).
    let args =
        match crate::executor::hook_driver::run_pre_tool_hooks(ctx, &capability_name, args).await {
            Ok(edited) => edited,
            Err(e) => {
                return Ok(Value::String(format!(
                    "[tool '{capability_name}' blocked: {e}]"
                )));
            }
        };
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_tool_start(&capability_name, &args);
    }

    // Invoke-site write boundary: enforce delegated authority for EVERY tool
    // call, including artifact-local Python tools that do not exist in the
    // process-wide capability registry.
    enforce_write_boundary(ctx, &capability_name, &args, python_handler_id.is_some())?;

    // Python branch is taken iff `bind-tool-handlers` stamped a handler id.
    let raw = if let Some(handler_id) = python_handler_id {
        tokio::select! {
            result = execute_python_tool(ctx, &capability_name, &handler_id, &args, timeout) => result?,
            _ = ctx.cancellation_token.cancelled() => return Err(RuntimeError::SchedulerCancelled),
        }
    } else {
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
        let outcome = tokio::select! {
            result = ctx.invoke_tool_with_timeout(&capability_name, args, timeout) => {
                result.map_err(|e| {
                    tracing::error!(
                        capability = %capability_name,
                        error = %e,
                        "Capability invocation failed"
                    );
                    e
                })?
            }
            _ = ctx.cancellation_token.cancelled() => return Err(RuntimeError::SchedulerCancelled),
        };
        if let Some((lock, guard)) = write_guard {
            drop(guard);
            crate::capability::tool_write_lock::release_write_lock_if_idle(&capability_name, &lock);
        }
        outcome
    };
    // post_tool hooks (replace_result) for both paths.
    let result =
        crate::executor::hook_driver::run_post_tool_hooks(ctx, &capability_name, raw).await;
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_tool_end(&capability_name, &result);
    }

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

fn args_from_params_json(
    op_type: AISOperationType,
    params_json: &str,
) -> Result<HashMap<String, Value>> {
    let parsed = serde_json::from_str::<serde_json::Value>(params_json).map_err(|err| {
        RuntimeError::Operation {
            op_type,
            message: format!("INV_TOOL params_json is invalid JSON: {err}"),
        }
    })?;
    let obj = parsed.as_object().ok_or_else(|| RuntimeError::Operation {
        op_type,
        message: "INV_TOOL params_json must be a JSON object".to_string(),
    })?;
    let mut args = HashMap::new();
    for (k, v) in obj {
        // Pass ALL values through, including nested objects/arrays — e.g. an
        // `http_get` `headers` object (and apxm-auth-injected `Authorization`)
        // must reach the capability.
        let value = Value::try_from(v.clone()).map_err(|err| RuntimeError::Operation {
            op_type,
            message: format!("INV_TOOL params_json value for '{k}' is unsupported: {err}"),
        })?;
        args.insert(k.clone(), value);
    }
    Ok(args)
}

fn render_named_in_value(
    value: &mut Value,
    inputs: &[Value],
    input_names: &[String],
) -> Result<()> {
    match value {
        Value::String(s) => {
            *s = render_named(s, inputs, input_names)?;
        }
        Value::Array(items) => {
            for item in items {
                render_named_in_value(item, inputs, input_names)?;
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                render_named_in_value(item, inputs, input_names)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Token(_) => {}
    }
    Ok(())
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

    #[test]
    fn args_from_params_json_preserves_structured_values() {
        let args = args_from_params_json(
            AISOperationType::InvTool,
            r#"{"chat_id":"chat-42","headers":{"x":"y"},"items":[1,true]}"#,
        )
        .unwrap();

        assert_eq!(
            args.get("chat_id").and_then(Value::as_string),
            Some(&"chat-42".to_string())
        );
        assert!(matches!(args.get("headers"), Some(Value::Object(_))));
        assert!(matches!(args.get("items"), Some(Value::Array(_))));
    }

    #[test]
    fn render_named_in_value_resolves_nested_params_json_placeholders() {
        let mut value = Value::try_from(serde_json::json!({
            "headers": { "x-chat": "{message.chat.id}" },
            "items": ["literal", "{reply}"]
        }))
        .unwrap();
        let inputs = vec![
            Value::try_from(serde_json::json!({"chat": {"id": "chat-42"}})).unwrap(),
            Value::String("hello".to_string()),
        ];
        let input_names = vec!["message".to_string(), "reply".to_string()];

        render_named_in_value(&mut value, &inputs, &input_names).unwrap();

        assert_eq!(
            value,
            Value::try_from(serde_json::json!({
                "headers": { "x-chat": "chat-42" },
                "items": ["literal", "hello"]
            }))
            .unwrap()
        );
    }

    #[test]
    fn args_from_params_json_rejects_invalid_json() {
        let err = args_from_params_json(AISOperationType::InvTool, r#"{"chat_id":""bad"}"#)
            .expect_err("invalid params_json should fail closed");
        assert!(err.to_string().contains("params_json is invalid JSON"));
    }

    #[test]
    fn args_from_params_json_rejects_non_object() {
        let err = args_from_params_json(AISOperationType::InvTool, r#"["not","object"]"#)
            .expect_err("params_json must be object");
        assert!(
            err.to_string()
                .contains("params_json must be a JSON object")
        );
    }

    #[test]
    fn delegated_authority_requires_runtime_minted_tool_binding_grant() {
        let metadata = serde_json::json!([{
            "capability_id": "cap_fixture",
            "tool_binding": "fixture.write",
            "operations": ["write"],
            "expires_at": null,
            "status": "active"
        }])
        .to_string();

        assert!(delegated_authority_admits_write(
            Some(&metadata),
            "fixture.write"
        ));
        assert!(!delegated_authority_admits_write(
            Some(&metadata),
            "other.write"
        ));
        assert!(!delegated_authority_admits_write(
            Some("not delegated capability metadata"),
            "fixture.write"
        ));
    }

    #[test]
    fn delegated_authority_rejects_expired_or_non_mutating_grants() {
        let expired = serde_json::json!([{
            "capability_id": "cap_fixture",
            "tool_binding": "fixture.write",
            "operations": ["write"],
            "expires_at": "2000-01-01T00:00:00Z",
            "status": "active"
        }])
        .to_string();
        let read_only = serde_json::json!([{
            "capability_id": "cap_fixture",
            "tool_binding": "fixture.write",
            "operations": ["read"],
            "expires_at": null,
            "status": "active"
        }])
        .to_string();

        assert!(!delegated_authority_admits_write(
            Some(&expired),
            "fixture.write"
        ));
        assert!(!delegated_authority_admits_write(
            Some(&read_only),
            "fixture.write"
        ));
    }
}
