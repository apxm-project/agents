//! Author-hook drivers — apply program-authored lifecycle hooks at the runtime
//! chokepoints, consuming `ExecutionContext::hook_registry` (constitution #5:
//! control, not just observation).
//!
//! Hooks are dispatched over the SAME python tool bridge as `@tool`
//! (constitution #4). A hook handler receives a JSON payload describing the
//! event and returns a decision object the runtime applies:
//! - `pre_cap` → allow | defer | deny(reason) | edit_args(args)
//! - `post_cap` → replace_result(x) | (none)
//! - `pre_ask` → prepend_system(text) | set_system(text) | (none)
//!
//! Failure semantics: a `gate` hook that errors fails CLOSED (the guarded
//! action is denied and the error surfaced); an `observe` hook that errors
//! surfaces a warning and CONTINUES.

use std::collections::HashMap;
use std::time::Duration;

use apxm_core::error::RuntimeError;
use apxm_core::events::payload::ToolCallCorrelation;
use apxm_core::types::values::{Number, Value};
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use super::ExecutionContext;
use super::hooks::{HookEvent, HookMode};
use crate::memory::MemorySpace;

const HOOK_DEADLINE: Duration = Duration::from_secs(30);
const HOOK_PAYLOAD_KEY: &str = "__apxm_hook__";

async fn call_hook_with_host_bridge<F, Fut>(
    ctx: &ExecutionContext,
    handler_id: &str,
    payload: JsonValue,
    deadline: Duration,
    host: F,
) -> Result<JsonValue, RuntimeError>
where
    F: Fn(String, JsonValue) -> Fut + Clone,
    Fut: std::future::Future<Output = std::result::Result<JsonValue, String>>,
{
    if let Some(bridge) = ctx.python_handler_bridge.as_ref() {
        match bridge
            .call_hook_with_host(handler_id, payload.clone(), deadline, {
                let host = host.clone();
                move |method, params| host(method, params)
            })
            .await
        {
            Ok(value) => return Ok(value),
            Err(err) => {
                if ctx.typescript_handler_bridge.is_none() {
                    return Err(err);
                }
            }
        }
    }
    if let Some(bridge) = ctx.typescript_handler_bridge.as_ref() {
        return bridge
            .call_hook_with_host(handler_id, payload, deadline, move |method, params| {
                host(method, params)
            })
            .await;
    }
    Err(RuntimeError::Capability {
        capability: "hooks".into(),
        message: "hook dispatch requested but no handler bridge is configured".into(),
    })
}

/// Service a worker-initiated host call (e.g. `ctx.summarize` → `llm.ask`) with
/// the SAME `ExecutionContext` that owns the hook, so the hook's LLM call runs
/// under the session's backend, budget, and cancellation — not a detached one.
/// This is what lets compaction (and any hook) summarize with a real model
/// despite running in the sandboxed worker subprocess.
async fn dispatch_host_call(
    ctx: &ExecutionContext,
    method: String,
    params: JsonValue,
) -> std::result::Result<JsonValue, String> {
    match method.as_str() {
        "llm.ask" => host_llm_ask(ctx, params).await,
        "tool.call" => host_tool_call(ctx, params).await,
        "mem.read" => host_mem_read(ctx, params).await,
        "mem.recent" => host_mem_recent(ctx, params).await,
        other => Err(format!("unknown host method '{}'", other)),
    }
}

/// Invoke a read-only apxm capability on behalf of a hook (e.g. `count_tokens`,
/// `http_get`, `search_skills`). Writes are NOT serviced here — a hook that needs
/// a write declares it via `ctx.umem`/the decision writes, which go through the
/// audited memory path. This is apxm handing the user its TOOLS; the user's hook
/// decides which to call and what to do with the result.
async fn host_tool_call(
    ctx: &ExecutionContext,
    params: JsonValue,
) -> std::result::Result<JsonValue, String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("tool.call requires a 'name'")?;
    if !ctx.capability_system.has_capability(name) {
        return Err(format!("capability '{name}' is not registered"));
    }
    if !ctx.capability_system.is_read_only(name) {
        return Err(format!(
            "capability '{name}' is not read-only; hooks may only call read-only tools"
        ));
    }
    let args: HashMap<String, Value> = params
        .get("args")
        .and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), json_to_value(v.clone())))
                .collect()
        })
        .unwrap_or_default();
    match ctx.invoke_capability(name, args).await {
        Ok(value) => Ok(value_to_json(value)),
        Err(e) => Err(format!("tool '{name}' failed: {e}")),
    }
}

/// Return the recent transcript window (oldest-first) of size `n` over `prefix`
/// for a hook. The hook chooses `n` — apxm does not cap how much context the
/// user may compact over (the previous fixed payload window did).
async fn host_mem_recent(
    ctx: &ExecutionContext,
    params: JsonValue,
) -> std::result::Result<JsonValue, String> {
    let prefix = params
        .get("prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("conversation:");
    let n = params.get("n").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
    let items = ctx
        .memory()
        .recent_scoped(MemorySpace::Stm, ctx.memory_scope(), prefix, n, &[])
        .await
        .map_err(|e| format!("mem.recent failed: {e}"))?;
    let texts: Vec<JsonValue> = items
        .into_iter()
        .filter_map(|r| {
            r.value
                .as_string()
                .map(|s| JsonValue::String(s.to_string()))
        })
        .collect();
    Ok(JsonValue::Array(texts))
}

/// Read a session-scoped STM key for a hook. WHICH key is the user's choice
/// (e.g. their rolling-summary key), so no key name is baked into the runtime.
async fn host_mem_read(
    ctx: &ExecutionContext,
    params: JsonValue,
) -> std::result::Result<JsonValue, String> {
    let key = params
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or("mem.read requires a 'key'")?;
    match ctx
        .memory()
        .read_scoped(MemorySpace::Stm, ctx.memory_scope(), key)
        .await
    {
        Ok(Some(v)) => Ok(serde_json::to_value(&v).unwrap_or(JsonValue::Null)),
        Ok(None) => Ok(JsonValue::Null),
        Err(e) => Err(format!("mem.read '{key}' failed: {e}")),
    }
}

/// One-shot LLM ask for hooks. Calls the backend DIRECTLY (non-streaming, no
/// event emitter, no nested `pre_ask` hooks). This is deliberate:
/// - no emitter → a hook's own LLM call (e.g. compaction's summarize) never
/// leaks tokens into the USER's reply stream;
/// - no nested hooks → no `pre_ask → llm → pre_ask` re-entrancy.
///
/// Routes through the ModelRouter when present (circuit breakers + policy).
async fn host_llm_ask(
    ctx: &ExecutionContext,
    params: JsonValue,
) -> std::result::Result<JsonValue, String> {
    use apxm_backends::LLMRequest;
    use apxm_core::types::operations::AISOperationType;

    let prompt = params
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if prompt.trim().is_empty() {
        return Err("llm.ask requires a non-empty 'prompt'".to_string());
    }
    let mut request = LLMRequest::new(prompt).with_operation_type(AISOperationType::Ask);
    if let Some(system) = params.get("system").and_then(|v| v.as_str()) {
        request = request.with_system_prompt(system.to_string());
    }
    let response = if let Some(router) = &ctx.model_router {
        router.generate(request).await
    } else {
        ctx.llm_registry.generate(request).await
    }
    .map_err(|e| format!("llm.ask failed: {e}"))?;
    Ok(JsonValue::String(response.content))
}

/// Decision returned by a `pre_cap` hook.
pub enum PreCapDecision {
    Allow,
    Defer,
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

fn value_to_json(v: Value) -> JsonValue {
    match v {
        Value::Null => JsonValue::Null,
        Value::Bool(b) => JsonValue::Bool(b),
        Value::Number(Number::Integer(i)) => JsonValue::Number(i.into()),
        Value::Number(Number::Float(f)) => {
            serde_json::Number::from_f64(f).map_or(JsonValue::Null, JsonValue::Number)
        }
        Value::String(s) => JsonValue::String(s),
        Value::Array(values) => JsonValue::Array(values.into_iter().map(value_to_json).collect()),
        Value::Object(values) => JsonValue::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, value_to_json(value)))
                .collect(),
        ),
        other => serde_json::to_value(&other).unwrap_or(JsonValue::Null),
    }
}

fn remaining_budget(ctx: &ExecutionContext) -> JsonValue {
    match ctx.token_budget {
        Some(b) => json!(
            b.saturating_sub(
                ctx.consumed_tokens
                    .load(std::sync::atomic::Ordering::Relaxed)
            )
        ),
        None => JsonValue::Null,
    }
}

fn canonical_requires_approval(ctx: &ExecutionContext, tool_name: &str) -> Option<bool> {
    ctx.capability_system
        .get_metadata(tool_name)
        .map(|meta| meta.requires_approval)
        .or_else(|| {
            crate::executor::handlers::llm::script_tool_policy(ctx, tool_name)
                .ok()
                .map(|policy| policy.requires_approval)
        })
}

/// Run all matching `pre_cap` hooks for `tool_name`, threading edited args.
/// A successful return always continues into canonical capability admission;
/// neither `allow` nor `defer` authorizes the invocation by itself.
pub async fn run_pre_cap_hooks(
    ctx: &ExecutionContext,
    tool_name: &str,
    args: HashMap<String, Value>,
    correlation: Option<&ToolCallCorrelation>,
) -> Result<HashMap<String, Value>, RuntimeError> {
    let Some(registry) = ctx.hook_registry() else {
        return Ok(args);
    };
    let bindings = registry.matching(HookEvent::PreCap, tool_name);
    if bindings.is_empty() {
        return Ok(args);
    }
    if ctx.python_handler_bridge.is_none() && ctx.typescript_handler_bridge.is_none() {
        return Ok(args);
    }

    // A gate hook may only narrow a capability whose canonical policy is
    // already open. Missing metadata is not evidence that allow is safe.
    let requires_approval = canonical_requires_approval(ctx, tool_name);

    let mut current = args;
    for binding in bindings {
        let args_json =
            serde_json::to_value(&current).unwrap_or(JsonValue::Object(Default::default()));
        let payload = json!({
        HOOK_PAYLOAD_KEY: {
        "event": "pre_cap",
        "remaining_budget": remaining_budget(ctx),
        "call": {
            "name": tool_name,
            "args": args_json,
            "tool_call_correlation": correlation,
        },
        }
        });
        match call_hook_with_host_bridge(
            ctx,
            &binding.handler_id,
            payload,
            HOOK_DEADLINE,
            |method, params| dispatch_host_call(ctx, method, params),
        )
        .await
        {
            Ok(decision) => match parse_pre_cap_decision(decision) {
                decision @ (PreCapDecision::Allow | PreCapDecision::Defer) => {
                    if pre_cap_continuation(binding.mode, requires_approval, &decision)
                        == PreCapContinuation::RejectWidening
                    {
                        return Err(RuntimeError::Capability {
                            capability: tool_name.to_string(),
                            message: format!(
                                "gate hook '{}' attempted to authorize a capability whose policy \
 is not already open; rejected — a hook may only narrow canonical \
 capability policy, never replace or widen it",
                                binding.handler_id
                            ),
                        });
                    }
                }
                PreCapDecision::Deny(reason) => {
                    return Err(RuntimeError::Capability {
                        capability: tool_name.to_string(),
                        message: format!("denied by pre_cap hook: {reason}"),
                    });
                }
                PreCapDecision::EditArgs(new_args) => current = new_args,
            },
            Err(e) => {
                // Gate hooks fail closed; observe hooks continue.
                if binding.mode == HookMode::Gate {
                    return Err(RuntimeError::Capability {
                        capability: tool_name.to_string(),
                        message: format!("pre_cap gate hook failed (fail-closed): {e}"),
                    });
                }
                tracing::warn!(tool = %tool_name, error = %e, "observe pre_cap hook failed; continuing");
            }
        }
    }
    Ok(current)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreCapContinuation {
    CanonicalAdmission,
    RejectWidening,
}

/// Resolve a nonterminal hook decision without granting authority. `defer`
/// always continues into canonical admission. `allow` does the same only when
/// metadata proves the capability policy is already open.
fn pre_cap_continuation(
    mode: HookMode,
    capability_requires_approval: Option<bool>,
    decision: &PreCapDecision,
) -> PreCapContinuation {
    match decision {
        PreCapDecision::Allow
            if mode == HookMode::Gate && capability_requires_approval != Some(false) =>
        {
            PreCapContinuation::RejectWidening
        }
        PreCapDecision::Allow | PreCapDecision::Defer => PreCapContinuation::CanonicalAdmission,
        _ => unreachable!("only nonterminal pre_cap decisions have a continuation"),
    }
}

/// Fold one `pre_turn` hook's decision into the running supplement.
/// `set_system` replaces; `prepend_system` stacks ahead of whatever earlier
/// `pre_turn` hooks already contributed (first-registered ends up innermost,
/// matching `pre_ask`'s prepend order). Anything else (missing/unknown
/// `decision`, non-object, no `text`) is a no-op — pure function, no I/O.
fn apply_pre_turn_decision(supplement: Option<String>, decision: &JsonValue) -> Option<String> {
    let Some(obj) = decision.as_object() else {
        return supplement;
    };
    match obj.get("decision").and_then(|v| v.as_str()) {
        Some("set_system") => obj
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(supplement),
        Some("prepend_system") => match obj.get("text").and_then(|v| v.as_str()) {
            Some(t) => Some(match supplement {
                Some(prev) => format!("{t}\n{prev}"),
                None => t.to_string(),
            }),
            None => supplement,
        },
        _ => supplement,
    }
}

fn parse_pre_cap_decision(decision: JsonValue) -> PreCapDecision {
    let Some(obj) = decision.as_object() else {
        return PreCapDecision::Defer;
    };
    match obj.get("decision").and_then(|v| v.as_str()) {
        Some("allow") => PreCapDecision::Allow,
        Some("defer") => PreCapDecision::Defer,
        Some("deny") => PreCapDecision::Deny(
            obj.get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("denied")
                .to_string(),
        ),
        Some("edit_args") => match obj.get("args") {
            Some(JsonValue::Object(m)) => PreCapDecision::EditArgs(
                m.iter()
                    .map(|(k, v)| (k.clone(), json_to_value(v.clone())))
                    .collect(),
            ),
            _ => PreCapDecision::Defer,
        },
        _ => PreCapDecision::Defer,
    }
}

/// Run all matching `post_cap` hooks; returns the (possibly replaced) result.
pub async fn run_post_cap_hooks(
    ctx: &ExecutionContext,
    tool_name: &str,
    result: Value,
    correlation: Option<&ToolCallCorrelation>,
) -> Value {
    let Some(registry) = ctx.hook_registry() else {
        return result;
    };
    let bindings = registry.matching(HookEvent::PostCap, tool_name);
    if bindings.is_empty() {
        return result;
    }
    if ctx.python_handler_bridge.is_none() && ctx.typescript_handler_bridge.is_none() {
        return result;
    }

    let mut current = result;
    for binding in bindings {
        let result_json = serde_json::to_value(&current).unwrap_or(JsonValue::Null);
        let payload = json!({
        HOOK_PAYLOAD_KEY: {
        "event": "post_cap",
        "call": {
            "name": tool_name,
            "tool_call_correlation": correlation,
        },
        "result": result_json,
        }
        });
        match call_hook_with_host_bridge(
            ctx,
            &binding.handler_id,
            payload,
            HOOK_DEADLINE,
            |method, params| dispatch_host_call(ctx, method, params),
        )
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
            // post_cap is observe-style: surface + continue (never fail closed).
            Err(e) => {
                tracing::warn!(tool = %tool_name, error = %e, "post_cap hook failed; continuing")
            }
        }
    }
    current
}

/// Fire turn/ask-level lifecycle hooks (`pre_turn`/`post_turn`/`post_ask`) that
/// have no tool/op name to match against — they are observed for their side
/// effects (logging, memory writes via the handler). A `gate` failure on a
/// pre-event fails closed; observe failures surface and continue.
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
    if ctx.python_handler_bridge.is_none() && ctx.typescript_handler_bridge.is_none() {
        return Ok(());
    }
    for binding in bindings {
        if let Err(e) = call_hook_with_host_bridge(
            ctx,
            &binding.handler_id,
            payload.clone(),
            HOOK_DEADLINE,
            |method, params| dispatch_host_call(ctx, method, params),
        )
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
///
/// `turn_context` is the turn's structured `context` field when the host
/// supplies one. `None` means the compiled AIR did not bind an additional
/// per-turn JSON payload for this call.
///
/// Like `pre_ask`, a hook may return a `set_system`/`prepend_system` decision;
/// the (possibly combined, first-wins-then-chains) text is returned so the
/// caller can stash it where the ask handler's system-prompt composition picks
/// it up (`ExecutionContext::pending_turn_prompt_supplement`).
pub async fn run_pre_turn_hooks(
    ctx: &ExecutionContext,
    turn_context: Option<JsonMap<String, JsonValue>>,
) -> Result<Option<String>, RuntimeError> {
    let Some(registry) = ctx.hook_registry() else {
        return Ok(None);
    };
    let bindings = registry.for_event(HookEvent::PreTurn);
    if bindings.is_empty() {
        return Ok(None);
    }
    if ctx.python_handler_bridge.is_none() && ctx.typescript_handler_bridge.is_none() {
        return Ok(None);
    }

    let mut supplement: Option<String> = None;
    for binding in bindings {
        let payload = pre_turn_payload(ctx, turn_context.clone());
        match call_hook_with_host_bridge(
            ctx,
            &binding.handler_id,
            payload,
            HOOK_DEADLINE,
            |method, params| dispatch_host_call(ctx, method, params),
        )
        .await
        {
            Ok(decision) => supplement = apply_pre_turn_decision(supplement, &decision),
            Err(e) => {
                if binding.mode == HookMode::Gate {
                    return Err(RuntimeError::Operation {
                        op_type: apxm_core::types::operations::AISOperationType::Ask,
                        message: format!("pre_turn gate hook failed (fail-closed): {e}"),
                    });
                }
                tracing::warn!(error = %e, "observe pre_turn hook failed; continuing");
            }
        }
    }
    Ok(supplement)
}

fn pre_turn_payload(
    ctx: &ExecutionContext,
    turn_context: Option<JsonMap<String, JsonValue>>,
) -> JsonValue {
    json!({
    HOOK_PAYLOAD_KEY: {
    "event": "pre_turn",
    "remaining_budget": remaining_budget(ctx),
    "context": turn_context.map(JsonValue::Object).unwrap_or(JsonValue::Null),
    }
    })
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
    if ctx.python_handler_bridge.is_none() && ctx.typescript_handler_bridge.is_none() {
        return;
    }

    // The hook reads whatever it wants on demand — the transcript window via
    // `ctx.recall_window(n)` (mem.recent, user-chosen `n`) and any prior state
    // via `ctx.recall(key)` (mem.read). The runtime bakes no window size or key;
    // we hand IN only the reply and remaining budget.
    let base = json!({
    "event": "post_turn",
    "reply": reply,
    "remaining_budget": remaining_budget(ctx),
    });

    for binding in bindings {
        let payload = json!({ HOOK_PAYLOAD_KEY: base.clone() });
        match call_hook_with_host_bridge(
            ctx,
            &binding.handler_id,
            payload,
            HOOK_DEADLINE,
            |method, params| dispatch_host_call(ctx, method, params),
        )
        .await
        {
            Ok(decision) => apply_hook_writes(ctx, &decision).await,
            Err(e) => {
                tracing::warn!(error = %e, "observe post_turn hook failed; continuing")
            }
        }
    }
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
        let (Some(key), Some(val)) = (w.get("key").and_then(|k| k.as_str()), w.get("value")) else {
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
    if ctx.python_handler_bridge.is_none() && ctx.typescript_handler_bridge.is_none() {
        return Ok(None);
    }

    // The hook recalls context on demand via `ctx.recall_window(n)` (mem.recent),
    // choosing how much to pull — no fixed window is baked into the payload.
    let mut system = base_system.to_string();
    let mut changed = false;
    for binding in bindings {
        let payload = json!({
        HOOK_PAYLOAD_KEY: {
        "event": "pre_ask",
        "remaining_budget": remaining_budget(ctx),
        "system": system,
        }
        });
        match call_hook_with_host_bridge(
            ctx,
            &binding.handler_id,
            payload,
            HOOK_DEADLINE,
            |method, params| dispatch_host_call(ctx, method, params),
        )
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

#[cfg(test)]
mod script_bridge_tests {
    /// Guard used before dispatching lifecycle hooks.
    fn script_handler_bridge_available(python: bool, typescript: bool) -> bool {
        python || typescript
    }

    #[test]
    fn hook_driver_accepts_python_or_typescript_bridge() {
        assert!(!script_handler_bridge_available(false, false));
        assert!(script_handler_bridge_available(true, false));
        assert!(script_handler_bridge_available(false, true));
        assert!(script_handler_bridge_available(true, true));
    }
}

#[cfg(test)]
mod gate_narrowing_tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use crate::python_tools::{PythonHandlerBridge, PythonHandlerRegistry};
    use apxm_backends::LLMRegistry;
    use std::sync::Arc;

    async fn context_with_python_policy(policy: &str) -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        let registry = PythonHandlerRegistry::from_json(policy).expect("python tool manifest");
        ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam)
            .with_python_handler_bridge(Arc::new(PythonHandlerBridge::new(registry)))
    }

    /// vector: a `gate` hook attempting to widen (auto-allow) a
    /// capability whose baseline policy requires approval is rejected.
    #[test]
    fn gate_hook_allow_on_approval_gated_capability_is_a_widen_attempt() {
        assert_eq!(
            pre_cap_continuation(HookMode::Gate, Some(true), &PreCapDecision::Allow),
            PreCapContinuation::RejectWidening
        );
    }

    #[test]
    fn gate_hook_allow_on_already_open_capability_continues_admission() {
        assert_eq!(
            pre_cap_continuation(HookMode::Gate, Some(false), &PreCapDecision::Allow),
            PreCapContinuation::CanonicalAdmission
        );
    }

    #[test]
    fn gate_hook_allow_without_known_policy_is_rejected() {
        assert_eq!(
            pre_cap_continuation(HookMode::Gate, None, &PreCapDecision::Allow),
            PreCapContinuation::RejectWidening
        );
    }

    #[test]
    fn observe_hook_allow_continues_admission() {
        assert_eq!(
            pre_cap_continuation(HookMode::Observe, Some(true), &PreCapDecision::Allow),
            PreCapContinuation::CanonicalAdmission
        );
    }

    #[test]
    fn defer_always_continues_canonical_admission() {
        assert_eq!(
            pre_cap_continuation(HookMode::Gate, Some(true), &PreCapDecision::Defer),
            PreCapContinuation::CanonicalAdmission
        );
        assert_eq!(
            pre_cap_continuation(HookMode::Gate, None, &PreCapDecision::Defer),
            PreCapContinuation::CanonicalAdmission
        );
    }

    #[test]
    fn defer_is_parsed_as_canonical_admission_continuation() {
        let decision = parse_pre_cap_decision(json!({"decision": "defer"}));
        assert!(matches!(decision, PreCapDecision::Defer));
        assert_eq!(
            pre_cap_continuation(HookMode::Gate, Some(true), &decision),
            PreCapContinuation::CanonicalAdmission
        );
    }

    #[tokio::test]
    async fn script_policy_participates_in_gate_narrowing() {
        let ctx = context_with_python_policy(
            r#"[{"handler_id":"sha256:abc","module":"mod","qualname":"open","name":"script-open","schema":{},"read_only":true,"requires_approval":false},{"handler_id":"sha256:def","module":"mod","qualname":"ask","name":"script-ask","schema":{},"read_only":true,"requires_approval":true}]"#,
        )
        .await;

        assert_eq!(
            canonical_requires_approval(&ctx, "script-open"),
            Some(false)
        );
        assert_eq!(canonical_requires_approval(&ctx, "script-ask"), Some(true));
    }

    #[tokio::test]
    async fn incomplete_script_policy_is_not_treated_as_open() {
        let ctx = context_with_python_policy(
            r#"[{"handler_id":"sha256:abc","module":"mod","qualname":"open","name":"script-open","schema":{},"requires_approval":false}]"#,
        )
        .await;

        assert_eq!(canonical_requires_approval(&ctx, "script-open"), None);
    }
}

#[cfg(test)]
mod pre_turn_decision_tests {
    use super::*;

    #[test]
    fn prepend_system_with_no_prior_supplement_sets_it() {
        let decision = json!({ "decision": "prepend_system", "text": "A" });
        assert_eq!(
            apply_pre_turn_decision(None, &decision),
            Some("A".to_string())
        );
    }

    #[test]
    fn prepend_system_stacks_ahead_of_prior_supplement() {
        let decision = json!({ "decision": "prepend_system", "text": "B" });
        let result = apply_pre_turn_decision(Some("A".to_string()), &decision);
        assert_eq!(result, Some("B\nA".to_string()));
    }

    #[test]
    fn set_system_replaces_prior_supplement() {
        let decision = json!({ "decision": "set_system", "text": "B" });
        let result = apply_pre_turn_decision(Some("A".to_string()), &decision);
        assert_eq!(result, Some("B".to_string()));
    }

    #[test]
    fn unknown_decision_is_a_no_op() {
        let decision = json!({ "decision": "allow" });
        assert_eq!(
            apply_pre_turn_decision(Some("A".to_string()), &decision),
            Some("A".to_string())
        );
    }

    #[test]
    fn missing_text_is_a_no_op() {
        let decision = json!({ "decision": "prepend_system" });
        assert_eq!(apply_pre_turn_decision(None, &decision), None);
    }

    #[test]
    fn non_object_decision_is_a_no_op() {
        let decision = JsonValue::Null;
        assert_eq!(
            apply_pre_turn_decision(Some("A".to_string()), &decision),
            Some("A".to_string())
        );
    }
}
