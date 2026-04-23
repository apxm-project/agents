//! Typed event identifiers and categories.

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
pub const TOOL_START: EventKind = EventKind::new("tool_start", EventCategory::Lifecycle, false);
pub const TOOL_END: EventKind = EventKind::new("tool_end", EventCategory::Lifecycle, false);
pub const PLAN_CREATED: EventKind = EventKind::new("plan_created", EventCategory::Lifecycle, false);
pub const PLAN_STEP_STARTED: EventKind =
    EventKind::new("plan_step_started", EventCategory::Lifecycle, false);
pub const PLAN_STEP_COMPLETED: EventKind =
    EventKind::new("plan_step_completed", EventCategory::Lifecycle, false);
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
