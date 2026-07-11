# AUTO-GENERATED from apxm.event.v1; DO NOT EDIT.

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Final

EVENT_KIND_REGISTRY: Final[dict[str, dict[str, object]]] = {
  "agent_message": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "agent_route_decision": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "agent_spawned": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "atomic_no_delta"
  },
  "approval_request": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "approval_resolved": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "cancelled": {
    "category": "error",
    "terminal": true,
    "terminal_sense": "run_end"
  },
  "checkpoint_restored": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "checkpoint_saved": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "citation": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "communicate_dispatched": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "atomic_no_delta"
  },
  "context_compacted": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "context_window_warning": {
    "category": "error",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "error": {
    "category": "error",
    "terminal": true,
    "terminal_sense": "run_end"
  },
  "execute_complete": {
    "category": "lifecycle",
    "terminal": true,
    "terminal_sense": "run_end"
  },
  "execution_started": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "gpu_utilization": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "graph_edge": {
    "category": "topology",
    "terminal": false,
    "terminal_sense": "atomic_no_delta"
  },
  "head_of_line_block": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "llm_done": {
    "category": "lifecycle",
    "terminal": true,
    "terminal_sense": "run_end"
  },
  "llm_prompt": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "loop_detected": {
    "category": "error",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "memoization_hit": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "memory_read": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "memory_write": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "model_rerouted": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "model_route_decision": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "node_metrics": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "node_output": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "operation_end": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "operation_start": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "plan_created": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "plan_step_completed": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "plan_step_started": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "plan_workflow_emitted": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "provider_event": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "retry": {
    "category": "error",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "scheduler_decision": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "session_end": {
    "category": "lifecycle",
    "terminal": true,
    "terminal_sense": "run_end"
  },
  "session_start": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "subagent_done": {
    "category": "agent",
    "terminal": true,
    "terminal_sense": "run_end"
  },
  "subagent_failed": {
    "category": "agent",
    "terminal": true,
    "terminal_sense": "run_end"
  },
  "subagent_llm_call_begin": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "subagent_llm_call_end": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "subagent_spawn_begin": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "subagent_spawn_end": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "thought": {
    "category": "stream",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "token": {
    "category": "stream",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "token_usage": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "tool_call": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "tool_call_begin": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "tool_call_end": {
    "category": "agent",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "tool_end": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "tool_start": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "turn_aborted": {
    "category": "lifecycle",
    "terminal": true,
    "terminal_sense": "run_end"
  },
  "turn_boundary": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "turn_complete": {
    "category": "lifecycle",
    "terminal": true,
    "terminal_sense": "run_end"
  },
  "turn_started": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "usage": {
    "category": "observability",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "warning": {
    "category": "error",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "workflow_finished": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "workflow_started": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "workflow_step_completed": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  },
  "workflow_step_started": {
    "category": "lifecycle",
    "terminal": false,
    "terminal_sense": "n/a"
  }
}

@dataclass(frozen=True)
class EventMeta:
    seq: int
    timestamp: str
    trace_id: str
    source: object
    span_id: str
    parent_span_id: str | None
    scope_id: str | None = None
    skill: dict[str, object] | None = None

@dataclass(frozen=True)
class ApxmEvent:
    meta: EventMeta
    payload: dict[str, Any]

CORE_EVENT_KINDS: Final[tuple[str, ...]] = (
    'token',
    'thought',
    'tool_call',
    'llm_done',
    'llm_prompt',
    'usage',
    'retry',
    'warning',
    'citation',
    'provider_event',
    'operation_start',
    'operation_end',
    'node_output',
    'node_metrics',
    'tool_start',
    'tool_end',
    'plan_created',
    'plan_step_started',
    'plan_step_completed',
    'plan_workflow_emitted',
    'workflow_started',
    'workflow_step_started',
    'workflow_step_completed',
    'workflow_finished',
    'execution_started',
    'execute_complete',
    'memory_read',
    'memory_write',
    'checkpoint_saved',
    'checkpoint_restored',
    'scheduler_decision',
    'model_route_decision',
    'agent_route_decision',
    'head_of_line_block',
    'gpu_utilization',
    'token_usage',
    'memoization_hit',
    'error',
    'agent_spawned',
    'communicate_dispatched',
    'graph_edge',
    'context_compacted',
    'model_rerouted',
    'cancelled',
    'loop_detected',
    'context_window_warning',
    'session_start',
    'session_end',
    'turn_boundary',
    'turn_started',
    'turn_complete',
    'turn_aborted',
    'subagent_spawn_begin',
    'subagent_spawn_end',
    'subagent_llm_call_begin',
    'subagent_llm_call_end',
    'tool_call_begin',
    'tool_call_end',
    'subagent_done',
    'subagent_failed',
    'agent_message',
    'approval_request',
    'approval_resolved',
)
