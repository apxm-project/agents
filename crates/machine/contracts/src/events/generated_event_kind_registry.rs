// AUTO-GENERATED from apxm.event.v1; DO NOT EDIT.

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaEventCategory {
    Stream,
    Lifecycle,
    Error,
    Observability,
    UserAction,
    Agent,
    Topology,
}

impl SchemaEventCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Lifecycle => "lifecycle",
            Self::Error => "error",
            Self::Observability => "observability",
            Self::UserAction => "user_action",
            Self::Agent => "agent",
            Self::Topology => "topology",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaTerminalSense {
    RunEnd,
    AtomicNoDelta,
    NA,
}

#[allow(dead_code)]
impl SchemaTerminalSense {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunEnd => "run_end",
            Self::AtomicNoDelta => "atomic_no_delta",
            Self::NA => "n/a",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaEventKindEntry {
    pub name: &'static str,
    pub category: SchemaEventCategory,
    pub terminal: bool,
    pub terminal_sense: SchemaTerminalSense,
}

pub const SCHEMA_EVENT_KIND_REGISTRY: &[SchemaEventKindEntry] = &[
    SchemaEventKindEntry {
        name: "agent_message",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "agent_route_decision",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "agent_spawned",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::AtomicNoDelta,
    },
    SchemaEventKindEntry {
        name: "approval_request",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "approval_resolved",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "cancelled",
        category: SchemaEventCategory::Error,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "checkpoint_restored",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "checkpoint_saved",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "citation",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "communicate_dispatched",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::AtomicNoDelta,
    },
    SchemaEventKindEntry {
        name: "context_compacted",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "context_window_warning",
        category: SchemaEventCategory::Error,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "error",
        category: SchemaEventCategory::Error,
        terminal: true,
        terminal_sense: SchemaTerminalSense::RunEnd,
    },
    SchemaEventKindEntry {
        name: "execute_complete",
        category: SchemaEventCategory::Lifecycle,
        terminal: true,
        terminal_sense: SchemaTerminalSense::RunEnd,
    },
    SchemaEventKindEntry {
        name: "execution_started",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "gpu_utilization",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "graph_edge",
        category: SchemaEventCategory::Topology,
        terminal: false,
        terminal_sense: SchemaTerminalSense::AtomicNoDelta,
    },
    SchemaEventKindEntry {
        name: "head_of_line_block",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "llm_done",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::AtomicNoDelta,
    },
    SchemaEventKindEntry {
        name: "llm_prompt",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "llm_step_completed",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "loop_detected",
        category: SchemaEventCategory::Error,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "memoization_hit",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "memory_read",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "memory_write",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "model_rerouted",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "model_route_decision",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "node_metrics",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "node_output",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "operation_end",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "operation_start",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "plan_created",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "plan_step_completed",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "plan_step_started",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "plan_workflow_emitted",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "provider_event",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "retry",
        category: SchemaEventCategory::Error,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "scheduler_decision",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "session_end",
        category: SchemaEventCategory::Lifecycle,
        terminal: true,
        terminal_sense: SchemaTerminalSense::RunEnd,
    },
    SchemaEventKindEntry {
        name: "session_start",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "subagent_done",
        category: SchemaEventCategory::Agent,
        terminal: true,
        terminal_sense: SchemaTerminalSense::RunEnd,
    },
    SchemaEventKindEntry {
        name: "subagent_failed",
        category: SchemaEventCategory::Agent,
        terminal: true,
        terminal_sense: SchemaTerminalSense::RunEnd,
    },
    SchemaEventKindEntry {
        name: "subagent_llm_call_begin",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "subagent_llm_call_end",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "subagent_spawn_begin",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "subagent_spawn_end",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "thought",
        category: SchemaEventCategory::Stream,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "token",
        category: SchemaEventCategory::Stream,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "token_usage",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "tool_call",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "tool_call_begin",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "tool_call_end",
        category: SchemaEventCategory::Agent,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "tool_end",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "tool_start",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "turn_aborted",
        category: SchemaEventCategory::Lifecycle,
        terminal: true,
        terminal_sense: SchemaTerminalSense::RunEnd,
    },
    SchemaEventKindEntry {
        name: "turn_boundary",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "turn_complete",
        category: SchemaEventCategory::Lifecycle,
        terminal: true,
        terminal_sense: SchemaTerminalSense::RunEnd,
    },
    SchemaEventKindEntry {
        name: "turn_started",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "usage",
        category: SchemaEventCategory::Observability,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "warning",
        category: SchemaEventCategory::Error,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "workflow_finished",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "workflow_started",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "workflow_step_completed",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
    SchemaEventKindEntry {
        name: "workflow_step_started",
        category: SchemaEventCategory::Lifecycle,
        terminal: false,
        terminal_sense: SchemaTerminalSense::NA,
    },
];
