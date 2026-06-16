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

    // Python branch is taken iff `bind-tool-handlers` stamped a handler id.
    let raw = if let Some(handler_id) = python_handler_id {
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
    // post_tool hooks (replace_result) for both paths.
    let result =
        crate::executor::hook_driver::run_post_tool_hooks(ctx, &capability_name, raw).await;

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
