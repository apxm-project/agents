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
  program?: ProgramExecutionProvenance;
}

export type EventSource = "runtime" | "session" | "server" | "gui" | { backend: string } | { acp: string };

export interface ProgramExecutionProvenance {
  program_id: string;
  program_version: string;
  parent_program_id?: string;
  parent_execution_id?: string;
  flow_name?: string;
}

export interface UnknownEventPayload {
  kind: string;
  [key: string]: unknown;
}

export interface TokenEventPayload {
  kind: "token";
  text: string;
  generation?: { "call_id": string; "attempt": number; "step_number": number; };
  [key: string]: unknown;
}

export interface LlmStepCompletedEventPayload {
  kind: "llm_step_completed";
  node_id: number;
  step_number: number;
  model: string;
  finish_reason: { "reason": string; };
  usage: { "input_tokens": number; "output_tokens": number; "cached_input_tokens": number; "reasoning_output_tokens": number; };
  performance: { "latency_ms": number; "prefill_ms": number; "decode_ms": number; };
  tool_call_count: number;
  [key: string]: unknown;
}

export interface ThoughtEventPayload {
  kind: "thought";
  text: string;
  summary: string | null;
  generation?: { "call_id": string; "attempt": number; "step_number": number; };
  [key: string]: unknown;
}

export interface ToolCallEventPayload {
  kind: "tool_call";
  id: string;
  name: string;
  tool_call_correlation?: { "generation": { "call_id": string; "attempt": number; "step_number": number; }; "tool_call_id": string; };
  arguments: unknown;
  [key: string]: unknown;
}

export interface LlmDoneEventPayload {
  kind: "llm_done";
  content: string;
  model: string;
  finish_reason: { "reason": string; [key: string]: unknown; };
  usage: { "input_tokens": number; "output_tokens": number; [key: string]: unknown; };
  tool_calls: { "id": string; "name": string; "tool_call_correlation"?: { "generation": { "call_id": string; "attempt": number; "step_number": number; }; "tool_call_id": string; }; "arguments": unknown; [key: string]: unknown; }[];
  response_id: string | null;
  generation?: { "call_id": string; "attempt": number; "step_number": number; };
  [key: string]: unknown;
}

export interface LlmPromptEventPayload {
  kind: "llm_prompt";
  node_id: number;
  node_name?: string;
  prompt: { "redacted": boolean; "policy": string; "hash": string; "size_bytes": number; "char_count"?: number | null; "content_type": string; "summary": string; [key: string]: unknown; };
  generation?: { "call_id": string; "attempt": number; "step_number": number; };
  [key: string]: unknown;
}

export interface UsageEventPayload {
  kind: "usage";
  input_tokens: number;
  output_tokens: number;
  generation?: { "call_id": string; "attempt": number; "step_number": number; };
  [key: string]: unknown;
}

export interface RetryEventPayload {
  kind: "retry";
  attempt: number;
  reason: string;
  retry_after_secs: number;
  backend: string | null;
  [key: string]: unknown;
}

export interface WarningEventPayload {
  kind: "warning";
  code: string;
  message: string;
  [key: string]: unknown;
}

export interface CitationEventPayload {
  kind: "citation";
  citations: Array<{ "url": string | null; "title": string | null; "start_index": number | null; "end_index": number | null; [key: string]: unknown; }>;
  [key: string]: unknown;
}

export interface ProviderEventEventPayload {
  kind: "provider_event";
  provider: string;
  event_type: string;
  data: unknown;
  [key: string]: unknown;
}

export interface OperationStartEventPayload {
  kind: "operation_start";
  node_id: number;
  op_type: string;
  [key: string]: unknown;
}

export interface OperationEndEventPayload {
  kind: "operation_end";
  node_id: number;
  op_type: string;
  duration_ms: number;
  success: boolean;
  [key: string]: unknown;
}

export interface NodeOutputEventPayload {
  kind: "node_output";
  node_id: number;
  node_name?: string;
  output: { "redacted": boolean; "policy": string; "hash": string; "size_bytes": number; "char_count"?: number | null; "content_type": string; "summary": string; [key: string]: unknown; };
  [key: string]: unknown;
}

export interface NodeMetricsEventPayload {
  kind: "node_metrics";
  node_id: number;
  node_name?: string;
  metrics: { "node_id": number; [key: string]: unknown; };
  [key: string]: unknown;
}

export interface ToolStartEventPayload {
  kind: "tool_start";
  name: string;
  args: Record<string, unknown>;
  tool_call_correlation?: { "generation": { "call_id": string; "attempt": number; "step_number": number; }; "tool_call_id": string; };
  [key: string]: unknown;
}

export interface ToolEndEventPayload {
  kind: "tool_end";
  name: string;
  tool_call_correlation?: { "generation": { "call_id": string; "attempt": number; "step_number": number; }; "tool_call_id": string; };
  result: unknown;
  [key: string]: unknown;
}

export interface PlanCreatedEventPayload {
  kind: "plan_created";
  plan_id: string;
  steps: number;
  [key: string]: unknown;
}

export interface PlanStepStartedEventPayload {
  kind: "plan_step_started";
  plan_id: string;
  step_index: number;
  [key: string]: unknown;
}

export interface PlanStepCompletedEventPayload {
  kind: "plan_step_completed";
  plan_id: string;
  step_index: number;
  success: boolean;
  [key: string]: unknown;
}

export interface PlanWorkflowEmittedEventPayload {
  kind: "plan_workflow_emitted";
  plan_id: string;
  generating_model: string;
  node_count: number;
  task_ids: number[];
  parallel_fanout_max: number;
  [key: string]: unknown;
}

export interface WorkflowStartedEventPayload {
  kind: "workflow_started";
  workflow_name: string;
  session_dir: string;
  step_count: number;
  [key: string]: unknown;
}

export interface WorkflowStepStartedEventPayload {
  kind: "workflow_step_started";
  workflow_name: string;
  workflow_session_dir: string;
  step_id: string;
  step_index: number;
  step_count: number;
  [key: string]: unknown;
}

export interface WorkflowStepCompletedEventPayload {
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
  [key: string]: unknown;
}

export interface WorkflowFinishedEventPayload {
  kind: "workflow_finished";
  workflow_name: string;
  session_dir: string;
  status: "success" | "partial_failure" | "failed";
  success: boolean;
  duration_ms: number;
  step_count: number;
  [key: string]: unknown;
}

export interface ExecutionStartedEventPayload {
  kind: "execution_started";
  execution_id: string;
  args?: string[];
  user_text?: string;
  [key: string]: unknown;
}

export interface ExecuteCompleteEventPayload {
  kind: "execute_complete";
  result: { "execution_id": string; "session_id": string; "outcome": { "status": "success"; } | { "status": "domain_failure"; "error": Record<string, unknown>; } | { "status": "cancellation"; } | { "status": "join_failure"; "failure": { "task": "scheduler_worker" | "scheduler_finalizer" | "runtime_finalizer"; "message": string; "cancelled": boolean; "panicked": boolean; }; }; } | { "execution_id"?: string; "workflow_id"?: string; "run_root"?: string; "trace_id"?: string; "results": Record<string, unknown>; "content": string | null; "session_dir": string | null; "stats": { "executed_nodes": number; "failed_nodes": number; "duration_ms": number; }; "llm_usage": { "input_tokens": number; "output_tokens": number; "total_requests": number; }; "tool_call_counts": Record<string, number>; "parked_session_id"?: string; };
  [key: string]: unknown;
}

export interface MemoryReadEventPayload {
  kind: "memory_read";
  scope: string;
  key: string;
  [key: string]: unknown;
}

export interface MemoryWriteEventPayload {
  kind: "memory_write";
  scope: string;
  key: string;
  [key: string]: unknown;
}

export interface CheckpointSavedEventPayload {
  kind: "checkpoint_saved";
  checkpoint_id: string;
  [key: string]: unknown;
}

export interface CheckpointRestoredEventPayload {
  kind: "checkpoint_restored";
  checkpoint_id: string;
  [key: string]: unknown;
}

export interface SchedulerDecisionEventPayload {
  kind: "scheduler_decision";
  node_id: number;
  delay_ms: number;
  reason: string;
  [key: string]: unknown;
}

export interface ModelRouteDecisionEventPayload {
  kind: "model_route_decision";
  backend: string;
  model?: string | null;
  was_failover: boolean;
  reason: string;
  rejected_candidates?: { "candidate": string; "backend": string; "reason_kind": string; "reason": string; [key: string]: unknown; }[];
  [key: string]: unknown;
}

export interface AgentRouteDecisionEventPayload {
  kind: "agent_route_decision";
  id: string;
  profile?: string | null;
  source: "explicit" | "selected" | "deterministic";
  reason: string;
  required_capabilities?: string[];
  rejected_candidates?: { "profile": string; "missing_capabilities"?: string[]; "reason": string; [key: string]: unknown; }[];
  [key: string]: unknown;
}

export interface HeadOfLineBlockEventPayload {
  kind: "head_of_line_block";
  blocker_node: number;
  blocked_node: number;
  wait_ms: number;
  reason: string;
  [key: string]: unknown;
}

export interface GpuUtilizationEventPayload {
  kind: "gpu_utilization";
  gpu_id: number;
  utilization_pct: number;
  memory_pct: number;
  [key: string]: unknown;
}

export interface TokenUsageEventPayload {
  kind: "token_usage";
  node_id: number;
  input_tokens: number;
  output_tokens: number;
  generation?: { "call_id": string; "attempt": number; "step_number": number; };
  [key: string]: unknown;
}

export interface MemoizationHitEventPayload {
  kind: "memoization_hit";
  node_id: number;
  [key: string]: unknown;
}

export interface ErrorEventPayload {
  kind: "error";
  message: string;
  status: string | null;
  recoverable: boolean;
  [key: string]: unknown;
}

export interface AgentSpawnedEventPayload {
  kind: "agent_spawned";
  node_id: number;
  agent_code: string;
  parent_execution_id: string;
  profile?: string;
  process_id?: string;
  scope_policy?: string;
  [key: string]: unknown;
}

export interface CommunicateDispatchedEventPayload {
  kind: "communicate_dispatched";
  node_id: number;
  target_agent: string;
  protocol: "local" | "http" | "https" | "acp" | "broadcast";
  message_excerpt?: string;
  [key: string]: unknown;
}

export interface GraphEdgeEventPayload {
  kind: "graph_edge";
  from_node_id: number;
  to_node_id: number;
  edge_kind: "dispatch" | "tool_invocation" | "synthesis_feed";
  [key: string]: unknown;
}

export interface ContextCompactedEventPayload {
  kind: "context_compacted";
  original_tokens: number;
  new_tokens: number;
  [key: string]: unknown;
}

export interface ModelContextMetricsEventPayload {
  kind: "model_context_metrics";
  node_id?: number;
  call_kind: "node" | "tool_continuation" | "warmup" | "compaction" | "hook";
  plan_status: "assembled" | "inherited" | "unplanned";
  token_budget?: number;
  original_tokens?: number;
  admitted_tokens?: number;
  kept_segments?: number;
  truncated_segments?: number;
  omitted_token_budget_segments?: number;
  omitted_empty_segments?: number;
  [key: string]: unknown;
}

export interface CapabilityEffectReceiptEventPayload {
  kind: "capability_effect_receipt";
  receipt_id: string;
  execution_id: string;
  graph_id?: string;
  node_id: number;
  invocation_id: string;
  call_id?: string;
  capability_binding: string;
  dispatch_path: "inv_cap" | "ask_tool";
  implementation_kind: "native" | "python" | "typescript" | "host";
  implementation_ref: string;
  request_digest: string;
  admission_kind: "read_only" | "sandbox" | "grant";
  grant_id?: string;
  approval_status?: "not_required" | "approved";
  approval_id?: string;
  idempotency_proof: "remote_deduplicated" | "transaction_verified";
  idempotency_key_digest: string;
  effect_ref: string;
  effect_digest?: string;
  effect_outcome?: "committed" | "deduplicated";
  verified_host_key_id?: string;
  prepare_digest?: string;
  commit_digest?: string;
  status: "committed";
}

export interface ModelReroutedEventPayload {
  kind: "model_rerouted";
  original_model: string;
  new_model: string;
  reason: string;
  [key: string]: unknown;
}

export interface CancelledEventPayload {
  kind: "cancelled";
  reason: string | null;
  [key: string]: unknown;
}

export interface LoopDetectedEventPayload {
  kind: "loop_detected";
  pattern: string;
  iterations: number;
  [key: string]: unknown;
}

export interface ContextWindowWarningEventPayload {
  kind: "context_window_warning";
  current_tokens: number;
  max_tokens: number;
  utilization_pct: number;
  [key: string]: unknown;
}

export interface SessionStartEventPayload {
  kind: "session_start";
  session_id: string;
  [key: string]: unknown;
}

export interface SessionEndEventPayload {
  kind: "session_end";
  session_id: string;
  total_turns: number;
  [key: string]: unknown;
}

export interface TurnBoundaryEventPayload {
  kind: "turn_boundary";
  turn_number: number;
  direction: "request" | "response";
  [key: string]: unknown;
}

export interface TurnStartedEventPayload {
  kind: "turn_started";
  execution_id: string;
  turn_id?: string;
  coordinator_label?: string;
  [key: string]: unknown;
}

export interface TurnCompleteEventPayload {
  kind: "turn_complete";
  execution_id: string;
  duration_ms: number;
  had_answer: boolean;
  [key: string]: unknown;
}

export interface TurnAbortedEventPayload {
  kind: "turn_aborted";
  execution_id: string;
  duration_ms: number;
  reason: string;
  error_message_safe?: string;
  [key: string]: unknown;
}

export interface SubagentSpawnBeginEventPayload {
  kind: "subagent_spawn_begin";
  agent_code: string;
  agent_name?: string;
  agent_type?: string;
  module_key?: string;
  autonomy_policy?: string;
  parent_span_id?: string;
  [key: string]: unknown;
}

export interface SubagentSpawnEndEventPayload {
  kind: "subagent_spawn_end";
  agent_code: string;
  [key: string]: unknown;
}

export interface SubagentLlmCallBeginEventPayload {
  kind: "subagent_llm_call_begin";
  agent_code: string;
  model: string;
  backend: string;
  tool_manifest_count: number;
  generation?: { "call_id": string; "attempt": number; "step_number": number; };
  [key: string]: unknown;
}

export interface SubagentLlmCallEndEventPayload {
  kind: "subagent_llm_call_end";
  agent_code: string;
  finish_reason: string;
  usage: { "input_tokens": number; "output_tokens": number; [key: string]: unknown; };
  content_len: number;
  generation?: { "call_id": string; "attempt": number; "step_number": number; };
  [key: string]: unknown;
}

export interface ToolCallBeginEventPayload {
  kind: "tool_call_begin";
  agent_code: string;
  tool_name: string;
  argument_keys: string[];
  tool_call_correlation?: { "generation": { "call_id": string; "attempt": number; "step_number": number; }; "tool_call_id": string; };
  [key: string]: unknown;
}

export interface ToolCallEndEventPayload {
  kind: "tool_call_end";
  agent_code: string;
  tool_name: string;
  result_keys: string[];
  status: "ok" | "error" | "approval_pending";
  latency_ms: number;
  tool_call_correlation?: { "generation": { "call_id": string; "attempt": number; "step_number": number; }; "tool_call_id": string; };
  [key: string]: unknown;
}

export interface SubagentDoneEventPayload {
  kind: "subagent_done";
  agent_code: string;
  total_tool_calls: number;
  usage_total: { "input_tokens": number; "output_tokens": number; [key: string]: unknown; };
  evidence_excerpt?: string;
  [key: string]: unknown;
}

export interface SubagentFailedEventPayload {
  kind: "subagent_failed";
  agent_code: string;
  error_class: string;
  error_message_safe: string;
  [key: string]: unknown;
}

export interface AgentMessageEventPayload {
  kind: "agent_message";
  text: string;
  item_id?: string;
  response_id?: string;
  usage?: { "input_tokens": number; "output_tokens": number; [key: string]: unknown; };
  [key: string]: unknown;
}

export interface ApprovalRequestEventPayload {
  kind: "approval_request";
  agent_code: string;
  tool_name: string;
  approval_id: string;
  risk_level: "low" | "medium" | "high";
  tool_call_correlation?: { "generation": { "call_id": string; "attempt": number; "step_number": number; }; "tool_call_id": string; };
  [key: string]: unknown;
}

export interface ApprovalResolvedEventPayload {
  kind: "approval_resolved";
  approval_id: string;
  decision: "approved" | "denied" | "expired";
  tool_call_correlation?: { "generation": { "call_id": string; "attempt": number; "step_number": number; }; "tool_call_id": string; };
  [key: string]: unknown;
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
  | ModelContextMetricsEventPayload
  | CapabilityEffectReceiptEventPayload
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

export type EventPayloadValue = KnownEventPayload | UnknownEventPayload;

export interface ApxmEvent {
  meta: EventMeta;
  payload: EventPayloadValue;
}

export interface ApxmEventLike {
  meta?: Partial<EventMeta> & Record<string, unknown>;
  payload?: EventPayloadValue;
  [key: string]: unknown;
}

export type EventKindDescriptor = EventKind & { readonly terminalSense: string };

export const EVENT_KIND_REGISTRY: Readonly<Record<CoreEventKindName, EventKindDescriptor>> = {
  "token": { name: "token", category: "stream", terminal: false, terminalSense: "n/a" },
  "thought": { name: "thought", category: "stream", terminal: false, terminalSense: "n/a" },
  "tool_call": { name: "tool_call", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "llm_done": { name: "llm_done", category: "lifecycle", terminal: false, terminalSense: "atomic_no_delta" },
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
  "model_context_metrics": { name: "model_context_metrics", category: "observability", terminal: false, terminalSense: "n/a" },
  "capability_effect_receipt": { name: "capability_effect_receipt", category: "observability", terminal: false, terminalSense: "n/a" },
  "model_rerouted": { name: "model_rerouted", category: "lifecycle", terminal: false, terminalSense: "n/a" },
  "cancelled": { name: "cancelled", category: "error", terminal: false, terminalSense: "n/a" },
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
