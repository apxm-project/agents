//! Author-hook drivers — apply program-authored lifecycle hooks at the runtime
//! chokepoints, consuming `ExecutionContext::hook_registry` (constitution #5:
//! control, not just observation).
//!
//! Hooks are dispatched over the SAME python tool bridge as `@tool`
//! (constitution #4). A hook handler receives a JSON payload describing the
//! event and returns a decision object the runtime applies:
//!   - `pre_tool`  → allow | deny(reason) | edit_args(args)
//!   - `post_tool` → replace_result(x) | (none)
//!   - `pre_ask`   → prepend_system(text) | set_system(text) | (none)
//!
//! Failure semantics (FR-014, T046): a `gate` hook that errors fails CLOSED
//! (the guarded action is denied and the error surfaced); an `observe` hook that
//! errors surfaces a warning and CONTINUES.

use std::collections::HashMap;
use std::time::Duration;

use apxm_core::error::RuntimeError;
use apxm_core::types::Value;
use serde_json::{Value as JsonValue, json};

use super::ExecutionContext;
use super::hooks::{HookEvent, HookMode};
use crate::memory::MemorySpace;

/// Session-memory keys the ConversationMemoryMiddleware writes (mirrored here so
/// post_turn hooks can be handed the recent transcript window).
const TURN_COUNT_KEY: &str = "conversation:turn_count";
const TURN_PREFIX: &str = "conversation:turn:";

const HOOK_DEADLINE: Duration = Duration::from_secs(30);
const HOOK_PAYLOAD_KEY: &str = "__apxm_hook__";

/// Decision returned by a `pre_tool` hook.
pub enum PreToolDecision {
    Allow,
    Deny(String),
    EditArgs(HashMap<String, Value>),
}

fn json_to_value(v: JsonValue) -> Value {
    match v {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Bool(b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(apxm_core::types::values::Number::Integer(i))
            } else {
                Value::Number(apxm_core::types::values::Number::Float(
                    n.as_f64().unwrap_or(0.0),
                ))
            }
        }
        JsonValue::String(s) => Value::String(s),
        JsonValue::Array(a) => Value::Array(a.into_iter().map(json_to_value).collect()),
        JsonValue::Object(m) => {
            Value::Object(m.into_iter().map(|(k, v)| (k, json_to_value(v))).collect())
        }
    }
}

fn remaining_budget(ctx: &ExecutionContext) -> JsonValue {
    match ctx.token_budget {
        Some(b) => json!(b.saturating_sub(
            ctx.consumed_tokens.load(std::sync::atomic::Ordering::Relaxed)
        )),
        None => JsonValue::Null,
    }
}

/// Run all matching `pre_tool` hooks for `tool_name`, threading edited args.
/// Returns the (possibly edited) args, or `Err` if a hook denied the call.
pub async fn run_pre_tool_hooks(
    ctx: &ExecutionContext,
    tool_name: &str,
    args: HashMap<String, Value>,
) -> Result<HashMap<String, Value>, RuntimeError> {
    let Some(registry) = ctx.hook_registry() else {
        return Ok(args);
    };
    let bindings = registry.matching(HookEvent::PreTool, tool_name);
    if bindings.is_empty() {
        return Ok(args);
    }
    let Some(bridge) = ctx.python_tool_bridge.as_ref() else {
        return Ok(args);
    };

    let mut current = args;
    for binding in bindings {
        let args_json =
            serde_json::to_value(&current).unwrap_or(JsonValue::Object(Default::default()));
        let payload = json!({
            HOOK_PAYLOAD_KEY: {
                "event": "pre_tool",
                "remaining_budget": remaining_budget(ctx),
                "call": { "name": tool_name, "args": args_json },
            }
        });
        match bridge
            .call_hook(&binding.handler_id, payload, HOOK_DEADLINE)
            .await
        {
            Ok(decision) => match parse_pre_tool_decision(decision) {
                PreToolDecision::Allow => {}
                PreToolDecision::Deny(reason) => {
                    return Err(RuntimeError::Capability {
                        capability: tool_name.to_string(),
                        message: format!("denied by pre_tool hook: {reason}"),
                    });
                }
                PreToolDecision::EditArgs(new_args) => current = new_args,
            },
            Err(e) => {
                // Failure semantics (T046): gate fails closed; observe continues.
                if binding.mode == HookMode::Gate {
                    return Err(RuntimeError::Capability {
                        capability: tool_name.to_string(),
                        message: format!("pre_tool gate hook failed (fail-closed): {e}"),
                    });
                }
                tracing::warn!(tool = %tool_name, error = %e, "observe pre_tool hook failed; continuing");
            }
        }
    }
    Ok(current)
}

fn parse_pre_tool_decision(decision: JsonValue) -> PreToolDecision {
    let Some(obj) = decision.as_object() else {
        return PreToolDecision::Allow;
    };
    match obj.get("decision").and_then(|v| v.as_str()) {
        Some("deny") => PreToolDecision::Deny(
            obj.get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("denied")
                .to_string(),
        ),
        Some("edit_args") => match obj.get("args") {
            Some(JsonValue::Object(m)) => PreToolDecision::EditArgs(
                m.iter()
                    .map(|(k, v)| (k.clone(), json_to_value(v.clone())))
                    .collect(),
            ),
            _ => PreToolDecision::Allow,
        },
        _ => PreToolDecision::Allow,
    }
}

/// Run all matching `post_tool` hooks; returns the (possibly replaced) result.
pub async fn run_post_tool_hooks(
    ctx: &ExecutionContext,
    tool_name: &str,
    result: Value,
) -> Value {
    let Some(registry) = ctx.hook_registry() else {
        return result;
    };
    let bindings = registry.matching(HookEvent::PostTool, tool_name);
    if bindings.is_empty() {
        return result;
    }
    let Some(bridge) = ctx.python_tool_bridge.as_ref() else {
        return result;
    };

    let mut current = result;
    for binding in bindings {
        let result_json =
            serde_json::to_value(&current).unwrap_or(JsonValue::Null);
        let payload = json!({
            HOOK_PAYLOAD_KEY: {
                "event": "post_tool",
                "call": { "name": tool_name },
                "result": result_json,
            }
        });
        match bridge
            .call_hook(&binding.handler_id, payload, HOOK_DEADLINE)
            .await
        {
            Ok(decision) => {
                if let Some(obj) = decision.as_object()
                    && obj.get("decision").and_then(|v| v.as_str()) == Some("replace_result")
                    && let Some(r) = obj.get("result")
                {
                    current = json_to_value(r.clone());
                }
            }
            // post_tool is observe-style: surface + continue (never fail closed).
            Err(e) => {
                tracing::warn!(tool = %tool_name, error = %e, "post_tool hook failed; continuing")
            }
        }
    }
    current
}

/// Fire turn/ask-level lifecycle hooks (`pre_turn`/`post_turn`/`post_ask`) that
/// have no tool/op name to match against — they are observed for their side
/// effects (logging, memory writes via the handler). A `gate` failure on a
/// pre-event fails closed; observe failures surface and continue (FR-014).
async fn fire_lifecycle_hooks(
    ctx: &ExecutionContext,
    event: HookEvent,
    payload: JsonValue,
) -> Result<(), RuntimeError> {
    let Some(registry) = ctx.hook_registry() else {
        return Ok(());
    };
    let bindings = registry.for_event(event);
    if bindings.is_empty() {
        return Ok(());
    }
    let Some(bridge) = ctx.python_tool_bridge.as_ref() else {
        return Ok(());
    };
    for binding in bindings {
        if let Err(e) = bridge
            .call_hook(&binding.handler_id, payload.clone(), HOOK_DEADLINE)
            .await
        {
            if binding.mode == HookMode::Gate {
                return Err(RuntimeError::Operation {
                    op_type: apxm_core::types::operations::AISOperationType::Ask,
                    message: format!("{} gate hook failed (fail-closed): {e}", event.as_str()),
                });
            }
            tracing::warn!(event = %event.as_str(), error = %e, "observe lifecycle hook failed; continuing");
        }
    }
    Ok(())
}

/// Fire `pre_turn` hooks (before the turn's ask). Gate-capable (fail-closed).
pub async fn run_pre_turn_hooks(ctx: &ExecutionContext) -> Result<(), RuntimeError> {
    fire_lifecycle_hooks(
        ctx,
        HookEvent::PreTurn,
        json!({ HOOK_PAYLOAD_KEY: { "event": "pre_turn", "remaining_budget": remaining_budget(ctx) } }),
    )
    .await
}

/// Fire `post_turn` hooks (after the turn's reply). The payload is enriched with
/// the remaining budget and a recent conversation window pre-loaded from session
/// memory (the bridge is unidirectional, so the runtime hands context IN rather
/// than the worker calling back). Any memory writes the hook accumulates via
/// `ctx.umem(...)` are applied to session STM afterward — this is what makes
/// compaction/context author-controllable from a post_turn hook (constitution
/// #2/#5). Observe-style: failures surface and continue.
pub async fn run_post_turn_hooks(ctx: &ExecutionContext, reply: &str) {
    let Some(registry) = ctx.hook_registry() else {
        return;
    };
    let bindings = registry.for_event(HookEvent::PostTurn);
    if bindings.is_empty() {
        return;
    }
    let Some(bridge) = ctx.python_tool_bridge.as_ref() else {
        return;
    };

    let window = recent_window(ctx, 8).await;
    let base = json!({
        "event": "post_turn",
        "reply": reply,
        "remaining_budget": remaining_budget(ctx),
        "window": window,
    });

    for binding in bindings {
        let payload = json!({ HOOK_PAYLOAD_KEY: base.clone() });
        match bridge
            .call_hook(&binding.handler_id, payload, HOOK_DEADLINE)
            .await
        {
            Ok(decision) => apply_hook_writes(ctx, &decision).await,
            Err(e) => {
                tracing::warn!(error = %e, "observe post_turn hook failed; continuing")
            }
        }
    }
}

/// Read the last-`n` recorded turn answers from session STM (the window the
/// ConversationMemoryMiddleware accrues), oldest-first.
async fn recent_window(ctx: &ExecutionContext, n: i64) -> Vec<String> {
    let scope = ctx.memory_scope().to_string();
    let mem = ctx.memory();
    let count = mem
        .read_scoped(MemorySpace::Stm, &scope, TURN_COUNT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if count <= 0 {
        return Vec::new();
    }
    let start = (count - n + 1).max(1);
    let mut out = Vec::new();
    for i in start..=count {
        if let Ok(Some(v)) = mem
            .read_scoped(MemorySpace::Stm, &scope, &format!("{TURN_PREFIX}{i}"))
            .await
            && let Some(s) = v.as_string()
        {
            out.push(s.clone());
        }
    }
    out
}

/// Apply a hook decision's accumulated memory `writes` to session STM. Each entry
/// is `{ "key": <str>, "value": <json> }`. Best-effort; a write failure is logged.
async fn apply_hook_writes(ctx: &ExecutionContext, decision: &JsonValue) {
    let Some(writes) = decision
        .as_object()
        .and_then(|o| o.get("writes"))
        .and_then(|w| w.as_array())
    else {
        return;
    };
    if writes.is_empty() {
        return;
    }
    let scope = ctx.memory_scope().to_string();
    let mem = ctx.memory();
    for w in writes {
        let (Some(key), Some(val)) = (
            w.get("key").and_then(|k| k.as_str()),
            w.get("value"),
        ) else {
            continue;
        };
        if let Err(e) = mem
            .write_scoped(
                MemorySpace::Stm,
                &scope,
                key.to_string(),
                json_to_value(val.clone()),
            )
            .await
        {
            tracing::warn!(key = %key, error = %e, "post_turn hook memory write failed");
        }
    }
}

/// Fire `post_ask` hooks with the reply payload (after an ask completes).
pub async fn run_post_ask_hooks(ctx: &ExecutionContext, reply: &str) {
    let _ = fire_lifecycle_hooks(
        ctx,
        HookEvent::PostAsk,
        json!({ HOOK_PAYLOAD_KEY: { "event": "post_ask", "reply": reply } }),
    )
    .await;
}

/// Run all matching `pre_ask` hooks; returns a system-prompt override, if any.
/// `Some(prompt)` replaces/prepends the system prompt; `None` leaves it.
pub async fn run_pre_ask_hooks(
    ctx: &ExecutionContext,
    base_system: &str,
) -> Result<Option<String>, RuntimeError> {
    let Some(registry) = ctx.hook_registry() else {
        return Ok(None);
    };
    let bindings = registry.matching(HookEvent::PreAsk, "ask");
    if bindings.is_empty() {
        return Ok(None);
    }
    let Some(bridge) = ctx.python_tool_bridge.as_ref() else {
        return Ok(None);
    };

    // Pre-load the recent window so a pre_ask hook can recall context for
    // injection (ctx.recall_window) without a worker->runtime callback.
    let window = recent_window(ctx, 8).await;
    let mut system = base_system.to_string();
    let mut changed = false;
    for binding in bindings {
        let payload = json!({
            HOOK_PAYLOAD_KEY: {
                "event": "pre_ask",
                "remaining_budget": remaining_budget(ctx),
                "system": system,
                "window": window,
            }
        });
        match bridge
            .call_hook(&binding.handler_id, payload, HOOK_DEADLINE)
            .await
        {
            Ok(decision) => {
                if let Some(obj) = decision.as_object() {
                    match obj.get("decision").and_then(|v| v.as_str()) {
                        Some("set_system") => {
                            if let Some(t) = obj.get("text").and_then(|v| v.as_str()) {
                                system = t.to_string();
                                changed = true;
                            }
                        }
                        Some("prepend_system") => {
                            if let Some(t) = obj.get("text").and_then(|v| v.as_str()) {
                                system = format!("{t}\n{system}");
                                changed = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                if binding.mode == HookMode::Gate {
                    return Err(RuntimeError::Operation {
                        op_type: apxm_core::types::operations::AISOperationType::Ask,
                        message: format!("pre_ask gate hook failed (fail-closed): {e}"),
                    });
                }
                tracing::warn!(error = %e, "observe pre_ask hook failed; continuing");
            }
        }
    }
    Ok(if changed { Some(system) } else { None })
}
