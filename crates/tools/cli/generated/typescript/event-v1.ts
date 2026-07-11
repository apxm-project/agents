// AUTO-GENERATED from apxm.event.v1; DO NOT EDIT.

import type { CoreEventKindName, EventKind } from "./core-event-kinds";

export interface EventMeta {
  seq: number;
  timestamp: string;
  trace_id: string;
  source: EventSource;
  span_id: string;
  parent_span_id: string | null;
  scope_id?: string;
  skill?: SkillEventProvenance;
}

export type EventSource = "runtime" | "session" | "server" | "gui" | { backend: string } | { acp: string };

export interface SkillEventProvenance {
  skill_id: string;
  skill_version: string;
  parent_skill_id?: string;
  parent_execution_id?: string;
  flow_name?: string;
}

export interface EventPayload {
  kind: string;
  [key: string]: unknown;
}

export interface TokenEventPayload extends EventPayload {
  kind: "token";
  text: string;
}

export interface LlmStepCompletedEventPayload extends EventPayload {
  kind: "llm_step_completed";
  node_id: number;
  step_number: number;
  model: string;
  finish_reason: Record<string, unknown>;
  usage: Record<string, unknown>;
  performance: Record<string, unknown>;
  tool_call_count: number;
}

export interface ThoughtEventPayload extends EventPayload {
  kind: "thought";
  text: string;
  summary: string | null;
}

export interface ToolCallEventPayload extends EventPayload {
  kind: "tool_call";
  id: string;
  name: string;
}

export interface LlmDoneEventPayload extends EventPayload {
  kind: "llm_done";
  content: string;
  model: string;
  finish_reason: Record<string, unknown>;
  usage: Record<string, unknown>;
  tool_calls: Record<string, unknown>[];
  response_id: string | null;
}

export interface LlmPromptEventPayload extends EventPayload {
  kind: "llm_prompt";
  node_id: number;
  node_name?: string;
  prompt: Record<string, unknown>;
}

export interface UsageEventPayload extends EventPayload {
  kind: "usage";
  input_tokens: number;
  output_tokens: number;
}

export interface RetryEventPayload extends EventPayload {
  kind: "retry";
  attempt: number;
  reason: string;
  retry_after_secs: number;
  backend: string | null;
}

export interface WarningEventPayload extends EventPayload {
  kind: "warning";
  code: string;
  message: string;
}

export interface CitationEventPayload extends EventPayload {
  kind: "citation";
  citations: Record<string, unknown>[];
}

export interface ProviderEventEventPayload extends EventPayload {
  kind: "provider_event";
  provider: string;
  event_type: string;
}

export interface OperationStartEventPayload extends EventPayload {
  kind: "operation_start";
  node_id: number;
  op_type: string;
}

export interface OperationEndEventPayload extends EventPayload {
  kind: "operation_end";
  node_id: number;
  op_type: string;
  duration_ms: number;
  success: boolean;
}

export interface NodeOutputEventPayload extends EventPayload {
  kind: "node_output";
  node_id: number;
  node_name?: string;
  output: Record<string, unknown>;
}

export interface NodeMetricsEventPayload extends EventPayload {
  kind: "node_metrics";
  node_id: number;
  node_name?: string;
  metrics: Record<string, unknown>;
}

export interface ToolStartEventPayload extends EventPayload {
  kind: "tool_start";
  name: string;
  args: Record<string, unknown>;
}

export interface ToolEndEventPayload extends EventPayload {
  kind: "tool_end";
  name: string;
}

export interface PlanCreatedEventPayload extends EventPayload {
  kind: "plan_created";
  plan_id: string;
  steps: number;
}

export interface PlanStepStartedEventPayload extends EventPayload {
  kind: "plan_step_started";
  plan_id: string;
  step_index: number;
}

export interface PlanStepCompletedEventPayload extends EventPayload {
  kind: "plan_step_completed";
  plan_id: string;
  step_index: number;
  success: boolean;
}

export interface PlanWorkflowEmittedEventPayload extends EventPayload {
  kind: "plan_workflow_emitted";
  plan_id: string;
  generating_model: string;
  node_count: number;
  task_ids: number[];
  parallel_fanout_max: number;
}

export interface WorkflowStartedEventPayload extends EventPayload {
  kind: "workflow_started";
  workflow_name: string;
  session_dir: string;
  step_count: number;
}

export interface WorkflowStepStartedEventPayload extends EventPayload {
  kind: "workflow_step_started";
  workflow_name: string;
  workflow_session_dir: string;
  step_id: string;
  step_index: number;
  step_count: number;
}

export interface WorkflowStepCompletedEventPayload extends EventPayload {
  kind: "workflow_step_completed";
  workflow_name: string;
  workflow_session_dir: string;
  step_id: string;
  step_index: number;
  status: "success" | "failed" | "skipped";
  success: boolean;
  duration_ms: number;
  session_dir?: string;
  error?: string;
}

export interface WorkflowFinishedEventPayload extends EventPayload {
  kind: "workflow_finished";
  workflow_name: string;
  session_dir: string;
  status: "success" | "partial_failure" | "failed";
  success: boolean;
  duration_ms: number;
  step_count: number;
}

export interface ExecutionStartedEventPayload extends EventPayload {
  kind: "execution_started";
  execution_id: string;
  args?: string[];
  user_text?: string;
}

export interface ExecuteCompleteEventPayload extends EventPayload {
  kind: "execute_complete";
}

export interface MemoryReadEventPayload extends EventPayload {
  kind: "memory_read";
  scope: string;
  key: string;
}

export interface MemoryWriteEventPayload extends EventPayload {
  kind: "memory_write";
  scope: string;
  key: string;
}

export interface CheckpointSavedEventPayload extends EventPayload {
  kind: "checkpoint_saved";
  checkpoint_id: string;
}

export interface CheckpointRestoredEventPayload extends EventPayload {
  kind: "checkpoint_restored";
  checkpoint_id: string;
}

export interface SchedulerDecisionEventPayload extends EventPayload {
  kind: "scheduler_decision";
  node_id: number;
  delay_ms: number;
  reason: string;
}

export interface ModelRouteDecisionEventPayload extends EventPayload {
  kind: "model_route_decision";
  backend: string;
  model?: string | null;
  was_failover: boolean;
  reason: string;
  rejected_candidates?: Record<string, unknown>[];
}

export interface AgentRouteDecisionEventPayload extends EventPayload {
  kind: "agent_route_decision";
  id: string;
  profile?: string | null;
  source: "explicit" | "selected" | "deterministic";
  reason: string;
  required_capabilities?: string[];
  rejected_candidates?: Record<string, unknown>[];
}

export interface HeadOfLineBlockEventPayload extends EventPayload {
  kind: "head_of_line_block";
  blocker_node: number;
  blocked_node: number;
  wait_ms: number;
  reason: string;
}

export interface GpuUtilizationEventPayload extends EventPayload {
  kind: "gpu_utilization";
  gpu_id: number;
  utilization_pct: number;
  memory_pct: number;
}

export interface TokenUsageEventPayload extends EventPayload {
  kind: "token_usage";
  node_id: number;
  input_tokens: number;
  output_tokens: number;
}

export interface MemoizationHitEventPayload extends EventPayload {
  kind: "memoization_hit";
  node_id: number;
}

export interface ErrorEventPayload extends EventPayload {
  kind: "error";
  message: string;
  status: string | null;
  recoverable: boolean;
}

export interface AgentSpawnedEventPayload extends EventPayload {
  kind: "agent_spawned";
  node_id: number;
  agent_code: string;
  parent_execution_id: string;
  profile?: string;
  process_id?: string;
  scope_policy?: string;
}

export interface CommunicateDispatchedEventPayload extends EventPayload {
  kind: "communicate_dispatched";
  node_id: number;
  target_agent: string;
  protocol: "local" | "http" | "https" | "acp" | "broadcast";
  message_excerpt?: string;
}

export interface GraphEdgeEventPayload extends EventPayload {
  kind: "graph_edge";
  from_node_id: number;
  to_node_id: number;
  edge_kind: "dispatch" | "tool_invocation" | "synthesis_feed";
}

export interface ContextCompactedEventPayload extends EventPayload {
  kind: "context_compacted";
  original_tokens: number;
  new_tokens: number;
}

export interface ModelReroutedEventPayload extends EventPayload {
  kind: "model_rerouted";
  original_model: string;
  new_model: string;
  reason: string;
}

export interface CancelledEventPayload extends EventPayload {
  kind: "cancelled";
  reason: string | null;
}

export interface LoopDetectedEventPayload extends EventPayload {
  kind: "loop_detected";
  pattern: string;
  iterations: number;
}

export interface ContextWindowWarningEventPayload extends EventPayload {
  kind: "context_window_warning";
  current_tokens: number;
  max_tokens: number;
  utilization_pct: number;
}

export interface SessionStartEventPayload extends EventPayload {
  kind: "session_start";
  session_id: string;
}

export interface SessionEndEventPayload extends EventPayload {
  kind: "session_end";
  session_id: string;
  total_turns: number;
}

export interface TurnBoundaryEventPayload extends EventPayload {
  kind: "turn_boundary";
  turn_number: number;
  direction: "request" | "response";
}

export interface TurnStartedEventPayload extends EventPayload {
  kind: "turn_started";
  execution_id: string;
  turn_id?: string;
  coordinator_label?: string;
}

export interface TurnCompleteEventPayload extends EventPayload {
  kind: "turn_complete";
  execution_id: string;
  duration_ms: number;
  had_answer: boolean;
}

export interface TurnAbortedEventPayload extends EventPayload {
  kind: "turn_aborted";
  execution_id: string;
  duration_ms: number;
  reason: string;
  error_message_safe?: string;
}

export interface SubagentSpawnBeginEventPayload extends EventPayload {
  kind: "subagent_spawn_begin";
  agent_code: string;
  agent_name?: string;
  agent_type?: string;
  module_key?: string;
  autonomy_policy?: string;
  parent_span_id?: string;
}

export interface SubagentSpawnEndEventPayload extends EventPayload {
  kind: "subagent_spawn_end";
  agent_code: string;
}

export interface SubagentLlmCallBeginEventPayload extends EventPayload {
  kind: "subagent_llm_call_begin";
  agent_code: string;
  model: string;
  backend: string;
  tool_manifest_count: number;
}

export interface SubagentLlmCallEndEventPayload extends EventPayload {
  kind: "subagent_llm_call_end";
  agent_code: string;
  finish_reason: string;
  usage: Record<string, unknown>;
  content_len: number;
}

export interface ToolCallBeginEventPayload extends EventPayload {
  kind: "tool_call_begin";
  agent_code: string;
  tool_name: string;
  argument_keys: string[];
}

export interface ToolCallEndEventPayload extends EventPayload {
  kind: "tool_call_end";
  agent_code: string;
  tool_name: string;
  result_keys: string[];
  status: "ok" | "error" | "approval_pending";
  latency_ms: number;
}

export interface SubagentDoneEventPayload extends EventPayload {
  kind: "subagent_done";
  agent_code: string;
  total_tool_calls: number;
  usage_total: Record<string, unknown>;
  evidence_excerpt?: string;
}

export interface SubagentFailedEventPayload extends EventPayload {
  kind: "subagent_failed";
  agent_code: string;
  error_class: string;
  error_message_safe: string;
}

export interface AgentMessageEventPayload extends EventPayload {
  kind: "agent_message";
  text: string;
  item_id?: string;
  response_id?: string;
  usage?: Record<string, unknown>;
}

export interface ApprovalRequestEventPayload extends EventPayload {
  kind: "approval_request";
  agent_code: string;
  tool_name: string;
  approval_id: string;
  risk_level: "low" | "medium" | "high";
}

export interface ApprovalResolvedEventPayload extends EventPayload {
  kind: "approval_resolved";
  approval_id: string;
  decision: "approved" | "denied" | "expired";
}

export type KnownEventPayload =
  | TokenEventPayload
  | LlmStepCompletedEventPayload
  | ThoughtEventPayload
  | ToolCallEventPayload
  | LlmDoneEventPayload
  | LlmPromptEventPayload
  | UsageEventPayload
  | RetryEventPayload
  | WarningEventPayload
  | CitationEventPayload
  | ProviderEventEventPayload
  | OperationStartEventPayload
  | OperationEndEventPayload
  | NodeOutputEventPayload
  | NodeMetricsEventPayload
  | ToolStartEventPayload
  | ToolEndEventPayload
  | PlanCreatedEventPayload
  | PlanStepStartedEventPayload
  | PlanStepCompletedEventPayload
  | PlanWorkflowEmittedEventPayload
  | WorkflowStartedEventPayload
  | WorkflowStepStartedEventPayload
  | WorkflowStepCompletedEventPayload
  | WorkflowFinishedEventPayload
  | ExecutionStartedEventPayload
  | ExecuteCompleteEventPayload
  | MemoryReadEventPayload
  | MemoryWriteEventPayload
  | CheckpointSavedEventPayload
  | CheckpointRestoredEventPayload
  | SchedulerDecisionEventPayload
  | ModelRouteDecisionEventPayload
  | AgentRouteDecisionEventPayload
  | HeadOfLineBlockEventPayload
  | GpuUtilizationEventPayload
  | TokenUsageEventPayload
  | MemoizationHitEventPayload
  | ErrorEventPayload
  | AgentSpawnedEventPayload
  | CommunicateDispatchedEventPayload
  | GraphEdgeEventPayload
  | ContextCompactedEventPayload
  | ModelReroutedEventPayload
  | CancelledEventPayload
  | LoopDetectedEventPayload
  | ContextWindowWarningEventPayload
  | SessionStartEventPayload
  | SessionEndEventPayload
  | TurnBoundaryEventPayload
  | TurnStartedEventPayload
  | TurnCompleteEventPayload
  | TurnAbortedEventPayload
  | SubagentSpawnBeginEventPayload
  | SubagentSpawnEndEventPayload
  | SubagentLlmCallBeginEventPayload
  | SubagentLlmCallEndEventPayload
  | ToolCallBeginEventPayload
  | ToolCallEndEventPayload
  | SubagentDoneEventPayload
  | SubagentFailedEventPayload
  | AgentMessageEventPayload
  | ApprovalRequestEventPayload
  | ApprovalResolvedEventPayload;

export type EventPayloadValue = KnownEventPayload | EventPayload;

export interface ApxmEvent {
  meta: EventMeta;
  payload: EventPayloadValue;
}

export interface ApxmEventLike {
  meta?: Partial<EventMeta> & Record<string, unknown>;
  payload?: EventPayload;
  [key: string]: unknown;
}

export type EventKindDescriptor = EventKind & { readonly terminalSense: string };

export const EVENT_KIND_REGISTRY: Readonly<Record<CoreEventKindName, EventKindDescriptor>> = {
  "token": { name: "token", category: "stream", terminal: false, terminalSense: "n/a" },
  "thought": { name: "thought", category: "stream", terminal: false, terminalSense: "n/a" },
  "tool_call": { name: "tool_call", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "llm_done": { name: "llm_done", category: "lifecycle", terminal: true, terminalSense: "run_end" },
  "llm_step_completed": { name: "llm_step_completed", category: "observability", terminal: false, terminalSense: "n/a" },
  "llm_prompt": { name: "llm_prompt", category: "observability", terminal: false, terminalSense: "n/a" },
  "usage": { name: "usage", category: "observability", terminal: false, terminalSense: "n/a" },
  "retry": { name: "retry", category: "error", terminal: false, terminalSense: "n/a" },
  "warning": { name: "warning", category: "error", terminal: false, terminalSense: "n/a" },
  "citation": { name: "citation", category: "observability", terminal: false, terminalSense: "n/a" },
  "provider_event": { name: "provider_event", category: "observability", terminal: false, terminalSense: "n/a" },
  "operation_start": { name: "operation_start", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "operation_end": { name: "operation_end", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "node_output": { name: "node_output", category: "observability", terminal: false, terminalSense: "n/a" },
  "node_metrics": { name: "node_metrics", category: "observability", terminal: false, terminalSense: "n/a" },
  "tool_start": { name: "tool_start", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "tool_end": { name: "tool_end", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "plan_created": { name: "plan_created", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "plan_step_started": { name: "plan_step_started", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "plan_step_completed": { name: "plan_step_completed", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "plan_workflow_emitted": { name: "plan_workflow_emitted", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "workflow_started": { name: "workflow_started", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "workflow_step_started": { name: "workflow_step_started", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "workflow_step_completed": { name: "workflow_step_completed", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "workflow_finished": { name: "workflow_finished", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "execution_started": { name: "execution_started", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "execute_complete": { name: "execute_complete", category: "lifecycle", terminal: true, terminalSense: "run_end" },
  "memory_read": { name: "memory_read", category: "observability", terminal: false, terminalSense: "n/a" },
  "memory_write": { name: "memory_write", category: "observability", terminal: false, terminalSense: "n/a" },
  "checkpoint_saved": { name: "checkpoint_saved", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "checkpoint_restored": { name: "checkpoint_restored", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "scheduler_decision": { name: "scheduler_decision", category: "observability", terminal: false, terminalSense: "n/a" },
  "model_route_decision": { name: "model_route_decision", category: "observability", terminal: false, terminalSense: "n/a" },
  "agent_route_decision": { name: "agent_route_decision", category: "observability", terminal: false, terminalSense: "n/a" },
  "head_of_line_block": { name: "head_of_line_block", category: "observability", terminal: false, terminalSense: "n/a" },
  "gpu_utilization": { name: "gpu_utilization", category: "observability", terminal: false, terminalSense: "n/a" },
  "token_usage": { name: "token_usage", category: "observability", terminal: false, terminalSense: "n/a" },
  "memoization_hit": { name: "memoization_hit", category: "observability", terminal: false, terminalSense: "n/a" },
  "error": { name: "error", category: "error", terminal: true, terminalSense: "run_end" },
  "agent_spawned": { name: "agent_spawned", category: "agent", terminal: false, terminalSense: "atomic_no_delta" },
  "communicate_dispatched": { name: "communicate_dispatched", category: "agent", terminal: false, terminalSense: "atomic_no_delta" },
  "graph_edge": { name: "graph_edge", category: "topology", terminal: false, terminalSense: "atomic_no_delta" },
  "context_compacted": { name: "context_compacted", category: "observability", terminal: false, terminalSense: "n/a" },
  "model_rerouted": { name: "model_rerouted", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "cancelled": { name: "cancelled", category: "error", terminal: true, terminalSense: "run_end" },
  "loop_detected": { name: "loop_detected", category: "error", terminal: false, terminalSense: "n/a" },
  "context_window_warning": { name: "context_window_warning", category: "error", terminal: false, terminalSense: "n/a" },
  "session_start": { name: "session_start", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "session_end": { name: "session_end", category: "lifecycle", terminal: true, terminalSense: "run_end" },
  "turn_boundary": { name: "turn_boundary", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "turn_started": { name: "turn_started", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "turn_complete": { name: "turn_complete", category: "lifecycle", terminal: true, terminalSense: "run_end" },
  "turn_aborted": { name: "turn_aborted", category: "lifecycle", terminal: true, terminalSense: "run_end" },
  "subagent_spawn_begin": { name: "subagent_spawn_begin", category: "agent", terminal: false, terminalSense: "n/a" },
  "subagent_spawn_end": { name: "subagent_spawn_end", category: "agent", terminal: false, terminalSense: "n/a" },
  "subagent_llm_call_begin": { name: "subagent_llm_call_begin", category: "agent", terminal: false, terminalSense: "n/a" },
  "subagent_llm_call_end": { name: "subagent_llm_call_end", category: "agent", terminal: false, terminalSense: "n/a" },
  "tool_call_begin": { name: "tool_call_begin", category: "agent", terminal: false, terminalSense: "n/a" },
  "tool_call_end": { name: "tool_call_end", category: "agent", terminal: false, terminalSense: "n/a" },
  "subagent_done": { name: "subagent_done", category: "agent", terminal: true, terminalSense: "run_end" },
  "subagent_failed": { name: "subagent_failed", category: "agent", terminal: true, terminalSense: "run_end" },
  "agent_message": { name: "agent_message", category: "agent", terminal: false, terminalSense: "n/a" },
  "approval_request": { name: "approval_request", category: "agent", terminal: false, terminalSense: "n/a" },
  "approval_resolved": { name: "approval_resolved", category: "agent", terminal: false, terminalSense: "n/a" },
};
