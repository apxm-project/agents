//! Typed event identifiers and categories.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

/// Coarse routing category for an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    Stream,
    Lifecycle,
    Error,
    Observability,
    UserAction,
    /// Multi-agent lifecycle: spawn, dispatch, communicate.
    Agent,
    /// Graph topology / edge resolution events (used by observers to
    /// reconstruct the dispatch tree without re-deriving it).
    Topology,
}

impl EventCategory {
    /// Stable list of all APXM event categories.
    pub const ALL: &[EventCategory] = &[
        EventCategory::Stream,
        EventCategory::Lifecycle,
        EventCategory::Error,
        EventCategory::Observability,
        EventCategory::UserAction,
        EventCategory::Agent,
        EventCategory::Topology,
    ];

    /// Wire spelling for this category.
    pub const fn as_str(self) -> &'static str {
        match self {
            EventCategory::Stream => "stream",
            EventCategory::Lifecycle => "lifecycle",
            EventCategory::Error => "error",
            EventCategory::Observability => "observability",
            EventCategory::UserAction => "user_action",
            EventCategory::Agent => "agent",
            EventCategory::Topology => "topology",
        }
    }
}

/// Typed event kind identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventKind {
    name: &'static str,
    category: EventCategory,
    terminal: bool,
}

impl EventKind {
    /// Create a new kind. Intended for use in `const` declarations.
    pub const fn new(name: &'static str, category: EventCategory, terminal: bool) -> Self {
        Self {
            name,
            category,
            terminal,
        }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }

    pub const fn category(self) -> EventCategory {
        self.category
    }

    pub const fn is_terminal(self) -> bool {
        self.terminal
    }

    pub const fn sse_event_type(self) -> &'static str {
        self.name
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

// LLM event kinds
pub const TOKEN: EventKind = EventKind::new("token", EventCategory::Stream, false);
pub const THOUGHT: EventKind = EventKind::new("thought", EventCategory::Stream, false);
pub const TOOL_CALL: EventKind = EventKind::new("tool_call", EventCategory::Lifecycle, false);
pub const LLM_DONE: EventKind = EventKind::new("llm_done", EventCategory::Lifecycle, true);
pub const LLM_PROMPT: EventKind = EventKind::new("llm_prompt", EventCategory::Observability, false);
pub const USAGE: EventKind = EventKind::new("usage", EventCategory::Observability, false);
pub const RETRY: EventKind = EventKind::new("retry", EventCategory::Error, false);
pub const WARNING: EventKind = EventKind::new("warning", EventCategory::Error, false);
pub const CITATION: EventKind = EventKind::new("citation", EventCategory::Observability, false);
pub const PROVIDER_EVENT: EventKind =
    EventKind::new("provider_event", EventCategory::Observability, false);

// Runtime event kinds
pub const OPERATION_START: EventKind =
    EventKind::new("operation_start", EventCategory::Lifecycle, false);
pub const OPERATION_END: EventKind =
    EventKind::new("operation_end", EventCategory::Lifecycle, false);
pub const NODE_OUTPUT: EventKind =
    EventKind::new("node_output", EventCategory::Observability, false);
pub const NODE_METRICS: EventKind =
    EventKind::new("node_metrics", EventCategory::Observability, false);
pub const TOOL_START: EventKind = EventKind::new("tool_start", EventCategory::Lifecycle, false);
pub const TOOL_END: EventKind = EventKind::new("tool_end", EventCategory::Lifecycle, false);
pub const PLAN_CREATED: EventKind = EventKind::new("plan_created", EventCategory::Lifecycle, false);
pub const PLAN_STEP_STARTED: EventKind =
    EventKind::new("plan_step_started", EventCategory::Lifecycle, false);
pub const PLAN_STEP_COMPLETED: EventKind =
    EventKind::new("plan_step_completed", EventCategory::Lifecycle, false);
pub const PLAN_GRAPH_EMITTED: EventKind =
    EventKind::new("plan_graph_emitted", EventCategory::Lifecycle, false);
pub const WORKFLOW_STARTED: EventKind =
    EventKind::new("workflow_started", EventCategory::Lifecycle, false);
pub const WORKFLOW_STEP_STARTED: EventKind =
    EventKind::new("workflow_step_started", EventCategory::Lifecycle, false);
pub const WORKFLOW_STEP_COMPLETED: EventKind =
    EventKind::new("workflow_step_completed", EventCategory::Lifecycle, false);
pub const WORKFLOW_FINISHED: EventKind =
    EventKind::new("workflow_finished", EventCategory::Lifecycle, false);
pub const EXECUTION_STARTED: EventKind =
    EventKind::new("execution_started", EventCategory::Lifecycle, false);
pub const EXECUTE_COMPLETE: EventKind =
    EventKind::new("execute_complete", EventCategory::Lifecycle, true);
pub const ORCHESTRATOR_SLEEP: EventKind =
    EventKind::new("orchestrator_sleep", EventCategory::Lifecycle, false);
pub const ORCHESTRATOR_WAKE: EventKind =
    EventKind::new("orchestrator_wake", EventCategory::Lifecycle, true);
// Goal-convergence event kinds. A goal is a bounded sequence of admitted
// orchestration passes that runs until a typed gate verdict reports the goal
// is met (or a bound is hit). These make the convergence decision a runtime,
// observable fact rather than an instruction the orchestrator prompt is
// trusted to honor. They are non-terminal: a pass still ends on
// EXECUTE_COMPLETE / ORCHESTRATOR_WAKE, and the goal decision rides alongside.
pub const GOAL_GATE_VERDICT: EventKind =
    EventKind::new("goal_gate_verdict", EventCategory::Lifecycle, false);
pub const GOAL_CONVERGED: EventKind =
    EventKind::new("goal_converged", EventCategory::Lifecycle, false);
pub const GOAL_NEEDS_ANOTHER_PASS: EventKind =
    EventKind::new("goal_needs_another_pass", EventCategory::Lifecycle, false);
pub const GOAL_HALTED: EventKind =
    EventKind::new("goal_halted", EventCategory::Lifecycle, false);
pub const MEMORY_READ: EventKind =
    EventKind::new("memory_read", EventCategory::Observability, false);
pub const MEMORY_WRITE: EventKind =
    EventKind::new("memory_write", EventCategory::Observability, false);
pub const CHECKPOINT_SAVED: EventKind =
    EventKind::new("checkpoint_saved", EventCategory::Lifecycle, false);
pub const CHECKPOINT_RESTORED: EventKind =
    EventKind::new("checkpoint_restored", EventCategory::Lifecycle, false);
pub const SCHEDULER_DECISION: EventKind =
    EventKind::new("scheduler_decision", EventCategory::Observability, false);
pub const HEAD_OF_LINE_BLOCK: EventKind =
    EventKind::new("head_of_line_block", EventCategory::Observability, false);
pub const GPU_UTILIZATION: EventKind =
    EventKind::new("gpu_utilization", EventCategory::Observability, false);
pub const TOKEN_USAGE: EventKind =
    EventKind::new("token_usage", EventCategory::Observability, false);
pub const MEMOIZATION_HIT: EventKind =
    EventKind::new("memoization_hit", EventCategory::Observability, false);
pub const ERROR: EventKind = EventKind::new("error", EventCategory::Error, true);

// Multi-agent / topology event kinds (Phase 14.8.A — per-graph-node
// enrichment). These are emitted in addition to OPERATION_START so
// observers can reconstruct an agent/tool tree without rederiving it.
pub const AGENT_SPAWNED: EventKind = EventKind::new("agent_spawned", EventCategory::Agent, true);
pub const COMMUNICATE_DISPATCHED: EventKind =
    EventKind::new("communicate_dispatched", EventCategory::Agent, true);
pub const GRAPH_EDGE: EventKind = EventKind::new("graph_edge", EventCategory::Topology, true);

// Session event kinds
pub const CONTEXT_COMPACTED: EventKind =
    EventKind::new("context_compacted", EventCategory::Observability, false);
pub const MODEL_REROUTED: EventKind =
    EventKind::new("model_rerouted", EventCategory::Lifecycle, false);
pub const CANCELLED: EventKind = EventKind::new("cancelled", EventCategory::Error, true);
pub const LOOP_DETECTED: EventKind = EventKind::new("loop_detected", EventCategory::Error, false);
pub const CONTEXT_WINDOW_WARNING: EventKind =
    EventKind::new("context_window_warning", EventCategory::Error, false);
pub const SESSION_START: EventKind =
    EventKind::new("session_start", EventCategory::Lifecycle, false);
pub const SESSION_END: EventKind = EventKind::new("session_end", EventCategory::Lifecycle, true);
pub const TURN_BOUNDARY: EventKind =
    EventKind::new("turn_boundary", EventCategory::Lifecycle, false);

// ── Layer 2 — agent-layer event kinds ──────────────────────────────
// These are emitted alongside the existing Layer 1 graph events
// whenever the executor is inside an agent scope (see
// `crates/runtime/apxm-runtime/src/executor/agent_scope.rs`). They are
// snake_case and mirror CLIC's `ClicDispatchEventKind` so the relay
// can stop translating.
pub const TURN_STARTED: EventKind = EventKind::new("turn_started", EventCategory::Lifecycle, false);
pub const TURN_COMPLETE: EventKind =
    EventKind::new("turn_complete", EventCategory::Lifecycle, true);
pub const TURN_ABORTED: EventKind = EventKind::new("turn_aborted", EventCategory::Lifecycle, true);
pub const SUBAGENT_SPAWN_BEGIN: EventKind =
    EventKind::new("subagent_spawn_begin", EventCategory::Agent, false);
pub const SUBAGENT_SPAWN_END: EventKind =
    EventKind::new("subagent_spawn_end", EventCategory::Agent, false);
pub const SUBAGENT_LLM_CALL_BEGIN: EventKind =
    EventKind::new("subagent_llm_call_begin", EventCategory::Agent, false);
pub const SUBAGENT_LLM_CALL_END: EventKind =
    EventKind::new("subagent_llm_call_end", EventCategory::Agent, false);
pub const TOOL_CALL_BEGIN: EventKind =
    EventKind::new("tool_call_begin", EventCategory::Agent, false);
pub const TOOL_CALL_END: EventKind = EventKind::new("tool_call_end", EventCategory::Agent, false);
pub const SUBAGENT_DONE: EventKind = EventKind::new("subagent_done", EventCategory::Agent, true);
pub const SUBAGENT_FAILED: EventKind =
    EventKind::new("subagent_failed", EventCategory::Agent, true);
pub const AGENT_MESSAGE: EventKind = EventKind::new("agent_message", EventCategory::Agent, false);
pub const APPROVAL_REQUEST: EventKind =
    EventKind::new("approval_request", EventCategory::Agent, false);
pub const APPROVAL_RESOLVED: EventKind =
    EventKind::new("approval_resolved", EventCategory::Agent, false);

/// All core APXM event kinds known to `apxm-core`.
///
/// Extension crates can still define their own [`EventKind`] constants with
/// [`EventKind::new`]. This slice is the stable registry for core events that
/// can round-trip through `ApxmEvent` deserialization in this crate.
pub const CORE_EVENT_KINDS: &[EventKind] = &[
    TOKEN,
    THOUGHT,
    TOOL_CALL,
    LLM_DONE,
    LLM_PROMPT,
    USAGE,
    RETRY,
    WARNING,
    CITATION,
    PROVIDER_EVENT,
    OPERATION_START,
    OPERATION_END,
    NODE_OUTPUT,
    NODE_METRICS,
    TOOL_START,
    TOOL_END,
    PLAN_CREATED,
    PLAN_STEP_STARTED,
    PLAN_STEP_COMPLETED,
    PLAN_GRAPH_EMITTED,
    WORKFLOW_STARTED,
    WORKFLOW_STEP_STARTED,
    WORKFLOW_STEP_COMPLETED,
    WORKFLOW_FINISHED,
    EXECUTION_STARTED,
    EXECUTE_COMPLETE,
    ORCHESTRATOR_SLEEP,
    ORCHESTRATOR_WAKE,
    GOAL_GATE_VERDICT,
    GOAL_CONVERGED,
    GOAL_NEEDS_ANOTHER_PASS,
    GOAL_HALTED,
    MEMORY_READ,
    MEMORY_WRITE,
    CHECKPOINT_SAVED,
    CHECKPOINT_RESTORED,
    SCHEDULER_DECISION,
    HEAD_OF_LINE_BLOCK,
    GPU_UTILIZATION,
    TOKEN_USAGE,
    MEMOIZATION_HIT,
    ERROR,
    AGENT_SPAWNED,
    COMMUNICATE_DISPATCHED,
    GRAPH_EDGE,
    CONTEXT_COMPACTED,
    MODEL_REROUTED,
    CANCELLED,
    LOOP_DETECTED,
    CONTEXT_WINDOW_WARNING,
    SESSION_START,
    SESSION_END,
    TURN_BOUNDARY,
    TURN_STARTED,
    TURN_COMPLETE,
    TURN_ABORTED,
    SUBAGENT_SPAWN_BEGIN,
    SUBAGENT_SPAWN_END,
    SUBAGENT_LLM_CALL_BEGIN,
    SUBAGENT_LLM_CALL_END,
    TOOL_CALL_BEGIN,
    TOOL_CALL_END,
    SUBAGENT_DONE,
    SUBAGENT_FAILED,
    AGENT_MESSAGE,
    APPROVAL_REQUEST,
    APPROVAL_RESOLVED,
];

/// Look up a core APXM event kind by its wire name.
pub fn core_event_kind(name: &str) -> Option<EventKind> {
    CORE_EVENT_KINDS
        .iter()
        .copied()
        .find(|kind| kind.name() == name)
}

/// Build an event kind for opaque/unknown event payloads.
///
/// Unknown event names are interned so the public `EventKind` representation
/// can remain a small copyable `&'static str` identifier.
pub(crate) fn opaque_event_kind(name: &str) -> EventKind {
    if let Some(kind) = core_event_kind(name) {
        return kind;
    }

    static INTERNED_EVENT_NAMES: OnceLock<RwLock<HashMap<String, &'static str>>> = OnceLock::new();
    let names = INTERNED_EVENT_NAMES.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(interned) = names.read().expect("event kind intern lock").get(name) {
        return EventKind::new(interned, EventCategory::Observability, false);
    }

    let mut names = names.write().expect("event kind intern lock");
    let interned = *names
        .entry(name.to_string())
        .or_insert_with_key(|key| Box::leak(key.clone().into_boxed_str()));
    EventKind::new(interned, EventCategory::Observability, false)
}
