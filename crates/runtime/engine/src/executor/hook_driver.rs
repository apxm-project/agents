//! Author-hook drivers — apply program-authored lifecycle hooks at the runtime
//! chokepoints, consuming `ExecutionContext::hook_registry` (constitution #5:
//! control, not just observation).
//!
//! Hooks are dispatched over the SAME python tool bridge as `@tool`
//! (constitution #4). A hook handler receives a JSON payload describing the
//! event and returns a decision object the runtime applies:
//! - `pre_cap` → allow | defer | deny(reason) | edit_args(args)
//! - `post_cap` → replace_result(x) | (none)
//!
//! Failure semantics: a `gate` hook that errors fails CLOSED (the guarded
//! action is denied and the error surfaced); an `observe` hook that errors
//! surfaces a warning and CONTINUES.

use std::collections::HashMap;
use std::time::Duration;

use apxm_capability_iface::events::{ModelContextCallKind, ModelContextMetrics};
use apxm_core::error::RuntimeError;
use apxm_core::events::payload::{GenerationIdentity, ToolCallCorrelation, UsagePayload};
use apxm_core::types::context_contracts::ContextLifecycleEventPayload;
use apxm_core::types::values::{Number, Value};
use serde_json::{Value as JsonValue, json};
use sha2::{Digest, Sha256};

use super::ExecutionContext;
use super::hooks::{HookBinding, HookEvent, HookMode};
use crate::context_envelope::lifecycle_metadata_from_serialized;
use crate::memory::MemorySpace;

const HOOK_DEADLINE: Duration = Duration::from_secs(30);
const HOOK_PAYLOAD_KEY: &str = "__apxm_hook__";
const CONTEXT_CONTRIBUTION_DECISION_KEY: &str = "context_contribution";
const CONTEXT_CONTRIBUTION_CONTENT_KEY: &str = "content";
const CONTEXT_CONTRIBUTION_TRUST_KEY: &str = "trust";
const CONTEXT_CONTRIBUTION_INSTRUCTION_TRUST: &str = "instruction";
const CONTEXT_CONTRIBUTION_LIFECYCLE_KIND: &str = "context_contribution";
const SHA256_PREFIX: &str = "sha256:";

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
    call_kind: ModelContextCallKind,
) -> std::result::Result<JsonValue, String> {
    match method.as_str() {
        "llm.ask" => host_llm_ask(ctx, params, call_kind).await,
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

/// One-shot LLM ask for hooks. Uses the shared egress admission path without
/// streaming, an event emitter, or nested lifecycle hooks. This is deliberate:
/// - no emitter → a hook's own LLM call (e.g. compaction's summarize) never
///   leaks tokens into the USER's reply stream;
/// - no nested hooks → no lifecycle re-entrancy.
///
/// Routes through the ModelRouter when present (circuit breakers + policy).
async fn host_llm_ask(
    ctx: &ExecutionContext,
    params: JsonValue,
    call_kind: ModelContextCallKind,
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
    let output_tokens = hook_output_token_limit(&params)?;
    let generation = GenerationIdentity::new(uuid::Uuid::now_v7().to_string(), 1, 1);
    let mut request = LLMRequest::new(prompt)
        .with_operation_type(AISOperationType::Ask)
        .with_max_tokens(output_tokens);
    if let Some(system) = params.get("system").and_then(|v| v.as_str()) {
        request = request.with_system_prompt(system.to_string());
    }
    let admission = crate::executor::handlers::llm::admit_model_egress(ctx, &request)
        .map_err(|error| format!("llm.ask budget reservation failed: {error}"))?;
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_model_context_metrics(
            &ModelContextMetrics::unplanned(None, call_kind).with_generation(generation.clone()),
        );
    }
    let response = if let Some(router) = &ctx.model_router {
        let (response, decision) = router
            .generate_with_decision(admission.request)
            .await
            .map_err(|error| format!("llm.ask routed dispatch failed: {error}"))?;
        if let Some(emitter) = &ctx.event_emitter {
            crate::executor::handlers::emit_model_route_decision_event(emitter.as_ref(), &decision);
        }
        Ok(response)
    } else {
        ctx.llm_registry.generate(admission.request).await
    }
    .map_err(|e| format!("llm.ask failed: {e}"))?;
    admission
        .reservation
        .reconcile(&response.usage)
        .map_err(|error| format!("llm.ask budget reconciliation failed: {error}"))?;
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_model_usage(UsagePayload {
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            generation: Some(generation),
        });
    }
    Ok(JsonValue::String(response.content))
}

/// Emit a redacted contribution receipt only when a hook has an explicit host
/// grant that matches the sealed context policy. The returned instruction is
/// deliberately not merged into the current runtime prompt: host assembly owns
/// frame construction, and ordinary hook output remains data.
fn emit_trusted_instruction_contribution(
    ctx: &ExecutionContext,
    binding: &HookBinding,
    decision: &JsonValue,
) -> Result<(), RuntimeError> {
    let Some(contribution) = decision
        .as_object()
        .and_then(|decision| decision.get(CONTEXT_CONTRIBUTION_DECISION_KEY))
        .and_then(JsonValue::as_object)
    else {
        return Ok(());
    };
    let trust = contribution
        .get(CONTEXT_CONTRIBUTION_TRUST_KEY)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            RuntimeError::Executor("context contribution requires a trust class".to_string())
        })?;
    if trust != CONTEXT_CONTRIBUTION_INSTRUCTION_TRUST {
        return Err(RuntimeError::Executor(
            "hook context contributions may only request the instruction trust class".to_string(),
        ));
    }
    let content = contribution
        .get(CONTEXT_CONTRIBUTION_CONTENT_KEY)
        .and_then(JsonValue::as_str)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| {
            RuntimeError::Executor("context contribution requires non-empty content".to_string())
        })?;
    let registry = ctx.hook_registry().ok_or_else(|| {
        RuntimeError::Executor(
            "trusted instruction contribution requires a hook registry".to_string(),
        )
    })?;
    let authority = registry
        .instruction_contribution_authority(&binding.handler_id)
        .ok_or_else(|| {
            RuntimeError::Executor(format!(
                "hook '{}' has no trusted instruction contribution authority",
                binding.handler_id
            ))
        })?;
    let serialized = ctx
        .metadata
        .get(crate::metadata_keys::SEALED_CONTEXT_TRANSPORT_V1)
        .ok_or_else(|| {
            RuntimeError::Executor(
                "trusted instruction contribution requires a sealed context transport".to_string(),
            )
        })?;
    let context = lifecycle_metadata_from_serialized(serialized).map_err(|error| {
        RuntimeError::Executor(format!(
            "trusted instruction contribution requires a valid sealed context: {error}"
        ))
    })?;
    if authority.policy_ref() != context.trusted_instruction_hook_policy_ref {
        return Err(RuntimeError::Executor(
            "trusted instruction contribution authority does not match the sealed context policy"
                .to_string(),
        ));
    }
    let contribution_digest = format!("{SHA256_PREFIX}{:x}", Sha256::digest(content.as_bytes()));
    let payload = ContextLifecycleEventPayload {
        lifecycle_kind: CONTEXT_CONTRIBUTION_LIFECYCLE_KIND.to_string(),
        context_id: context.context_id,
        invocation_id: context.invocation_id,
        context_digest: context.context_digest,
        policy_ref: context.policy_ref,
        frame_count: context.frame_count,
        token_count: context.token_count,
        content_redacted: true,
        compaction_ref: None,
        contributor_ref: Some(binding.handler_id.clone()),
        authority_ref: Some(authority.policy_ref().to_string()),
        contribution_digest: Some(contribution_digest),
        contribution_trust: Some(CONTEXT_CONTRIBUTION_INSTRUCTION_TRUST.to_string()),
    };
    payload.validate().map_err(RuntimeError::Executor)?;
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_context_lifecycle(&payload);
    }
    Ok(())
}

fn hook_output_token_limit(params: &JsonValue) -> std::result::Result<usize, String> {
    let raw = params
        .get("max_tokens")
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| "llm.ask requires an explicit non-negative 'max_tokens'".to_string())?;
    usize::try_from(raw)
        .map_err(|_| "llm.ask 'max_tokens' exceeds the platform maximum".to_string())
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
            |method, params| dispatch_host_call(ctx, method, params, ModelContextCallKind::Hook),
        )
        .await
        {
            Ok(decision) => {
                emit_trusted_instruction_contribution(ctx, &binding, &decision)?;
                match parse_pre_cap_decision(decision) {
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
                }
            }
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
            |method, params| dispatch_host_call(ctx, method, params, ModelContextCallKind::Hook),
        )
        .await
        {
            Ok(decision) => {
                if let Err(error) = emit_trusted_instruction_contribution(ctx, &binding, &decision)
                {
                    tracing::warn!(tool = %tool_name, error = %error, "post_cap context contribution rejected");
                }
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
        match call_hook_with_host_bridge(
            ctx,
            &binding.handler_id,
            payload.clone(),
            HOOK_DEADLINE,
            |method, params| dispatch_host_call(ctx, method, params, ModelContextCallKind::Hook),
        )
        .await
        {
            Ok(decision) => {
                if let Err(error) = emit_trusted_instruction_contribution(ctx, &binding, &decision)
                {
                    if binding.mode == HookMode::Gate {
                        return Err(error);
                    }
                    tracing::warn!(event = %event.as_str(), error = %error, "observe lifecycle context contribution rejected");
                }
            }
            Err(e) => {
                if binding.mode == HookMode::Gate {
                    return Err(RuntimeError::Operation {
                        op_type: apxm_core::types::operations::AISOperationType::Ask,
                        message: format!("{} gate hook failed (fail-closed): {e}", event.as_str()),
                    });
                }
                tracing::warn!(event = %event.as_str(), error = %e, "observe lifecycle hook failed; continuing");
            }
        }
    }
    Ok(())
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
            |method, params| {
                dispatch_host_call(ctx, method, params, ModelContextCallKind::Compaction)
            },
        )
        .await
        {
            Ok(decision) => {
                if let Err(error) = emit_trusted_instruction_contribution(ctx, &binding, &decision)
                {
                    tracing::warn!(error = %error, "post_turn context contribution rejected");
                }
                apply_hook_writes(ctx, &decision).await;
            }
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

#[cfg(test)]
mod script_bridge_tests {
    use super::hook_output_token_limit;
    use serde_json::json;
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

    #[test]
    fn hook_llm_request_requires_explicit_output_reservation() {
        assert!(hook_output_token_limit(&json!({})).is_err());
        assert_eq!(
            hook_output_token_limit(&json!({"max_tokens": 23})).unwrap(),
            23
        );
    }
}

#[cfg(test)]
mod admitted_budget_e2e_tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::context_stack::{ContextPlanningPolicy, ContextTokenizer};
    use crate::executor::{HookBinding, HookRegistry};
    use crate::memory::{MemoryConfig, MemorySystem};
    use crate::python_tools::{PythonHandlerBridge, PythonHandlerRegistry};
    use apxm_backends::LLMRegistry;
    use apxm_backends::llm::backends::MockLLMBackend;
    use apxm_capability_iface::events::ExecutionEventEmitter;
    use apxm_core::events::payload::UsagePayload;
    use apxm_core::types::values::Value;
    use apxm_core::types::{
        HandlerDescriptor, HandlerKind, HandlerLanguage, HandlerManifest, HandlerSource,
        context_contracts::BudgetSet,
    };
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;

    const COMPACTION_BACKEND: &str = "compaction-budget-backend";
    const COMPACTION_MODEL: &str = "compaction-budget-model";
    const COMPACTION_HANDLER_ID: &str =
        "sha256:de5e2f09a4b6c7d8e9f00112233445566778899aabbccddeeff0011223344556";
    const COMPACTION_HOOK_SOURCE: &str = r#"
def compact(ctx, reply):
    ctx.ask("summarize the completed turn", 8)
    return None
"#;

    #[derive(Default)]
    struct RecordedModelLifecycle {
        metrics: Mutex<Vec<ModelContextMetrics>>,
        usage: Mutex<Vec<UsagePayload>>,
    }

    impl ExecutionEventEmitter for RecordedModelLifecycle {
        fn emit_llm_token(&self, _content: &str) {}

        fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}

        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_model_context_metrics(&self, metrics: &ModelContextMetrics) {
            self.metrics
                .lock()
                .expect("metrics lock")
                .push(metrics.clone());
        }

        fn emit_model_usage(&self, usage: UsagePayload) {
            self.usage.lock().expect("usage lock").push(usage);
        }
    }

    fn admitted_budget(input_tokens: u64, output_tokens: u64) -> BudgetSet {
        BudgetSet {
            input_tokens,
            output_tokens,
            tool_calls: 0,
            memory_bytes: 0,
            concurrency: 1,
            effects: 0,
            wall_clock_ms: 1,
        }
    }

    fn compaction_hook_bridge() -> Arc<PythonHandlerBridge> {
        let manifest = HandlerManifest::new(vec![HandlerDescriptor {
            kind: HandlerKind::Hook,
            language: HandlerLanguage::Python,
            handler_id: COMPACTION_HANDLER_ID.to_string(),
            module: "compaction_budget_hook".to_string(),
            qualname: "compact".to_string(),
            name: "compact".to_string(),
            source: HandlerSource {
                artifact_path: "handlers/compaction_budget_hook.py".to_string(),
                content: COMPACTION_HOOK_SOURCE.to_string(),
            },
            description: String::new(),
            schema: serde_json::json!({}),
            read_only: None,
            requires_approval: None,
            event: Some(HookEvent::PostTurn.as_str().to_string()),
            r#match: Some("*".to_string()),
            mode: Some(HookMode::Observe.as_str().to_string()),
        }]);
        let registry = PythonHandlerRegistry::from_manifest(manifest).expect("hook registry");
        Arc::new(PythonHandlerBridge::new(registry))
    }

    async fn compaction_context(
        input_tokens: u64,
        output_tokens: u64,
    ) -> (ExecutionContext, MockLLMBackend) {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        let mut ctx =
            ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam);
        ctx.context_planning = Some(ContextPlanningPolicy {
            tokenizer: ContextTokenizer::O200kBase,
            token_budget: 1_024,
            profiles: BTreeMap::new(),
        });

        let mock = MockLLMBackend::static_response("summary").model_name(COMPACTION_MODEL);
        ctx.llm_registry
            .register(COMPACTION_BACKEND, mock.clone())
            .expect("backend");
        ctx.llm_registry
            .set_model_route(COMPACTION_MODEL, COMPACTION_BACKEND)
            .expect("model route");
        ctx.llm_registry
            .set_default_model(COMPACTION_MODEL)
            .expect("default model");

        let hooks = Arc::new(HookRegistry::new());
        hooks.register(HookBinding {
            handler_id: COMPACTION_HANDLER_ID.to_string(),
            event: HookEvent::PostTurn,
            match_glob: "*".to_string(),
            mode: HookMode::Observe,
        });
        let ctx = ctx
            .with_admitted_invocation_budget(admitted_budget(input_tokens, output_tokens))
            .expect("admitted budget")
            .with_python_handler_bridge(compaction_hook_bridge())
            .with_hook_registry(hooks);
        (ctx, mock)
    }

    #[tokio::test]
    async fn exhausted_admitted_input_budget_blocks_compaction_before_provider_egress() {
        let (ctx, backend) = compaction_context(0, 8).await;

        run_post_turn_hooks(&ctx, "completed reply").await;

        assert_eq!(
            backend.call_count(),
            0,
            "budget rejection precedes provider egress"
        );
        assert_eq!(
            ctx.admitted_invocation_budget
                .as_ref()
                .expect("admitted budget")
                .consumed(),
            (0, 0),
            "a rejected compaction request does not consume capacity"
        );
    }

    #[tokio::test]
    async fn exhausted_admitted_output_budget_blocks_compaction_before_provider_egress() {
        let (ctx, backend) = compaction_context(1_024, 0).await;

        run_post_turn_hooks(&ctx, "completed reply").await;

        assert_eq!(
            backend.call_count(),
            0,
            "budget rejection precedes provider egress"
        );
        assert_eq!(
            ctx.admitted_invocation_budget
                .as_ref()
                .expect("admitted budget")
                .consumed(),
            (0, 0),
            "a rejected compaction request does not consume capacity"
        );
    }

    #[tokio::test]
    async fn compaction_model_call_emits_correlated_redacted_lifecycle_evidence() {
        let (ctx, backend) = compaction_context(1_024, 64).await;
        let recorder = Arc::new(RecordedModelLifecycle::default());
        let ctx = ctx.with_event_emitter(Some(recorder.clone()));

        run_post_turn_hooks(&ctx, "completed reply").await;

        assert_eq!(
            backend.call_count(),
            1,
            "compaction reaches the admitted provider"
        );
        let metrics = recorder.metrics.lock().expect("metrics lock");
        assert_eq!(metrics.len(), 1);
        assert!(matches!(
            metrics[0].call_kind,
            ModelContextCallKind::Compaction
        ));
        let generation = metrics[0]
            .generation
            .as_ref()
            .expect("pre-dispatch context evidence carries a generation identity");
        let usage = recorder.usage.lock().expect("usage lock");
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].generation.as_ref(), Some(generation));
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

    fn python_tool_manifest(entries: &[(&str, Option<bool>, Option<bool>)]) -> String {
        let handlers = entries
            .iter()
            .enumerate()
            .map(|(index, (name, read_only, requires_approval))| {
                json!({
                    "kind": "tool",
                    "language": "python",
                    "handler_id": format!("sha256:{index:064x}"),
                    "module": "policy_fixture",
                    "qualname": name,
                    "name": name,
                    "source": {
                        "artifact_path": format!("handlers/{name}.py"),
                        "content": "def handler(args, ctx):\n    return {}\n"
                    },
                    "schema": {},
                    "read_only": read_only,
                    "requires_approval": requires_approval
                })
            })
            .collect::<Vec<_>>();
        json!({
            "version": "apxm.handler-manifest.v1",
            "handlers": handlers
        })
        .to_string()
    }

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
        let manifest = python_tool_manifest(&[
            ("script-open", Some(true), Some(false)),
            ("script-ask", Some(true), Some(true)),
        ]);
        let ctx = context_with_python_policy(&manifest).await;

        assert_eq!(
            canonical_requires_approval(&ctx, "script-open"),
            Some(false)
        );
        assert_eq!(canonical_requires_approval(&ctx, "script-ask"), Some(true));
    }

    #[tokio::test]
    async fn incomplete_script_policy_is_not_treated_as_open() {
        let manifest = python_tool_manifest(&[("script-open", None, Some(false))]);
        let ctx = context_with_python_policy(&manifest).await;

        assert_eq!(canonical_requires_approval(&ctx, "script-open"), None);
    }
}
