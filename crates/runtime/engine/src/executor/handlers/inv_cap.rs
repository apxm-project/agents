//! INV_CAP operation - Capability invocation with validation and timeout
//!
//! Invokes registered capabilities/tools through the capability system.
//! Provides automatic input validation, timeout enforcement, and error handling.

use std::collections::HashMap;

use super::{
    ExecutionContext, Node, Result, Value, get_optional_u64_attribute, get_string_attribute,
    template::{input_names_from_node, render_named},
};
use crate::capability::CapabilitySandboxPreflight;
use crate::executor::capability_admission::metadata_admits_write;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;
use apxm_core::types::AISOperationType;

/// Invoke-site capability admission — enforced for EVERY tool call regardless of how
/// the execution was launched. A direct (write) capability runs only when this
/// execution's effective capability grants admit its tool binding.
async fn enforce_write_boundary(
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
    if metadata_admits_write(&ctx.metadata, name) {
        if ctx.host_id.is_some() {
            use apxm_core::types::consent::{
                ConsentDecision, PermissionPrompt, PromptMode, RiskLevel,
            };
            let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
            let prompt = PermissionPrompt {
                prompt_id: uuid::Uuid::new_v4().to_string(),
                call_id: uuid::Uuid::new_v4().to_string(),
                grant_id: "runtime-grant".to_string(),
                capability_id: name.to_string(),
                capability_binding: name.to_string(),
                host_id: ctx.host_id.clone(),
                operation: "invoke".to_string(),
                mode: PromptMode::Confirm,
                subject: None,
                args_digest: format!("args-len:{}", args.len()),
                args_preview: serde_json::json!({ "arg_count": args.len() }),
                risk_level: RiskLevel::High,
                expires_at,
                channel_id: None,
                description: Some(format!("Host capability '{}' requires consent", name)),
                target_ref: None,
                resource: None,
                diff_ref: None,
            };
            match ctx
                .consent_broker
                .request_consent(prompt, std::time::Duration::from_secs(30))
                .await
            {
                ConsentDecision::Approved(_) | ConsentDecision::NoBroker => Ok(()),
                ConsentDecision::Denied { reason } => Err(RuntimeError::Capability {
                    capability: name.to_string(),
                    message: reason,
                }),
                ConsentDecision::TimedOut => Err(RuntimeError::Capability {
                    capability: name.to_string(),
                    message: "consent timed out".to_string(),
                }),
            }
        } else {
            Ok(())
        }
    } else {
        Err(RuntimeError::Capability {
            capability: name.to_string(),
            message: format!(
                "capability '{}' performs writes and is missing a capability grant; \
 mint a grant for its tool binding and present grant_* ids in capability_grant_ids",
                name
            ),
        })
    }
}

/// Execute INV_CAP operation - Invoke a registered capability
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
/// INV_CAP(capability="echo", timeout_ms=5000) -> result
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
     "Executing INV_CAP operation"
    );

    // Convert inputs to HashMap<String, Value> strictly from params_json.
    let mut args = HashMap::new();

    // First, check for params_json attribute (from InvCapOp MLIR)
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
    // input_names: ["query"]
    // params_json: {"agent": "mock-agent", "prompt": "{query}"}
    if !inputs.is_empty() {
        let input_names = input_names_from_node(node);
        for val in args.values_mut() {
            render_named_in_value(val, &inputs, &input_names)?;
        }
    }

    if !node.attributes.contains_key(graph_attrs::PARAMS_JSON) && !inputs.is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "INV_CAP inputs require a params_json object with named placeholders"
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

    // pre_cap hooks run for EVERY tool — Python-bridge AND native/builtin
    // capabilities. A deny / gate failure does NOT
    // fail the node: the turn continues gracefully with a denial message (m4,
    // matching the LLM tool-loop's graceful `ToolResult::error`).
    let args =
        match crate::executor::hook_driver::run_pre_cap_hooks(ctx, &capability_name, args).await {
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

    // Invoke-site capability admission for EVERY tool call.
    // call, including artifact-local Python tools that do not exist in the
    // process-wide capability registry.
    enforce_write_boundary(ctx, &capability_name, &args, python_handler_id.is_some()).await?;

    // Script-handler branch: stamped by bind-capability-handlers or resolved by bridge.
    let raw = if python_handler_id.is_some() || script_handler_for_capability(ctx, &capability_name)
    {
        tokio::select! {
        result = execute_script_handler(ctx, &capability_name, &args, timeout) => result?,
        _ = ctx.cancellation_token.cancelled() => return Err(RuntimeError::SchedulerCancelled),
        }
    } else {
        // Write serialization on the GRAPH path: the dataflow scheduler runs
        // independent inv_cap nodes concurrently and serializes only by data
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
         result = ctx.invoke_capability_with_timeout(&capability_name, args, timeout) => {
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
    // post_cap hooks (replace_result) for both paths.
    let result = crate::executor::hook_driver::run_post_cap_hooks(ctx, &capability_name, raw).await;
    // Deterministic tool-result trimming (W2.7): an oversized result (e.g. a
    // full raw web page body) must not silently inflate the conversation's
    // token budget. Reuses the SAME `truncate_to_budget` primitive the
    // subagent prompt-budget mechanism ships — a pure, deterministic
    // function over a real bpe tokenizer, never a chars/4 re-derivation.
    let result = trim_oversized_tool_result(result);
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
            message: format!("INV_CAP params_json is invalid JSON: {err}"),
        }
    })?;
    let obj = parsed.as_object().ok_or_else(|| RuntimeError::Operation {
        op_type,
        message: "INV_CAP params_json must be a JSON object".to_string(),
    })?;
    let mut args = HashMap::new();
    for (k, v) in obj {
        // Pass ALL values through, including nested objects/arrays — e.g. an
        // `http_get` `headers` object (and apxm-auth-injected `Authorization`)
        // must reach the capability.
        let value = Value::try_from(v.clone()).map_err(|err| RuntimeError::Operation {
            op_type,
            message: format!("INV_CAP params_json value for '{k}' is unsupported: {err}"),
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

/// Trim a string-valued tool result down to
/// [`apxm_core::constants::runtime::tool_result_trim::DEFAULT_MAX_TOKENS`]
/// via [`crate::context_stack::truncate_to_budget`] (deterministic
/// prefix-keep + a stable `"... [truncated N tokens]"` marker). Structured
/// (object/array) results pass through untouched — trimming JSON structure
/// mid-value would corrupt it for any caller that parses the result rather
/// than reads it as prose.
fn trim_oversized_tool_result(value: Value) -> Value {
    match value {
        Value::String(text) => {
            let max_tokens =
                apxm_core::constants::runtime::tool_result_trim::DEFAULT_MAX_TOKENS;
            let (trimmed, _was_trimmed) = crate::context_stack::truncate_to_budget(&text, max_tokens);
            Value::String(trimmed)
        }
        other => other,
    }
}

fn script_handler_for_capability(ctx: &ExecutionContext, capability_name: &str) -> bool {
    ctx.python_handler_bridge
        .as_ref()
        .is_some_and(|bridge| bridge.has_tool(capability_name))
        || ctx
            .typescript_handler_bridge
            .as_ref()
            .is_some_and(|bridge| bridge.has_tool(capability_name))
}

/// Dispatch an INV_CAP call to a Python or TypeScript tool worker bridge.
async fn execute_script_handler(
    ctx: &ExecutionContext,
    capability_name: &str,
    args: &HashMap<String, Value>,
    timeout: std::time::Duration,
) -> Result<Value> {
    let json_args =
        serde_json::to_value(args).map_err(|e| RuntimeError::Serialization(e.to_string()))?;

    tracing::debug!(
     capability = %capability_name,
     timeout_ms = timeout.as_millis() as u64,
     "Dispatching to script tool worker"
    );

    if let Some(bridge) = ctx.python_handler_bridge.as_ref() {
        if bridge.has_tool(capability_name) {
            let json_result = bridge.call(capability_name, json_args.clone(), timeout).await.map_err(|e| {
 tracing::error!(capability = %capability_name, error = %e, "Python tool invocation failed");
 RuntimeError::Capability {
 capability: capability_name.to_string(),
 message: format!("Python tool failed: {}", e),
 }
 })?;
            return json_to_value(json_result);
        }
    }

    let bridge =
        ctx.typescript_handler_bridge
            .as_ref()
            .ok_or_else(|| RuntimeError::Capability {
                capability: capability_name.to_string(),
                message: "INV_CAP has script handler id but no handler bridge is configured"
                    .to_string(),
            })?;

    let json_result = bridge
        .call(capability_name, json_args, timeout)
        .await
        .map_err(|e| {
            tracing::error!(
             capability = %capability_name,
             error = %e,
             "TypeScript tool invocation failed"
            );
            RuntimeError::Capability {
                capability: capability_name.to_string(),
                message: format!("TypeScript tool failed: {}", e),
            }
        })?;

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
            AISOperationType::InvCap,
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
        let err = args_from_params_json(AISOperationType::InvCap, r#"{"chat_id":""bad"}"#)
            .expect_err("invalid params_json should fail closed");
        assert!(err.to_string().contains("params_json is invalid JSON"));
    }

    #[test]
    fn args_from_params_json_rejects_non_object() {
        let err = args_from_params_json(AISOperationType::InvCap, r#"["not","object"]"#)
            .expect_err("params_json must be object");
        assert!(
            err.to_string()
                .contains("params_json must be a JSON object")
        );
    }

    /// **Approved protected-path write denial (required evidence, W1.9):** an
    /// operator-set `blocked_paths` entry on `WriteCapability` is a floor a
    /// consent `Approved` decision cannot reach. `enforce_write_boundary`
    /// (the admission gate above) approves this call via the
    /// `StubBroker`/`ConsentDecision::Approved` path — proving the eventual
    /// denial below comes from `WriteCapability::validate_path_and_content`
    /// running downstream, independent of and never overridden by the
    /// approval outcome.
    #[tokio::test]
    async fn approved_write_to_protected_path_is_still_denied() {
        use crate::aam::Aam;
        use crate::capability::CapabilitySystem;
        use crate::capability::builtins::{WriteCapability, WriteConfig};
        use crate::memory::{MemoryConfig, MemorySystem};
        use apxm_backends::LLMRegistry;
        use apxm_core::types::consent::{ConsentBroker, ConsentDecision, PermissionPrompt};
        use std::sync::Arc;
        use std::time::Duration;

        // Reuses the `StubBroker` fixture pattern from
        // `apxm-capability`'s `interceptor.rs` tests: a `ConsentBroker` that
        // always returns a fixed, caller-chosen decision.
        struct StubBroker {
            decision: ConsentDecision,
        }

        #[async_trait::async_trait]
        impl ConsentBroker for StubBroker {
            async fn request_consent(
                &self,
                _prompt: PermissionPrompt,
                _timeout: Duration,
            ) -> ConsentDecision {
                self.decision.clone()
            }
        }

        let base = tempfile::tempdir().expect("write base tempdir");
        std::fs::create_dir_all(base.path().join("capabilities")).expect("mkdir capabilities");
        let protected = base.path().join("capabilities").join("permissions.toml");

        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        capability_system
            .register(Arc::new(WriteCapability::with_config(WriteConfig {
                base_directory: Some(base.path().to_path_buf()),
                blocked_paths: vec![protected.clone()],
                ..Default::default()
            })))
            .expect("register write capability");

        // Runtime-minted, active, mutating grant admitting a direct write to
        // the `write` capability binding — the admission gate
        // (`enforce_write_boundary`) requires this before it even asks the
        // consent broker.
        let grants = serde_json::json!([{
            "grant_id": "grant_fixture",
            "capability_binding": "write",
            "operations": ["write"],
            "expires_at": null,
            "status": "active"
        }])
        .to_string();

        let mut ctx =
            ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam);
        ctx.host_id = Some("test-host".to_string());
        ctx.consent_broker = Arc::new(StubBroker {
            decision: ConsentDecision::Approved(vec![]),
        });
        ctx.metadata.insert(
            crate::metadata_keys::CAPABILITY_GRANTS.to_string(),
            grants,
        );

        let mut node = Node::new(1, AISOperationType::InvCap);
        node.set_attribute(
            graph_attrs::CAPABILITY.to_string(),
            Value::String("write".to_string()),
        );
        node.set_attribute(
            graph_attrs::PARAMS_JSON.to_string(),
            Value::String(
                serde_json::json!({
                    "file_path": "capabilities/permissions.toml",
                    "content": "attacker-authored permissions"
                })
                .to_string(),
            ),
        );

        let err = execute(&ctx, &node, vec![])
            .await
            .expect_err("write to a blocked_paths entry must be denied even when approved");
        assert!(
            err.to_string().contains("blocked by policy"),
            "unexpected error: {err}"
        );
        assert!(
            !protected.exists(),
            "denied write must not have created the protected file"
        );
    }
}
