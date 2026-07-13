//! Conversation-memory middleware.
//!
//! After every `ASK` turn, appends the answer to session-scoped STM so the
//! conversation transcript accrues automatically. Keyed by `memory_scope()`
//! (the session id), so turns are readable by the next turn's `qmem` recall.

use crate::executor::handlers::template::input_names_from_node;
use crate::executor::{ExecutionContext, Next, OperationMiddleware, Result};
use crate::memory::MemorySpace;
use apxm_core::events::payload::{TurnBoundaryPayload, TurnDirection};
use apxm_core::types::{
    conversation::TurnInput,
    execution::Node,
    operations::AISOperationType,
    values::{Number, Value},
};
use async_trait::async_trait;
use serde_json::{Map as JsonMap, Value as JsonValue};

/// Session-memory key holding the running turn count.
const TURN_COUNT_KEY: &str = "conversation:turn_count";
/// Prefix for per-turn answer entries (`conversation:turn:<n>`).
const TURN_PREFIX: &str = "conversation:turn:";
/// Session-memory key holding the accumulated conversation-window token
/// estimate. Reset to the post-compaction estimate every time compaction
/// fires, so re-triggering measures against the NEW baseline, not the
/// pre-compaction total.
const TOKEN_ESTIMATE_KEY: &str = "conversation:token_estimate";
/// Node attributes the frontend stamps on the marked conversational-turn ASK
/// carrying the `CompactionPolicy` (`conversational.py::_build_turn_flow`).
/// Absent entirely is the opt-out dial.
const COMPACTION_AT_TOKENS_ATTR: &str = "compaction_at_tokens";
const COMPACTION_KEEP_RECENT_ATTR: &str = "compaction_keep_recent";
const COMPACTION_SUMMARY_KEY_ATTR: &str = "compaction_summary_key";
/// Fail-closed override-precedence signal stamped by the frontend when the
/// program ALSO registers a `post_turn` hook — the runtime default must do
/// nothing (the runtime compaction invariant: "Fail-closed precedence").
const COMPACTION_OVERRIDE_PRESENT_ATTR: &str = "compaction_override_present";
/// Default rolling-summary key when the policy did not set one explicitly —
/// matches `CompactionPolicy.summary_key`'s dataclass default.
const DEFAULT_SUMMARY_KEY: &str = "conversation:summary";
/// Node attribute the frontend stamps on the *top-level* conversational turn
/// `ASK` (`ConversationalAgent._build_turn_flow`). It scopes turn accounting
/// and lifecycle hooks to the user-facing turn, so sub-agent `ASK`s (which
/// inherit the parent `session_id`, and thus the same `memory_scope`) do not
/// inflate `conversation:turn_count`, pollute the recall window, or fire
/// `pre_turn`/`post_turn`/`post_ask` hooks. The program declares the turn
/// (constitution #2: program owns cognition); absent the marker, the ask is
/// not a conversational turn.
const TURN_MARKER_KEY: &str = "conversational_turn";
/// The conversational frontends bind the host turn input under this parameter
/// name on the marked top-level turn ask.
const TURN_INPUT_PARAM_NAME: &str = "user_message";

/// Records each ASK answer into session memory so conversation history accrues
/// without the program threading a transcript.
#[derive(Debug, Clone, Default)]
pub struct ConversationMemoryMiddleware;

impl ConversationMemoryMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl OperationMiddleware for ConversationMemoryMiddleware {
    fn name(&self) -> &str {
        "conversation-memory"
    }

    /// Only the top-level conversational ASK turn accrues history and fires
    /// turn hooks. Sub-agent `ASK`s share the session `memory_scope` (they
    /// inherit `session_id`), so without the marker gate they would inflate the
    /// turn count and re-fire lifecycle hooks. The marker is stamped by the
    /// frontend on the turn flow's ask.
    fn applies_to(&self, node: &Node) -> bool {
        node.op_type == AISOperationType::Ask
            && node
                .attributes
                .get(TURN_MARKER_KEY)
                .and_then(|v| v.as_str())
                == Some("true")
    }

    async fn around(
        &self,
        ctx: &ExecutionContext,
        node: &Node,
        inputs: Vec<Value>,
        next: Next<'_>,
    ) -> Result<Value> {
        let (inputs, turn_context) = Self::normalize_turn_input(node, inputs)?;
        let scope = ctx.memory_scope().to_string();
        let mem = ctx.memory();
        let next_stored_turn = mem
            .read_scoped(MemorySpace::Stm, &scope, TURN_COUNT_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|value| value.as_i64())
            .unwrap_or(0)
            + 1;
        let turn_number = ctx
            .session_ledger()
            .map(|ledger| ledger.turns_used())
            .filter(|turn| *turn > 0)
            .unwrap_or(next_stored_turn.max(1) as usize);
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_turn_boundary(TurnBoundaryPayload {
                turn_number,
                direction: TurnDirection::Request,
            });
        }

        // pre_turn hooks fire before the turn's ask (gate-capable → fail-closed).
        let supplement =
            crate::executor::hook_driver::run_pre_turn_hooks(ctx, turn_context).await?;
        if let Some(text) = supplement {
            *ctx.pending_turn_prompt_supplement.write() = Some(text);
        }
        let result = next.run(ctx, node, inputs).await;
        if let Ok(Value::String(answer)) = &result {
            // post_ask + post_turn hooks fire with the reply (observe;).
            crate::executor::hook_driver::run_post_ask_hooks(ctx, answer).await;
            crate::executor::hook_driver::run_post_turn_hooks(ctx, answer).await;
            // Best-effort: a memory write failure must not fail the turn.
            let _ = mem
                .write_scoped(
                    MemorySpace::Stm,
                    &scope,
                    TURN_COUNT_KEY.to_string(),
                    Value::Number(Number::Integer(next_stored_turn)),
                )
                .await;
            let _ = mem
                .write_scoped(
                    MemorySpace::Stm,
                    &scope,
                    format!("{TURN_PREFIX}{next_stored_turn}"),
                    Value::String(answer.clone()),
                )
                .await;

            // Conversation-window compaction (the runtime compaction mechanism): the SAME turn-scoped
            // choke point measures the budget and, absent an author
            // override, folds older turns into the rolling summary. A
            // no-op when the policy is absent (opt-out dial).
            Self::maybe_compact(ctx, node, &scope, answer, next_stored_turn).await;
            if let Some(emitter) = &ctx.event_emitter {
                emitter.emit_turn_boundary(TurnBoundaryPayload {
                    turn_number,
                    direction: TurnDirection::Response,
                });
            }
        }
        result
    }
}

impl ConversationMemoryMiddleware {
    fn normalize_turn_input(
        node: &Node,
        mut inputs: Vec<Value>,
    ) -> Result<(Vec<Value>, Option<JsonMap<String, JsonValue>>)> {
        let Some(index) = Self::turn_input_index(node, &inputs) else {
            return Ok((inputs, None));
        };
        let Some(raw_turn_input) = inputs.get(index).cloned() else {
            return Ok((inputs, None));
        };
        let turn_input = TurnInput::try_from(raw_turn_input).map_err(|error| {
            apxm_core::error::RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("invalid conversational turn input: {error}"),
            }
        })?;
        inputs[index] = Value::String(turn_input.message);
        Ok((inputs, turn_input.context))
    }

    fn turn_input_index(node: &Node, inputs: &[Value]) -> Option<usize> {
        let input_names = input_names_from_node(node);
        if let Some(index) = input_names
            .iter()
            .position(|name| name == TURN_INPUT_PARAM_NAME)
        {
            return (index < inputs.len()).then_some(index);
        }
        if inputs.len() == 1 || input_names.len() == 1 {
            return (!inputs.is_empty()).then_some(0);
        }

        let mut matches = inputs.iter().enumerate().filter_map(|(index, value)| {
            Self::looks_like_turn_input_envelope(value).then_some(index)
        });
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    fn looks_like_turn_input_envelope(value: &Value) -> bool {
        matches!(value, Value::Object(fields) if fields.contains_key("message"))
    }

    /// Conversation-window compaction — the four control dials
    /// (default/configure/override/opt-out), all driven off the SAME
    /// node attributes the frontend stamps on the marked conversational-turn
    /// ASK (`conversational.py::_build_turn_flow`). Reuses
    /// `context_stack::estimate_tokens`/`truncate_to_budget` (the shipped
    /// subagent prompt-budget primitive) rather than re-deriving a chars/4
    /// heuristic, and the SAME direct-LLM-call primitive
    /// `hook_driver::host_llm_ask` uses (the session's own backend/router).
    async fn maybe_compact(
        ctx: &ExecutionContext,
        node: &Node,
        scope: &str,
        answer: &str,
        turn_no: i64,
    ) {
        // Opt-out dial: the policy is absent entirely => zero extra
        // measurement, zero events, zero extra `count_tokens`-equivalent
        // calls attributable to compaction.
        let Some(compact_at_tokens_raw) = node
            .attributes
            .get(COMPACTION_AT_TOKENS_ATTR)
            .and_then(|v| v.as_i64())
        else {
            return;
        };
        // Negative — malformed input: a non-positive budget is a
        // documented no-op, never a panic or a turn-failing error.
        if compact_at_tokens_raw <= 0 {
            return;
        }
        let compact_at_tokens = compact_at_tokens_raw as usize;

        // Fail-closed precedence: an author `post_turn` hook already owns
        // compaction — the runtime default must never double-compact.
        let override_present = node
            .attributes
            .get(COMPACTION_OVERRIDE_PRESENT_ATTR)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if override_present {
            return;
        }

        let keep_recent = node
            .attributes
            .get(COMPACTION_KEEP_RECENT_ATTR)
            .and_then(|v| v.as_i64())
            .filter(|v| *v > 0)
            .unwrap_or(
                apxm_core::constants::runtime::conversation_compaction::DEFAULT_KEEP_RECENT_TURNS
                    as i64,
            );
        let summary_key = node
            .attributes
            .get(COMPACTION_SUMMARY_KEY_ATTR)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| DEFAULT_SUMMARY_KEY.to_string());

        let mem = ctx.memory();
        let added = crate::context_stack::estimate_tokens(answer);
        let prior_tokens = mem
            .read_scoped(MemorySpace::Stm, scope, TOKEN_ESTIMATE_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0) as usize;
        let current_tokens = prior_tokens + added;
        let _ = mem
            .write_scoped(
                MemorySpace::Stm,
                scope,
                TOKEN_ESTIMATE_KEY.to_string(),
                Value::Number(Number::Integer(current_tokens as i64)),
            )
            .await;

        let utilization_pct = (current_tokens as f64 / compact_at_tokens as f64) * 100.0;
        if utilization_pct
            >= apxm_core::constants::runtime::conversation_compaction::DEFAULT_WARNING_UTILIZATION_PCT
            && let Some(emitter) = &ctx.event_emitter
        {
            emitter.emit_context_window_warning(current_tokens, compact_at_tokens, utilization_pct);
        }

        if current_tokens < compact_at_tokens {
            return;
        }

        // Crossed: fold every turn OLDER than the kept window into the
        // rolling summary. Nothing old enough to fold yet is a no-op (never
        // fires a compaction with no content to fold).
        let fold_through = turn_no - keep_recent;
        if fold_through < 1 {
            return;
        }

        let mut folded_text = String::new();
        if let Ok(Some(prior_summary)) =
            mem.read_scoped(MemorySpace::Stm, scope, &summary_key).await
            && let Some(s) = prior_summary.as_str()
        {
            folded_text.push_str(s);
            folded_text.push('\n');
        }
        for i in 1..=fold_through {
            if let Ok(Some(v)) = mem
                .read_scoped(MemorySpace::Stm, scope, &format!("{TURN_PREFIX}{i}"))
                .await
                && let Some(s) = v.as_str()
            {
                folded_text.push_str(s);
                folded_text.push('\n');
            }
        }
        if folded_text.trim().is_empty() {
            return;
        }

        let prompt = format!(
            "Summarize the following conversation excerpt into a concise running \
             summary that preserves decisions, facts, names, and open tasks. Be \
             terse.\n\n{folded_text}"
        );
        let request =
            apxm_backends::LLMRequest::new(prompt).with_operation_type(AISOperationType::Ask);
        let summary_result = if let Some(router) = &ctx.model_router {
            router.generate(request).await
        } else {
            ctx.llm_registry.generate(request).await
        };
        // Fail-open budget enforcement is a correctness bug:
        // a summarization failure degrades to a warning, never panics or
        // fails the turn, and never silently no-ops without a trace.
        let summary = match summary_result {
            Ok(response) => response.content,
            Err(e) => {
                if let Some(emitter) = &ctx.event_emitter {
                    emitter.emit_warning(
                        "compaction_summarize_failed",
                        &format!("conversation compaction summarization call failed: {e}"),
                    );
                }
                return;
            }
        };

        let new_summary_tokens = crate::context_stack::estimate_tokens(&summary);
        let mut kept_tokens = 0usize;
        for i in (fold_through + 1)..=turn_no {
            if let Ok(Some(v)) = mem
                .read_scoped(MemorySpace::Stm, scope, &format!("{TURN_PREFIX}{i}"))
                .await
                && let Some(s) = v.as_str()
            {
                kept_tokens += crate::context_stack::estimate_tokens(s);
            }
        }
        let new_tokens = new_summary_tokens + kept_tokens;

        // Write the folded summary to STM (the live recall path,
        // `recall_pin` in `qmem.rs`) AND to LTM as a durable copy of the
        // SAME logical key. STM is deliberately volatile
        // (`ShortTermMemory` doc comment) — a process crash/restart between
        // this compaction and the next turn must not silently drop the
        // summary. `MemorySystem::recent_scoped` falls back a missing pin
        // from STM to its LTM copy, so a fresh (post-restart) STM still
        // surfaces it — see the crash/restart recovery test.
        let _ = mem
            .write_scoped(
                MemorySpace::Stm,
                scope,
                summary_key.clone(),
                Value::String(summary.clone()),
            )
            .await;
        let _ = mem
            .write_scoped(
                MemorySpace::Ltm,
                scope,
                summary_key.clone(),
                Value::String(summary),
            )
            .await;
        let _ = mem
            .write_scoped(
                MemorySpace::Stm,
                scope,
                TOKEN_ESTIMATE_KEY.to_string(),
                Value::Number(Number::Integer(new_tokens as i64)),
            )
            .await;

        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_context_compacted(current_tokens, new_tokens);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::executor::events::ExecutionEventEmitter;
    use crate::executor::hooks::{HookBinding, HookEvent, HookMode, HookRegistry};
    use crate::executor::middleware::{BoxFuture, Next};
    use crate::executor::session_ledger::SessionLedger;
    use crate::executor::{ExecutionContext, OperationMiddleware};
    use crate::memory::{MemoryConfig, MemorySystem};
    use crate::typescript_tools::{TypeScriptHandlerBridge, TypeScriptHandlerRegistry};
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::error::RuntimeError;
    use apxm_core::types::conversation::TurnInput;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};

    fn ask(marked: bool) -> Node {
        ask_with_input_names(marked, &[])
    }

    fn ask_with_input_names(marked: bool, input_names: &[&str]) -> Node {
        let mut node = Node::new(1, AISOperationType::Ask);
        if marked {
            node.set_attribute(TURN_MARKER_KEY.to_string(), Value::String("true".into()));
        }
        if !input_names.is_empty() {
            node.set_attribute(
                graph_attrs::INPUT_NAMES.to_string(),
                Value::Array(
                    input_names
                        .iter()
                        .map(|name| Value::String((*name).to_string()))
                        .collect(),
                ),
            );
        }
        node
    }

    static TERMINAL_CAPTURE_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug, Clone, PartialEq)]
    struct TerminalCapture {
        inputs: Vec<Value>,
        supplement: Option<String>,
    }

    fn terminal_capture_slot() -> &'static Mutex<Option<TerminalCapture>> {
        static SLOT: OnceLock<Mutex<Option<TerminalCapture>>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(None))
    }

    fn clear_terminal_capture() {
        *terminal_capture_slot().lock().unwrap() = None;
    }

    fn take_terminal_capture() -> TerminalCapture {
        terminal_capture_slot()
            .lock()
            .unwrap()
            .take()
            .expect("terminal capture should be populated")
    }

    #[test]
    fn applies_only_to_marked_conversational_turn() {
        let mw = ConversationMemoryMiddleware::new();
        // The top-level turn ask carries the marker the frontend stamps.
        assert!(mw.applies_to(&ask(true)));
        // A sub-agent ask shares the session scope but is unmarked: it must NOT
        // accrue history or fire turn hooks.
        assert!(!mw.applies_to(&ask(false)));
        // Non-ask ops never apply.
        let inv = Node::new(2, AISOperationType::InvCap);
        assert!(!mw.applies_to(&inv));
    }

    #[derive(Clone, Default)]
    struct BoundaryCapturingEmitter {
        boundaries: Arc<Mutex<Vec<TurnBoundaryPayload>>>,
    }

    impl ExecutionEventEmitter for BoundaryCapturingEmitter {
        fn emit_llm_token(&self, _content: &str) {}
        fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}
        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_turn_boundary(&self, payload: TurnBoundaryPayload) {
            self.boundaries.lock().unwrap().push(payload);
        }
    }

    async fn test_context(
        emitter: Arc<BoundaryCapturingEmitter>,
        ledger: Arc<SessionLedger>,
    ) -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam)
            .with_session_id("boundary-session".to_string())
            .with_session_ledger(ledger)
            .with_event_emitter(Some(emitter as Arc<dyn ExecutionEventEmitter>))
    }

    fn success_terminal<'a>(
        _ctx: &'a ExecutionContext,
        _node: &'a Node,
        _inputs: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value>> {
        Box::pin(async { Ok(Value::String("answer".to_string())) })
    }

    fn failure_terminal<'a>(
        _ctx: &'a ExecutionContext,
        node: &'a Node,
        _inputs: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value>> {
        Box::pin(async move {
            Err(RuntimeError::Operation {
                op_type: node.op_type,
                message: "turn failed".to_string(),
            })
        })
    }

    fn capturing_terminal<'a>(
        ctx: &'a ExecutionContext,
        _node: &'a Node,
        inputs: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value>> {
        Box::pin(async move {
            *terminal_capture_slot().lock().unwrap() = Some(TerminalCapture {
                inputs: inputs.clone(),
                supplement: ctx.pending_turn_prompt_supplement.read().clone(),
            });
            Ok(Value::String("answer".to_string()))
        })
    }

    fn node_available() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_ok()
    }

    async fn context_with_typescript_pre_turn_hook(
        hook_source: &str,
    ) -> (tempfile::TempDir, ExecutionContext) {
        let temp = tempfile::tempdir().expect("tempdir");
        let hook_path = temp.path().join("pre_turn_hook.mjs");
        std::fs::write(&hook_path, hook_source).expect("write hook source");
        let registry = TypeScriptHandlerRegistry::from_json(
            &json!([{
                "handler_id": "sha256:pre-turn-hook",
                "module": "hook_mod",
                "qualname": "capture",
                "name": "hook",
                "schema": {},
                "source_file": hook_path.to_string_lossy().into_owned(),
            }])
            .to_string(),
        )
        .expect("typescript hook manifest");

        let hook_registry = Arc::new(HookRegistry::new());
        hook_registry.register(HookBinding {
            handler_id: "sha256:pre-turn-hook".to_string(),
            event: HookEvent::PreTurn,
            match_glob: "*".to_string(),
            mode: HookMode::Observe,
        });

        let emitter = Arc::new(BoundaryCapturingEmitter::default());
        let ledger = Arc::new(SessionLedger::new(None, HashMap::new()));
        let ctx = test_context(emitter, ledger)
            .await
            .with_typescript_handler_bridge(Arc::new(TypeScriptHandlerBridge::new(registry)))
            .with_hook_registry(hook_registry);
        (temp, ctx)
    }

    #[tokio::test]
    async fn emits_numbered_request_and_response_boundaries_per_successful_turn() {
        let emitter = Arc::new(BoundaryCapturingEmitter::default());
        let ledger = Arc::new(SessionLedger::new(None, HashMap::new()));
        let ctx = test_context(emitter.clone(), ledger.clone()).await;
        let middleware = ConversationMemoryMiddleware::new();
        let node = ask(true);

        for expected_turn in 1..=2 {
            assert_eq!(ledger.charge_turn().expect("charge turn"), expected_turn);
            let result = middleware
                .around(
                    &ctx,
                    &node,
                    Vec::new(),
                    Next {
                        chain: &[],
                        idx: 0,
                        terminal: success_terminal,
                    },
                )
                .await
                .expect("turn succeeds");
            assert_eq!(result, Value::String("answer".to_string()));
        }

        let boundaries = emitter.boundaries.lock().unwrap().clone();
        assert_eq!(boundaries.len(), 4);
        assert_eq!(boundaries[0].turn_number, 1);
        assert_eq!(boundaries[0].direction, TurnDirection::Request);
        assert_eq!(boundaries[1].turn_number, 1);
        assert_eq!(boundaries[1].direction, TurnDirection::Response);
        assert_eq!(boundaries[2].turn_number, 2);
        assert_eq!(boundaries[2].direction, TurnDirection::Request);
        assert_eq!(boundaries[3].turn_number, 2);
        assert_eq!(boundaries[3].direction, TurnDirection::Response);
    }

    #[tokio::test]
    async fn failed_turn_emits_request_boundary_without_false_response() {
        let emitter = Arc::new(BoundaryCapturingEmitter::default());
        let ledger = Arc::new(SessionLedger::new(None, HashMap::new()));
        assert_eq!(ledger.charge_turn().expect("charge turn"), 1);
        let ctx = test_context(emitter.clone(), ledger).await;
        let middleware = ConversationMemoryMiddleware::new();

        let result = middleware
            .around(
                &ctx,
                &ask(true),
                Vec::new(),
                Next {
                    chain: &[],
                    idx: 0,
                    terminal: failure_terminal,
                },
            )
            .await;
        assert!(result.is_err());

        let boundaries = emitter.boundaries.lock().unwrap().clone();
        assert_eq!(boundaries.len(), 1);
        assert_eq!(boundaries[0].turn_number, 1);
        assert_eq!(boundaries[0].direction, TurnDirection::Request);
    }

    #[tokio::test]
    async fn legacy_string_turn_input_is_unchanged_for_existing_ask_path() {
        let _guard = TERMINAL_CAPTURE_LOCK.lock().unwrap();
        clear_terminal_capture();

        let emitter = Arc::new(BoundaryCapturingEmitter::default());
        let ledger = Arc::new(SessionLedger::new(None, HashMap::new()));
        let ctx = test_context(emitter, ledger).await;
        let middleware = ConversationMemoryMiddleware::new();
        let node = ask_with_input_names(true, &[TURN_INPUT_PARAM_NAME, "history"]);
        let original_inputs = vec![
            Value::String("hello".to_string()),
            Value::String("recalled context".to_string()),
        ];

        let result = middleware
            .around(
                &ctx,
                &node,
                original_inputs.clone(),
                Next {
                    chain: &[],
                    idx: 0,
                    terminal: capturing_terminal,
                },
            )
            .await
            .expect("turn succeeds");

        assert_eq!(result, Value::String("answer".to_string()));
        let capture = take_terminal_capture();
        assert_eq!(capture.inputs, original_inputs);
        assert_eq!(capture.supplement, None);
    }

    #[tokio::test]
    async fn structured_turn_input_context_reaches_pre_turn_hook_and_message_is_normalized() {
        if !node_available() {
            return;
        }

        let _guard = TERMINAL_CAPTURE_LOCK.lock().unwrap();
        clear_terminal_capture();

        let (_temp, ctx) = context_with_typescript_pre_turn_hook(
            r#"
export function capture(ctx) {
  return ctx.prependSystem(String(ctx.context?.topic ?? "missing"));
}
"#,
        )
        .await;
        let middleware = ConversationMemoryMiddleware::new();
        let node = ask_with_input_names(true, &[TURN_INPUT_PARAM_NAME, "history"]);
        let turn_input = TurnInput {
            message: "hello".to_string(),
            context: Some(JsonMap::from_iter([(
                "topic".to_string(),
                json!("priority-sync"),
            )])),
        };

        let result = middleware
            .around(
                &ctx,
                &node,
                vec![
                    Value::from(turn_input),
                    Value::String("recalled context".to_string()),
                ],
                Next {
                    chain: &[],
                    idx: 0,
                    terminal: capturing_terminal,
                },
            )
            .await
            .expect("turn succeeds");

        assert_eq!(result, Value::String("answer".to_string()));
        let capture = take_terminal_capture();
        assert_eq!(
            capture.inputs,
            vec![
                Value::String("hello".to_string()),
                Value::String("recalled context".to_string()),
            ]
        );
        assert_eq!(capture.supplement.as_deref(), Some("priority-sync"));
    }
}

/// Conversation-window compaction — the four control dials
/// (default/configure/override/opt-out), each independently tested, plus
/// the negative and degraded-input cases.
#[cfg(test)]
mod compaction_tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::executor::ExecutionContext;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_backends::llm::backends::MockLLMBackend;
    use std::sync::{Arc, Mutex};

    fn ask(marked: bool) -> Node {
        let mut node = Node::new(1, AISOperationType::Ask);
        if marked {
            node.set_attribute(TURN_MARKER_KEY.to_string(), Value::String("true".into()));
        }
        node
    }

    /// Captures `context_window_warning`/`context_compacted`/`emit_warning`
    /// calls; every other `ExecutionEventEmitter` method uses the trait's
    /// default no-op (same pattern as `engine.rs`'s `WarningCapturingEmitter`).
    #[derive(Default, Clone)]
    struct CompactionCapturingEmitter {
        window_warnings: Arc<Mutex<Vec<(usize, usize, f64)>>>,
        compacted: Arc<Mutex<Vec<(usize, usize)>>>,
        warnings: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl crate::executor::events::ExecutionEventEmitter for CompactionCapturingEmitter {
        fn emit_llm_token(&self, _content: &str) {}
        fn emit_tool_start(&self, _name: &str, _args: &std::collections::HashMap<String, Value>) {}
        fn emit_tool_end(&self, _name: &str, _result: &Value) {}
        fn emit_context_window_warning(
            &self,
            current_tokens: usize,
            max_tokens: usize,
            utilization_pct: f64,
        ) {
            self.window_warnings.lock().unwrap().push((
                current_tokens,
                max_tokens,
                utilization_pct,
            ));
        }
        fn emit_context_compacted(&self, original_tokens: usize, new_tokens: usize) {
            self.compacted
                .lock()
                .unwrap()
                .push((original_tokens, new_tokens));
        }
        fn emit_warning(&self, code: &str, message: &str) {
            self.warnings
                .lock()
                .unwrap()
                .push((code.to_string(), message.to_string()));
        }
    }

    async fn test_ctx(emitter: Arc<CompactionCapturingEmitter>) -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        let llm_registry = Arc::new(LLMRegistry::new());
        llm_registry
            .register("mock", MockLLMBackend::static_response("ROLLING SUMMARY"))
            .expect("register mock backend");
        llm_registry
            .set_default("mock")
            .expect("set default backend");

        ExecutionContext::new(memory, llm_registry, capability_system, aam).with_event_emitter(
            Some(emitter as Arc<dyn crate::executor::events::ExecutionEventEmitter>),
        )
    }

    fn marked_ask_with_compaction(
        compact_at_tokens: i64,
        keep_recent: i64,
        override_present: bool,
    ) -> Node {
        let mut node = Node::new(1, AISOperationType::Ask);
        node.set_attribute(TURN_MARKER_KEY.to_string(), Value::String("true".into()));
        node.set_attribute(
            COMPACTION_AT_TOKENS_ATTR.to_string(),
            Value::Number(Number::Integer(compact_at_tokens)),
        );
        node.set_attribute(
            COMPACTION_KEEP_RECENT_ATTR.to_string(),
            Value::Number(Number::Integer(keep_recent)),
        );
        node.set_attribute(
            COMPACTION_OVERRIDE_PRESENT_ATTR.to_string(),
            Value::Bool(override_present),
        );
        node
    }

    /// Simulate N turns: writes `conversation:turn:<n>` the same way
    /// `around()` does, then calls `maybe_compact` — the exact sequence the
    /// middleware runs per turn.
    async fn run_turns(ctx: &ExecutionContext, node: &Node, scope: &str, answers: &[&str]) {
        for (i, answer) in answers.iter().enumerate() {
            let turn_no = (i + 1) as i64;
            ctx.memory()
                .write_scoped(
                    MemorySpace::Stm,
                    scope,
                    format!("{TURN_PREFIX}{turn_no}"),
                    Value::String(answer.to_string()),
                )
                .await
                .unwrap();
            ConversationMemoryMiddleware::maybe_compact(ctx, node, scope, answer, turn_no).await;
        }
    }

    /// **Opt-out dial:** `compaction=None` (no attributes on the node at
    /// all) is a TRUE no-op — zero events, zero memory writes attributable
    /// to compaction, regardless of how much text flows through.
    #[tokio::test]
    async fn opt_out_dial_is_a_true_noop() {
        let emitter = Arc::new(CompactionCapturingEmitter::default());
        let ctx = test_ctx(emitter.clone()).await;
        let node = ask(true); // no compaction attributes at all
        let scope = "sess-opt-out";
        let big_answer = "x ".repeat(5_000);

        run_turns(&ctx, &node, scope, &[&big_answer, &big_answer, &big_answer]).await;

        assert!(emitter.window_warnings.lock().unwrap().is_empty());
        assert!(emitter.compacted.lock().unwrap().is_empty());
        assert!(emitter.warnings.lock().unwrap().is_empty());
        assert!(
            ctx.memory()
                .read_scoped(MemorySpace::Stm, scope, TOKEN_ESTIMATE_KEY)
                .await
                .unwrap()
                .is_none(),
            "opt-out must not even start measuring"
        );
    }

    /// **Default + configure dials:** an explicit policy crossing
    /// `compact_at_tokens` emits `context_window_warning`
    /// (`current_tokens/max_tokens/utilization_pct`) then `context_compacted`
    /// (`original_tokens > new_tokens`); the folded summary is written to
    /// `summary_key` so the next turn's `recall_pin` surfaces it ahead of the
    /// recency window (`recent_scoped_surfaces_compaction_summary_outside_window`
    /// already proves that half of the mechanism; this proves compaction is
    /// the thing that populates the pinned key).
    #[tokio::test]
    async fn default_dial_emits_warning_then_compacted_and_folds_summary() {
        let emitter = Arc::new(CompactionCapturingEmitter::default());
        let ctx = test_ctx(emitter.clone()).await;
        // keep_recent=2, compact_at_tokens=300 — the record's exact "Default" fixture.
        let node = marked_ask_with_compaction(300, 2, false);
        let scope = "sess-default";

        // Each answer is long enough that a handful of turns crosses 300
        // tokens (real bpe tokenizer, not chars/4).
        let answer = "The quarterly report shows revenue growth across every region and segment. "
            .repeat(20);
        run_turns(&ctx, &node, scope, &[&answer, &answer, &answer, &answer]).await;

        let warnings = emitter.window_warnings.lock().unwrap().clone();
        assert!(
            !warnings.is_empty(),
            "must warn before/at the crossing turn"
        );
        for (current, max, pct) in &warnings {
            assert_eq!(*max, 300);
            assert!(*current > 0);
            assert!(*pct >= 80.0);
        }

        let compacted = emitter.compacted.lock().unwrap().clone();
        assert!(
            !compacted.is_empty(),
            "must compact at least once across a session that keeps crossing the budget"
        );
        for (original_tokens, new_tokens) in &compacted {
            assert!(
                original_tokens > new_tokens,
                "compaction must reduce the token estimate: {original_tokens} vs {new_tokens}"
            );
        }

        // The folded summary is readable at the configured summary_key —
        // the SAME key `recall_pin` (`qmem.rs`) reads ahead of the window.
        let summary = ctx
            .memory()
            .read_scoped(MemorySpace::Stm, scope, DEFAULT_SUMMARY_KEY)
            .await
            .unwrap()
            .and_then(|v| v.as_str().map(str::to_string));
        assert_eq!(summary.as_deref(), Some("ROLLING SUMMARY"));

        // `recent_scoped` (the runtime's real recall path) surfaces the
        // pinned summary ahead of the recency window.
        let recalled = ctx
            .memory()
            .recent_scoped(
                MemorySpace::Stm,
                scope,
                TURN_PREFIX,
                2,
                &[DEFAULT_SUMMARY_KEY.to_string()],
            )
            .await
            .unwrap();
        assert_eq!(recalled[0].key, DEFAULT_SUMMARY_KEY);
        assert_eq!(recalled[0].value.as_str(), Some("ROLLING SUMMARY"));
    }

    /// **Configure dial:** a DIFFERENT `keep_recent`/`compact_at_tokens`
    /// changes the crossing point — asserted on `original_tokens`, not just
    /// "compaction happened".
    #[tokio::test]
    async fn configure_dial_changes_trigger_point_reflected_in_payload_tokens() {
        let answer = "The quarterly report shows revenue growth across every region and segment. "
            .repeat(20);

        // Tight budget: crosses (and reports) at a small original_tokens.
        let emitter_tight = Arc::new(CompactionCapturingEmitter::default());
        let ctx_tight = test_ctx(emitter_tight.clone()).await;
        let node_tight = marked_ask_with_compaction(150, 1, false);
        run_turns(&ctx_tight, &node_tight, "sess-tight", &[&answer, &answer]).await;
        let tight_original = emitter_tight.compacted.lock().unwrap()[0].0;

        // Looser budget + larger keep_recent: needs more turns / a larger
        // original_tokens before the same content crosses.
        let emitter_loose = Arc::new(CompactionCapturingEmitter::default());
        let ctx_loose = test_ctx(emitter_loose.clone()).await;
        let node_loose = marked_ask_with_compaction(900, 1, false);
        run_turns(
            &ctx_loose,
            &node_loose,
            "sess-loose",
            &[&answer, &answer, &answer, &answer, &answer, &answer],
        )
        .await;
        let loose_compacted = emitter_loose.compacted.lock().unwrap().clone();
        assert_eq!(loose_compacted.len(), 1);
        let loose_original = loose_compacted[0].0;

        assert!(
            loose_original > tight_original,
            "a higher compact_at_tokens must accumulate more original_tokens \
             before triggering: tight={tight_original} loose={loose_original}"
        );
    }

    /// **Override dial:** `compaction_override_present=true` (the frontend's
    /// fail-closed signal that a `post_turn` hook already owns compaction)
    /// makes the runtime default a complete no-op — zero events, zero LLM
    /// calls — never a second (double) compaction path.
    #[tokio::test]
    async fn override_dial_disables_runtime_default_entirely() {
        let emitter = Arc::new(CompactionCapturingEmitter::default());
        let ctx = test_ctx(emitter.clone()).await;
        let node = marked_ask_with_compaction(300, 2, /* override_present */ true);
        let scope = "sess-override";
        let answer = "The quarterly report shows revenue growth across every region and segment. "
            .repeat(20);

        run_turns(&ctx, &node, scope, &[&answer, &answer, &answer, &answer]).await;

        assert!(emitter.window_warnings.lock().unwrap().is_empty());
        assert!(emitter.compacted.lock().unwrap().is_empty());
        assert!(
            ctx.memory()
                .read_scoped(MemorySpace::Stm, scope, DEFAULT_SUMMARY_KEY)
                .await
                .unwrap()
                .is_none(),
            "override present: the runtime must never write a summary itself"
        );
    }

    /// **Negative — malformed input:** a zero or negative `compact_at_tokens`
    /// is a documented no-op — never a panic, never a turn failure.
    #[tokio::test]
    async fn negative_zero_or_negative_compact_at_tokens_is_a_documented_noop() {
        for bad_budget in [0_i64, -5_i64] {
            let emitter = Arc::new(CompactionCapturingEmitter::default());
            let ctx = test_ctx(emitter.clone()).await;
            let node = marked_ask_with_compaction(bad_budget, 2, false);
            let scope = format!("sess-malformed-{bad_budget}");

            run_turns(&ctx, &node, &scope, &["some answer text"]).await;

            assert!(emitter.window_warnings.lock().unwrap().is_empty());
            assert!(emitter.compacted.lock().unwrap().is_empty());
            assert!(emitter.warnings.lock().unwrap().is_empty());
        }
    }

    /// **Negative — summarization failure degrades gracefully:** fail-open
    /// budget enforcement (silently skipping the check) is a correctness bug
    /// by the runtime invariant; a failed LLM call must degrade to
    /// `emit_warning` and skip folding, never panic, never emit a bogus
    /// `context_compacted`.
    #[tokio::test]
    async fn negative_summarization_failure_degrades_to_warning_without_panicking() {
        let emitter = Arc::new(CompactionCapturingEmitter::default());
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        let llm_registry = Arc::new(LLMRegistry::new());
        llm_registry
            .register(
                "mock",
                MockLLMBackend::new().always_fail("simulated backend outage"),
            )
            .expect("register failing mock backend");
        llm_registry
            .set_default("mock")
            .expect("set default backend");
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam)
            .with_event_emitter(Some(
                emitter.clone() as Arc<dyn crate::executor::events::ExecutionEventEmitter>
            ));

        let node = marked_ask_with_compaction(300, 2, false);
        let scope = "sess-llm-failure";
        let answer = "The quarterly report shows revenue growth across every region and segment. "
            .repeat(20);

        // Must not panic even though every summarization call fails.
        run_turns(&ctx, &node, scope, &[&answer, &answer, &answer, &answer]).await;

        assert!(
            emitter.compacted.lock().unwrap().is_empty(),
            "a failed summarization must never emit a bogus context_compacted"
        );
        let warnings = emitter.warnings.lock().unwrap().clone();
        assert!(
            warnings
                .iter()
                .any(|(code, _)| code == "compaction_summarize_failed"),
            "must degrade to a warning: {warnings:?}"
        );
        assert!(
            ctx.memory()
                .read_scoped(MemorySpace::Stm, scope, DEFAULT_SUMMARY_KEY)
                .await
                .unwrap()
                .is_none(),
            "a failed summarization must not write a partial/stale summary"
        );
    }

    /// **Parity/deletion gate:** the runtime default's built-in fallback
    /// budget/keep-window matches the retired `apxm-machine-ais` CLI
    /// duplicate (`chat.rs`'s `KEEP_RECENT_TURNS=4`/`COMPACT_AT_TOKENS=20_000`,
    /// deleted in the same change once this parity held) — this test stays
    /// in the suite, retargeted at the runtime default, per
    /// the runtime compaction invariant.
    #[test]
    fn runtime_default_matches_retired_cli_duplicate_constants() {
        use apxm_core::constants::runtime::conversation_compaction::{
            DEFAULT_COMPACT_AT_TOKENS, DEFAULT_KEEP_RECENT_TURNS,
        };
        // Mirrors the deleted `apxm-machine-ais::chat::{KEEP_RECENT_TURNS,
        // COMPACT_AT_TOKENS}` values exactly — the parity this WP's
        // deletion gate proved before the CLI duplicate was removed.
        assert_eq!(DEFAULT_KEEP_RECENT_TURNS, 4);
        assert_eq!(DEFAULT_COMPACT_AT_TOKENS, 20_000);
    }

    /// **Recovery (crash/restart):** a session that emits `context_compacted`
    /// and folds a summary into `summary_key` mid-conversation, then the
    /// process is killed/restarted (not just resumed cleanly) before the
    /// next turn starts — the durable LTM copy (`maybe_compact` writes both
    /// STM and LTM) lets a FRESH `MemorySystem` (new volatile STM, same
    /// on-disk LTM — exactly what a real process restart looks like) still
    /// recall the folded summary via `recall_pin`'s STM→LTM fallback, and
    /// resume never re-runs the compaction that already committed (the
    /// restarted process starts a NEW `maybe_compact` call sequence; it
    /// never replays the one that already fired).
    #[tokio::test]
    async fn recovery_summary_survives_a_simulated_process_restart() {
        let tmp = tempfile::tempdir().expect("tempdir for durable LTM");
        let ltm_path = tmp.path().join("recovery.sqlite");
        let scope = "sess-recover";

        // --- "before the crash": a live process compacts and commits a summary.
        let emitter = Arc::new(CompactionCapturingEmitter::default());
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        let llm_registry = Arc::new(LLMRegistry::new());
        llm_registry
            .register("mock", MockLLMBackend::static_response("ROLLING SUMMARY"))
            .expect("register mock backend");
        llm_registry
            .set_default("mock")
            .expect("set default backend");
        let memory_before = Arc::new(
            MemorySystem::new(MemoryConfig::with_ltm_path(ltm_path.clone()))
                .await
                .expect("durable-LTM memory"),
        );
        let ctx_before = ExecutionContext::new(memory_before, llm_registry, capability_system, aam)
            .with_event_emitter(Some(
                emitter.clone() as Arc<dyn crate::executor::events::ExecutionEventEmitter>
            ));
        let node = marked_ask_with_compaction(300, 2, false);
        let answer = "The quarterly report shows revenue growth across every region and segment. "
            .repeat(20);
        run_turns(
            &ctx_before,
            &node,
            scope,
            &[&answer, &answer, &answer, &answer],
        )
        .await;
        assert!(
            !emitter.compacted.lock().unwrap().is_empty(),
            "setup must actually compact before simulating the crash"
        );

        // --- "kill -9; restart": drop the whole process-side context/memory
        // and build a brand-new `MemorySystem` — a fresh (empty) STM, same
        // on-disk LTM path. No object is shared with `memory_before`.
        drop(ctx_before);
        let memory_after = MemorySystem::new(MemoryConfig::with_ltm_path(ltm_path))
            .await
            .expect("reopen durable LTM after restart");

        // The fresh STM genuinely has nothing — proves the durability below
        // comes from the LTM fallback, not leftover shared state.
        assert!(
            memory_after
                .read_scoped(MemorySpace::Stm, scope, DEFAULT_SUMMARY_KEY)
                .await
                .unwrap()
                .is_none(),
            "a fresh STM must start empty, exactly like a real process restart"
        );

        // The first post-restart turn's recall (the SAME `recent_scoped` +
        // `recall_pin` path `qmem.rs` drives) still surfaces the folded
        // summary — exactly once, ahead of the recency window.
        let recalled = memory_after
            .recent_scoped(
                MemorySpace::Stm,
                scope,
                TURN_PREFIX,
                2,
                &[DEFAULT_SUMMARY_KEY.to_string()],
            )
            .await
            .expect("recall after restart");
        let pinned: Vec<_> = recalled
            .iter()
            .filter(|r| r.key == DEFAULT_SUMMARY_KEY)
            .collect();
        assert_eq!(
            pinned.len(),
            1,
            "the folded summary must recall exactly once, not zero or duplicated: {recalled:?}"
        );
        assert_eq!(pinned[0].value.as_str(), Some("ROLLING SUMMARY"));
    }
}
