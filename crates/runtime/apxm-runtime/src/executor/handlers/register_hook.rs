//! REGISTER_HOOK operation — register an author lifecycle hook.
//!
//! Mirrors REGISTER_CAPABILITY: a belief-style registration node whose binding
//! (event, match glob, mode, python handler id) is recorded into the
//! per-artifact [`crate::executor::hooks::HookRegistry`] on `ExecutionContext`.
//! The handler is dispatched later via the SAME python tool bridge as `@tool`
//! (constitution #4). The binding travels inside the artifact (AIR-portable,
//! constitution #3); registering here (in the entry flow) makes it visible to
//! later turn/tool nodes through the shared registry.
//!
//! ## Attributes
//! - `hook_event`               (required): lifecycle event
//! - `hook_match`               (optional): glob over tool/op name (default `*`)
//! - `hook_mode`                (optional): `observe` | `gate`
//! - `python_hook_handler_id`   (required): sha256 handler id

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use crate::aam::TransitionLabel;
use crate::executor::hooks::{HookBinding, HookEvent, HookMode};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let event_str = get_string_attribute(node, graph_attrs::HOOK_EVENT)?;
    let handler_id = get_string_attribute(node, graph_attrs::PYTHON_HOOK_HANDLER_ID)?;
    let match_glob = node
        .attributes
        .get(graph_attrs::HOOK_MATCH)
        .and_then(|v| v.as_string())
        .cloned()
        .unwrap_or_else(|| "*".to_string());
    let mode_str = node
        .attributes
        .get(graph_attrs::HOOK_MODE)
        .and_then(|v| v.as_string())
        .cloned()
        .unwrap_or_else(|| "observe".to_string());

    let event = HookEvent::parse(&event_str).ok_or_else(|| RuntimeError::Operation {
        op_type: node.op_type,
        message: format!("REGISTER_HOOK: unknown hook_event '{event_str}'"),
    })?;
    let mode = HookMode::parse(&mode_str).ok_or_else(|| RuntimeError::Operation {
        op_type: node.op_type,
        message: format!("REGISTER_HOOK: invalid hook_mode '{mode_str}'"),
    })?;
    // Gate is only valid on pre-execution events (constitution #5).
    if mode == HookMode::Gate && !event.is_pre() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "REGISTER_HOOK: gate mode is only valid on pre-execution events, got '{}'",
                event.as_str()
            ),
        });
    }

    if let Some(registry) = ctx.hook_registry() {
        registry.register(HookBinding {
            handler_id: handler_id.clone(),
            event,
            match_glob: match_glob.clone(),
            mode,
        });
        // session_start fires once, now — registration happens at session start
        // in the entry flow. Awaited async pre-step via the bridge (NOT the dead
        // sync ExecutionHook); a gate failure fails closed (T045/T046).
        if event == HookEvent::SessionStart
            && let Some(bridge) = ctx.python_tool_bridge.as_ref()
        {
            let payload = serde_json::json!({
                "__apxm_hook__": { "event": "session_start" }
            });
            let deadline = std::time::Duration::from_secs(30);
            if let Err(e) = bridge.call_hook(&handler_id, payload, deadline).await {
                if mode == HookMode::Gate {
                    return Err(RuntimeError::Operation {
                        op_type: node.op_type,
                        message: format!("session_start gate hook failed (fail-closed): {e}"),
                    });
                }
                tracing::warn!(error = %e, "observe session_start hook failed; continuing");
            }
        }
    } else {
        tracing::warn!(
            execution_id = %ctx.execution_id,
            event = %event.as_str(),
            "REGISTER_HOOK: no hook registry on context; binding not installed"
        );
    }

    ctx.aam.set_belief(
        format!("__registered_hook:{}:{}", event.as_str(), handler_id),
        Value::String(match_glob.clone()),
        TransitionLabel::Custom(format!("register_hook:{}", event.as_str())),
    );

    tracing::info!(
        execution_id = %ctx.execution_id,
        event = %event.as_str(),
        match_glob = %match_glob,
        mode = %mode.as_str(),
        "REGISTER_HOOK registered author lifecycle hook"
    );

    let mut result = HashMap::new();
    result.insert(
        "hook_event".to_string(),
        Value::String(event.as_str().to_string()),
    );
    result.insert("registered".to_string(), Value::Bool(true));
    Ok(Value::Object(result))
}
