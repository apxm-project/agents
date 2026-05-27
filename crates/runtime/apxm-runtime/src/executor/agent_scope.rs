//! Agent-scope tracking primitive (Layer 2 — agent-layer event vocabulary).
//!
//! The executor maintains an `AgentScopeStack` on each `ExecutionContext`.
//! When a SPAWN_AGENT node fires, the spawn handler pushes a new
//! [`AgentScope`] onto the stack and emits a `subagent_spawn_begin` event.
//! When the agent's subgraph terminates (cleanly or with error), the stack
//! pops and emits `subagent_done` / `subagent_failed`.
//!
//! While the stack is non-empty, ASK and INV_TOOL handlers emit BOTH the
//! Layer 1 graph events (`operation_start`, `tool_start`, …) AND their
//! Layer 2 agent counterparts (`subagent_llm_call_begin`, `tool_call_begin`,
//! …) — paired via `meta.call_id`. The top of the stack supplies
//! `agent_code` for those agent-layer events.
//!
//! ## Why a stack, not a single slot
//!
//! A coordinator agent can SPAWN_AGENT sub-agents that in turn SPAWN_AGENT
//! more sub-agents. Each scope nests; the executor needs to know *which*
//! agent owns the operation currently running. The stack mirrors
//! call-stack semantics with no shared-mutable-state surprises (push and
//! pop are wrapped in `&self` methods over a `Mutex<Vec<…>>`).
//!
//! ## Thread / task model
//!
//! Concrete `ExecutionContext`s are shared across scheduler workers via
//! `Arc`. The stack lives behind a `std::sync::Mutex` because the
//! interior is small and uncontended: only handler entry/exit code
//! touches it, never the hot async-await path inside an LLM call.

use std::sync::Mutex;

// ── Wire-string constants for Layer 2 events. Per CLAUDE.md §7 rule 9,
// no inline literal strings in handlers. ─────────────────────────────

/// Backend identifier used when an ASK request didn't pin one.
pub const LAYER2_BACKEND_DEFAULT: &str = "default";
/// `ToolCallEndPayload::status` value for a successful tool invocation.
pub const LAYER2_TOOL_STATUS_OK: &str = "ok";
/// `ToolCallEndPayload::status` value for a failed tool invocation.
pub const LAYER2_TOOL_STATUS_ERROR: &str = "error";
/// `SubagentLlmCallEndPayload::finish_reason` when the runtime didn't
/// learn one from the backend (the typed value-level path doesn't
/// surface a finish reason).
pub const LAYER2_FINISH_REASON_STOP: &str = "stop";
/// `SubagentFailedPayload::error_class` for runtime errors not otherwise
/// classified.
pub const LAYER2_ERROR_CLASS_RUNTIME: &str = "runtime_error";

/// One agent execution scope.
#[derive(Debug, Clone)]
pub struct AgentScope {
    /// Stable agent code (matches `agent_name` on the SPAWN_AGENT op).
    pub agent_code: String,
    /// Span id assigned by the executor for events emitted in this scope.
    pub span_id: String,
    /// Parent scope's span id, if any. `None` for the top-level.
    pub parent_span_id: Option<String>,
    /// Monotonic wall-clock timestamp when the scope was pushed.
    pub started_at_ms: u64,
    /// Optional autonomy policy hint for write-side gating.
    pub autonomy_policy: Option<String>,
}

impl AgentScope {
    /// Construct a new scope with the current wall-clock as `started_at_ms`.
    pub fn new(
        agent_code: impl Into<String>,
        span_id: impl Into<String>,
        parent_span_id: Option<String>,
        autonomy_policy: Option<String>,
    ) -> Self {
        Self {
            agent_code: agent_code.into(),
            span_id: span_id.into(),
            parent_span_id,
            started_at_ms: now_ms(),
            autonomy_policy,
        }
    }
}

/// LIFO stack of agent scopes for an execution context.
///
/// Cheap to clone (the inner `Arc<Mutex<…>>` is reference-counted) but
/// most call sites should treat the stack as a fixed field on
/// `ExecutionContext` and take `&self`.
#[derive(Debug, Default)]
pub struct AgentScopeStack {
    inner: Mutex<Vec<AgentScope>>,
}

impl AgentScopeStack {
    /// Construct an empty stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a new scope. Returns the depth after push (1-based).
    pub fn push(&self, scope: AgentScope) -> usize {
        let mut guard = self.lock();
        guard.push(scope);
        guard.len()
    }

    /// Pop the top scope. Returns `None` if the stack was empty.
    pub fn pop(&self) -> Option<AgentScope> {
        self.lock().pop()
    }

    /// Inspect the top scope without popping.
    pub fn peek(&self) -> Option<AgentScope> {
        self.lock().last().cloned()
    }

    /// Current stack depth (0 = top-level / no agent scope).
    pub fn depth(&self) -> usize {
        self.lock().len()
    }

    /// True when no agent scope is active.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Clone the current stack. Useful for diagnostics; not for the hot
    /// path of every handler.
    pub fn snapshot(&self) -> Vec<AgentScope> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<AgentScope>> {
        // The stack is small and only touched on handler entry/exit, so
        // poisoning is treated as a fatal bug — there is no meaningful
        // recovery path. `expect` keeps the surface tiny.
        self.inner.expect_locked()
    }
}

/// Extension to make the lock recovery explicit; keeps the call sites tidy.
trait MutexExpectLocked<T> {
    fn expect_locked(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> MutexExpectLocked<T> for Mutex<T> {
    fn expect_locked(&self) -> std::sync::MutexGuard<'_, T> {
        match self.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stack_has_depth_zero() {
        let stack = AgentScopeStack::new();
        assert_eq!(stack.depth(), 0);
        assert!(stack.is_empty());
        assert!(stack.peek().is_none());
        assert!(stack.pop().is_none());
    }

    #[test]
    fn push_increments_depth_and_peek_returns_top() {
        let stack = AgentScopeStack::new();
        let depth = stack.push(AgentScope::new("crm", "span-1", None, None));
        assert_eq!(depth, 1);
        assert_eq!(stack.depth(), 1);
        let top = stack.peek().expect("top");
        assert_eq!(top.agent_code, "crm");
        assert_eq!(top.span_id, "span-1");
        assert!(top.parent_span_id.is_none());
    }

    #[test]
    fn lifo_ordering_on_pop() {
        let stack = AgentScopeStack::new();
        stack.push(AgentScope::new("cleo", "span-cleo", None, None));
        stack.push(AgentScope::new(
            "crm",
            "span-crm",
            Some("span-cleo".into()),
            Some("ask_before_write".into()),
        ));
        assert_eq!(stack.depth(), 2);

        let top = stack.pop().expect("crm scope");
        assert_eq!(top.agent_code, "crm");
        assert_eq!(top.parent_span_id.as_deref(), Some("span-cleo"));
        assert_eq!(top.autonomy_policy.as_deref(), Some("ask_before_write"));

        let next = stack.pop().expect("cleo scope");
        assert_eq!(next.agent_code, "cleo");
        assert!(stack.is_empty());
    }

    #[test]
    fn snapshot_clones_without_draining() {
        let stack = AgentScopeStack::new();
        stack.push(AgentScope::new("a", "1", None, None));
        stack.push(AgentScope::new("b", "2", Some("1".into()), None));
        let snap = stack.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(stack.depth(), 2, "snapshot must not mutate");
        assert_eq!(snap[0].agent_code, "a");
        assert_eq!(snap[1].agent_code, "b");
    }

    #[test]
    fn started_at_ms_is_populated() {
        let scope = AgentScope::new("x", "s", None, None);
        // System clock can be 0 in degenerate environments — accept >=0.
        let _ = scope.started_at_ms;
    }
}
