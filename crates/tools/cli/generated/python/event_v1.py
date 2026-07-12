# AUTO-GENERATED from apxm.event.v1; DO NOT EDIT.

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Final, Literal, TypeAlias, TypedDict

EVENT_KIND_REGISTRY: Final[dict[str, dict[str, object]]] = {'agent_message': {'category': 'agent', 'terminal': False, 'terminal_sense': 'n/a'},
 'agent_route_decision': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'agent_spawned': {'category': 'agent', 'terminal': False, 'terminal_sense': 'atomic_no_delta'},
 'approval_request': {'category': 'agent', 'terminal': False, 'terminal_sense': 'n/a'},
 'approval_resolved': {'category': 'agent', 'terminal': False, 'terminal_sense': 'n/a'},
 'cancelled': {'category': 'error', 'terminal': False, 'terminal_sense': 'n/a'},
 'checkpoint_restored': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'checkpoint_saved': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'citation': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'communicate_dispatched': {'category': 'agent',
                            'terminal': False,
                            'terminal_sense': 'atomic_no_delta'},
 'context_compacted': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'context_window_warning': {'category': 'error', 'terminal': False, 'terminal_sense': 'n/a'},
 'error': {'category': 'error', 'terminal': True, 'terminal_sense': 'run_end'},
 'execute_complete': {'category': 'lifecycle', 'terminal': True, 'terminal_sense': 'run_end'},
 'execution_started': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'gpu_utilization': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'graph_edge': {'category': 'topology', 'terminal': False, 'terminal_sense': 'atomic_no_delta'},
 'head_of_line_block': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'llm_done': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'atomic_no_delta'},
 'llm_prompt': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'llm_step_completed': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'loop_detected': {'category': 'error', 'terminal': False, 'terminal_sense': 'n/a'},
 'memoization_hit': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'memory_read': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'memory_write': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'model_rerouted': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'model_route_decision': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'node_metrics': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'node_output': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'operation_end': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'operation_start': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'plan_created': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'plan_step_completed': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'plan_step_started': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'plan_workflow_emitted': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'provider_event': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'retry': {'category': 'error', 'terminal': False, 'terminal_sense': 'n/a'},
 'scheduler_decision': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'session_end': {'category': 'lifecycle', 'terminal': True, 'terminal_sense': 'run_end'},
 'session_start': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'subagent_done': {'category': 'agent', 'terminal': True, 'terminal_sense': 'run_end'},
 'subagent_failed': {'category': 'agent', 'terminal': True, 'terminal_sense': 'run_end'},
 'subagent_llm_call_begin': {'category': 'agent', 'terminal': False, 'terminal_sense': 'n/a'},
 'subagent_llm_call_end': {'category': 'agent', 'terminal': False, 'terminal_sense': 'n/a'},
 'subagent_spawn_begin': {'category': 'agent', 'terminal': False, 'terminal_sense': 'n/a'},
 'subagent_spawn_end': {'category': 'agent', 'terminal': False, 'terminal_sense': 'n/a'},
 'thought': {'category': 'stream', 'terminal': False, 'terminal_sense': 'n/a'},
 'token': {'category': 'stream', 'terminal': False, 'terminal_sense': 'n/a'},
 'token_usage': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'tool_call': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'tool_call_begin': {'category': 'agent', 'terminal': False, 'terminal_sense': 'n/a'},
 'tool_call_end': {'category': 'agent', 'terminal': False, 'terminal_sense': 'n/a'},
 'tool_end': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'tool_start': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'turn_aborted': {'category': 'lifecycle', 'terminal': True, 'terminal_sense': 'run_end'},
 'turn_boundary': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'turn_complete': {'category': 'lifecycle', 'terminal': True, 'terminal_sense': 'run_end'},
 'turn_started': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'usage': {'category': 'observability', 'terminal': False, 'terminal_sense': 'n/a'},
 'warning': {'category': 'error', 'terminal': False, 'terminal_sense': 'n/a'},
 'workflow_finished': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'workflow_started': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'workflow_step_completed': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'},
 'workflow_step_started': {'category': 'lifecycle', 'terminal': False, 'terminal_sense': 'n/a'}}

class TokenEventPayloadGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class _TokenEventPayloadOptional(TypedDict, total=False):
    generation: TokenEventPayloadGeneration

class TokenEventPayload(_TokenEventPayloadOptional):
    kind: Literal['token']
    text: str

class LlmStepCompletedEventPayloadFinishReason(TypedDict):
    reason: str

class LlmStepCompletedEventPayloadUsage(TypedDict):
    input_tokens: int
    output_tokens: int
    cached_input_tokens: int
    reasoning_output_tokens: int

class LlmStepCompletedEventPayloadPerformance(TypedDict):
    latency_ms: int | float
    prefill_ms: int | float
    decode_ms: int | float

class LlmStepCompletedEventPayload(TypedDict):
    kind: Literal['llm_step_completed']
    node_id: int
    step_number: int
    model: str
    finish_reason: LlmStepCompletedEventPayloadFinishReason
    usage: LlmStepCompletedEventPayloadUsage
    performance: LlmStepCompletedEventPayloadPerformance
    tool_call_count: int

class ThoughtEventPayloadGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class _ThoughtEventPayloadOptional(TypedDict, total=False):
    generation: ThoughtEventPayloadGeneration

class ThoughtEventPayload(_ThoughtEventPayloadOptional):
    kind: Literal['thought']
    text: str
    summary: str | None

class ToolCallEventPayloadToolCallCorrelationGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class ToolCallEventPayloadToolCallCorrelation(TypedDict):
    generation: ToolCallEventPayloadToolCallCorrelationGeneration
    tool_call_id: str

class _ToolCallEventPayloadOptional(TypedDict, total=False):
    tool_call_correlation: ToolCallEventPayloadToolCallCorrelation

class ToolCallEventPayload(_ToolCallEventPayloadOptional):
    kind: Literal['tool_call']
    id: str
    name: str
    arguments: Any

class LlmDoneEventPayloadFinishReason(TypedDict):
    reason: str

class LlmDoneEventPayloadUsage(TypedDict):
    input_tokens: int
    output_tokens: int

class LlmDoneEventPayloadToolCallsItemToolCallCorrelationGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class LlmDoneEventPayloadToolCallsItemToolCallCorrelation(TypedDict):
    generation: LlmDoneEventPayloadToolCallsItemToolCallCorrelationGeneration
    tool_call_id: str

class _LlmDoneEventPayloadToolCallsItemOptional(TypedDict, total=False):
    tool_call_correlation: LlmDoneEventPayloadToolCallsItemToolCallCorrelation

class LlmDoneEventPayloadToolCallsItem(_LlmDoneEventPayloadToolCallsItemOptional):
    id: str
    name: str
    arguments: Any

class LlmDoneEventPayloadGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class _LlmDoneEventPayloadOptional(TypedDict, total=False):
    generation: LlmDoneEventPayloadGeneration

class LlmDoneEventPayload(_LlmDoneEventPayloadOptional):
    kind: Literal['llm_done']
    content: str
    model: str
    finish_reason: LlmDoneEventPayloadFinishReason
    usage: LlmDoneEventPayloadUsage
    tool_calls: list[LlmDoneEventPayloadToolCallsItem]
    response_id: str | None

class _LlmPromptEventPayloadPromptOptional(TypedDict, total=False):
    char_count: int | None

class LlmPromptEventPayloadPrompt(_LlmPromptEventPayloadPromptOptional):
    redacted: bool
    policy: str
    hash: str
    size_bytes: int
    content_type: str
    summary: str

class LlmPromptEventPayloadGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class _LlmPromptEventPayloadOptional(TypedDict, total=False):
    node_name: str
    generation: LlmPromptEventPayloadGeneration

class LlmPromptEventPayload(_LlmPromptEventPayloadOptional):
    kind: Literal['llm_prompt']
    node_id: int
    prompt: LlmPromptEventPayloadPrompt

class UsageEventPayloadGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class _UsageEventPayloadOptional(TypedDict, total=False):
    generation: UsageEventPayloadGeneration

class UsageEventPayload(_UsageEventPayloadOptional):
    kind: Literal['usage']
    input_tokens: int
    output_tokens: int

class RetryEventPayload(TypedDict):
    kind: Literal['retry']
    attempt: int
    reason: str
    retry_after_secs: int | float
    backend: str | None

class WarningEventPayload(TypedDict):
    kind: Literal['warning']
    code: str
    message: str

class CitationEventPayloadCitationsItem(TypedDict):
    url: str | None
    title: str | None
    start_index: int | None
    end_index: int | None

class CitationEventPayload(TypedDict):
    kind: Literal['citation']
    citations: list[CitationEventPayloadCitationsItem]

class ProviderEventEventPayload(TypedDict):
    kind: Literal['provider_event']
    provider: str
    event_type: str
    data: Any

class OperationStartEventPayload(TypedDict):
    kind: Literal['operation_start']
    node_id: int
    op_type: str

class OperationEndEventPayload(TypedDict):
    kind: Literal['operation_end']
    node_id: int
    op_type: str
    duration_ms: int
    success: bool

class _NodeOutputEventPayloadOutputOptional(TypedDict, total=False):
    char_count: int | None

class NodeOutputEventPayloadOutput(_NodeOutputEventPayloadOutputOptional):
    redacted: bool
    policy: str
    hash: str
    size_bytes: int
    content_type: str
    summary: str

class _NodeOutputEventPayloadOptional(TypedDict, total=False):
    node_name: str

class NodeOutputEventPayload(_NodeOutputEventPayloadOptional):
    kind: Literal['node_output']
    node_id: int
    output: NodeOutputEventPayloadOutput

class NodeMetricsEventPayloadMetrics(TypedDict):
    node_id: int

class _NodeMetricsEventPayloadOptional(TypedDict, total=False):
    node_name: str

class NodeMetricsEventPayload(_NodeMetricsEventPayloadOptional):
    kind: Literal['node_metrics']
    node_id: int
    metrics: NodeMetricsEventPayloadMetrics

class ToolStartEventPayloadToolCallCorrelationGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class ToolStartEventPayloadToolCallCorrelation(TypedDict):
    generation: ToolStartEventPayloadToolCallCorrelationGeneration
    tool_call_id: str

class _ToolStartEventPayloadOptional(TypedDict, total=False):
    tool_call_correlation: ToolStartEventPayloadToolCallCorrelation

class ToolStartEventPayload(_ToolStartEventPayloadOptional):
    kind: Literal['tool_start']
    name: str
    args: dict[str, Any]

class ToolEndEventPayloadToolCallCorrelationGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class ToolEndEventPayloadToolCallCorrelation(TypedDict):
    generation: ToolEndEventPayloadToolCallCorrelationGeneration
    tool_call_id: str

class _ToolEndEventPayloadOptional(TypedDict, total=False):
    tool_call_correlation: ToolEndEventPayloadToolCallCorrelation

class ToolEndEventPayload(_ToolEndEventPayloadOptional):
    kind: Literal['tool_end']
    name: str
    result: Any

class PlanCreatedEventPayload(TypedDict):
    kind: Literal['plan_created']
    plan_id: str
    steps: int

class PlanStepStartedEventPayload(TypedDict):
    kind: Literal['plan_step_started']
    plan_id: str
    step_index: int

class PlanStepCompletedEventPayload(TypedDict):
    kind: Literal['plan_step_completed']
    plan_id: str
    step_index: int
    success: bool

class PlanWorkflowEmittedEventPayload(TypedDict):
    kind: Literal['plan_workflow_emitted']
    plan_id: str
    generating_model: str
    node_count: int
    task_ids: list[int]
    parallel_fanout_max: int

class WorkflowStartedEventPayload(TypedDict):
    kind: Literal['workflow_started']
    workflow_name: str
    session_dir: str
    step_count: int

class WorkflowStepStartedEventPayload(TypedDict):
    kind: Literal['workflow_step_started']
    workflow_name: str
    workflow_session_dir: str
    step_id: str
    step_index: int
    step_count: int

class _WorkflowStepCompletedEventPayloadOptional(TypedDict, total=False):
    session_dir: str
    error: str

class WorkflowStepCompletedEventPayload(_WorkflowStepCompletedEventPayloadOptional):
    kind: Literal['workflow_step_completed']
    workflow_name: str
    workflow_session_dir: str
    step_id: str
    step_index: int
    status: Literal['success', 'failed', 'skipped']
    success: bool
    duration_ms: int

class WorkflowFinishedEventPayload(TypedDict):
    kind: Literal['workflow_finished']
    workflow_name: str
    session_dir: str
    status: Literal['success', 'partial_failure', 'failed']
    success: bool
    duration_ms: int
    step_count: int

class _ExecutionStartedEventPayloadOptional(TypedDict, total=False):
    args: list[str]
    user_text: str

class ExecutionStartedEventPayload(_ExecutionStartedEventPayloadOptional):
    kind: Literal['execution_started']
    execution_id: str

class ExecuteCompleteEventPayloadResultVariant1OutcomeVariant1(TypedDict):
    status: Literal['success']

class ExecuteCompleteEventPayloadResultVariant1OutcomeVariant2(TypedDict):
    status: Literal['domain_failure']
    error: dict[str, Any]

class ExecuteCompleteEventPayloadResultVariant1OutcomeVariant3(TypedDict):
    status: Literal['cancellation']

class ExecuteCompleteEventPayloadResultVariant1OutcomeVariant4Failure(TypedDict):
    task: Literal['scheduler_worker', 'scheduler_finalizer', 'runtime_finalizer']
    message: str
    cancelled: bool
    panicked: bool

class ExecuteCompleteEventPayloadResultVariant1OutcomeVariant4(TypedDict):
    status: Literal['join_failure']
    failure: ExecuteCompleteEventPayloadResultVariant1OutcomeVariant4Failure

class ExecuteCompleteEventPayloadResultVariant1(TypedDict):
    execution_id: str
    session_id: str
    outcome: ExecuteCompleteEventPayloadResultVariant1OutcomeVariant1 | ExecuteCompleteEventPayloadResultVariant1OutcomeVariant2 | ExecuteCompleteEventPayloadResultVariant1OutcomeVariant3 | ExecuteCompleteEventPayloadResultVariant1OutcomeVariant4

class ExecuteCompleteEventPayloadResultVariant2Stats(TypedDict):
    executed_nodes: int
    failed_nodes: int
    duration_ms: int

class ExecuteCompleteEventPayloadResultVariant2LlmUsage(TypedDict):
    input_tokens: int
    output_tokens: int
    total_requests: int

class _ExecuteCompleteEventPayloadResultVariant2Optional(TypedDict, total=False):
    execution_id: str
    workflow_id: str
    run_root: str
    trace_id: str
    parked_session_id: str

class ExecuteCompleteEventPayloadResultVariant2(_ExecuteCompleteEventPayloadResultVariant2Optional):
    results: dict[str, Any]
    content: str | None
    session_dir: str | None
    stats: ExecuteCompleteEventPayloadResultVariant2Stats
    llm_usage: ExecuteCompleteEventPayloadResultVariant2LlmUsage
    tool_call_counts: dict[str, int]

class ExecuteCompleteEventPayload(TypedDict):
    kind: Literal['execute_complete']
    result: ExecuteCompleteEventPayloadResultVariant1 | ExecuteCompleteEventPayloadResultVariant2

class MemoryReadEventPayload(TypedDict):
    kind: Literal['memory_read']
    scope: str
    key: str

class MemoryWriteEventPayload(TypedDict):
    kind: Literal['memory_write']
    scope: str
    key: str

class CheckpointSavedEventPayload(TypedDict):
    kind: Literal['checkpoint_saved']
    checkpoint_id: str

class CheckpointRestoredEventPayload(TypedDict):
    kind: Literal['checkpoint_restored']
    checkpoint_id: str

class SchedulerDecisionEventPayload(TypedDict):
    kind: Literal['scheduler_decision']
    node_id: int
    delay_ms: int
    reason: str

class ModelRouteDecisionEventPayloadRejectedCandidatesItem(TypedDict):
    candidate: str
    backend: str
    reason_kind: str
    reason: str

class _ModelRouteDecisionEventPayloadOptional(TypedDict, total=False):
    model: str | None
    rejected_candidates: list[ModelRouteDecisionEventPayloadRejectedCandidatesItem]

class ModelRouteDecisionEventPayload(_ModelRouteDecisionEventPayloadOptional):
    kind: Literal['model_route_decision']
    backend: str
    was_failover: bool
    reason: str

class _AgentRouteDecisionEventPayloadRejectedCandidatesItemOptional(TypedDict, total=False):
    missing_capabilities: list[str]

class AgentRouteDecisionEventPayloadRejectedCandidatesItem(_AgentRouteDecisionEventPayloadRejectedCandidatesItemOptional):
    profile: str
    reason: str

class _AgentRouteDecisionEventPayloadOptional(TypedDict, total=False):
    profile: str | None
    required_capabilities: list[str]
    rejected_candidates: list[AgentRouteDecisionEventPayloadRejectedCandidatesItem]

class AgentRouteDecisionEventPayload(_AgentRouteDecisionEventPayloadOptional):
    kind: Literal['agent_route_decision']
    id: str
    source: Literal['explicit', 'selected', 'deterministic']
    reason: str

class HeadOfLineBlockEventPayload(TypedDict):
    kind: Literal['head_of_line_block']
    blocker_node: int
    blocked_node: int
    wait_ms: int
    reason: str

class GpuUtilizationEventPayload(TypedDict):
    kind: Literal['gpu_utilization']
    gpu_id: int
    utilization_pct: int | float
    memory_pct: int | float

class TokenUsageEventPayload(TypedDict):
    kind: Literal['token_usage']
    node_id: int
    input_tokens: int
    output_tokens: int

class MemoizationHitEventPayload(TypedDict):
    kind: Literal['memoization_hit']
    node_id: int

class ErrorEventPayload(TypedDict):
    kind: Literal['error']
    message: str
    status: str | None
    recoverable: bool

class _AgentSpawnedEventPayloadOptional(TypedDict, total=False):
    profile: str
    process_id: str
    scope_policy: str

class AgentSpawnedEventPayload(_AgentSpawnedEventPayloadOptional):
    kind: Literal['agent_spawned']
    node_id: int
    agent_code: str
    parent_execution_id: str

class _CommunicateDispatchedEventPayloadOptional(TypedDict, total=False):
    message_excerpt: str

class CommunicateDispatchedEventPayload(_CommunicateDispatchedEventPayloadOptional):
    kind: Literal['communicate_dispatched']
    node_id: int
    target_agent: str
    protocol: Literal['local', 'http', 'https', 'acp', 'broadcast']

class GraphEdgeEventPayload(TypedDict):
    kind: Literal['graph_edge']
    from_node_id: int
    to_node_id: int
    edge_kind: Literal['dispatch', 'tool_invocation', 'synthesis_feed']

class ContextCompactedEventPayload(TypedDict):
    kind: Literal['context_compacted']
    original_tokens: int
    new_tokens: int

class ModelReroutedEventPayload(TypedDict):
    kind: Literal['model_rerouted']
    original_model: str
    new_model: str
    reason: str

class CancelledEventPayload(TypedDict):
    kind: Literal['cancelled']
    reason: str | None

class LoopDetectedEventPayload(TypedDict):
    kind: Literal['loop_detected']
    pattern: str
    iterations: int

class ContextWindowWarningEventPayload(TypedDict):
    kind: Literal['context_window_warning']
    current_tokens: int
    max_tokens: int
    utilization_pct: int | float

class SessionStartEventPayload(TypedDict):
    kind: Literal['session_start']
    session_id: str

class SessionEndEventPayload(TypedDict):
    kind: Literal['session_end']
    session_id: str
    total_turns: int

class TurnBoundaryEventPayload(TypedDict):
    kind: Literal['turn_boundary']
    turn_number: int
    direction: Literal['request', 'response']

class _TurnStartedEventPayloadOptional(TypedDict, total=False):
    turn_id: str
    coordinator_label: str

class TurnStartedEventPayload(_TurnStartedEventPayloadOptional):
    kind: Literal['turn_started']
    execution_id: str

class TurnCompleteEventPayload(TypedDict):
    kind: Literal['turn_complete']
    execution_id: str
    duration_ms: int
    had_answer: bool

class _TurnAbortedEventPayloadOptional(TypedDict, total=False):
    error_message_safe: str

class TurnAbortedEventPayload(_TurnAbortedEventPayloadOptional):
    kind: Literal['turn_aborted']
    execution_id: str
    duration_ms: int
    reason: str

class _SubagentSpawnBeginEventPayloadOptional(TypedDict, total=False):
    agent_name: str
    agent_type: str
    module_key: str
    autonomy_policy: str
    parent_span_id: str

class SubagentSpawnBeginEventPayload(_SubagentSpawnBeginEventPayloadOptional):
    kind: Literal['subagent_spawn_begin']
    agent_code: str

class SubagentSpawnEndEventPayload(TypedDict):
    kind: Literal['subagent_spawn_end']
    agent_code: str

class SubagentLlmCallBeginEventPayloadGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class _SubagentLlmCallBeginEventPayloadOptional(TypedDict, total=False):
    generation: SubagentLlmCallBeginEventPayloadGeneration

class SubagentLlmCallBeginEventPayload(_SubagentLlmCallBeginEventPayloadOptional):
    kind: Literal['subagent_llm_call_begin']
    agent_code: str
    model: str
    backend: str
    tool_manifest_count: int

class SubagentLlmCallEndEventPayloadUsage(TypedDict):
    input_tokens: int
    output_tokens: int

class SubagentLlmCallEndEventPayloadGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class _SubagentLlmCallEndEventPayloadOptional(TypedDict, total=False):
    generation: SubagentLlmCallEndEventPayloadGeneration

class SubagentLlmCallEndEventPayload(_SubagentLlmCallEndEventPayloadOptional):
    kind: Literal['subagent_llm_call_end']
    agent_code: str
    finish_reason: str
    usage: SubagentLlmCallEndEventPayloadUsage
    content_len: int

class ToolCallBeginEventPayloadToolCallCorrelationGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class ToolCallBeginEventPayloadToolCallCorrelation(TypedDict):
    generation: ToolCallBeginEventPayloadToolCallCorrelationGeneration
    tool_call_id: str

class _ToolCallBeginEventPayloadOptional(TypedDict, total=False):
    tool_call_correlation: ToolCallBeginEventPayloadToolCallCorrelation

class ToolCallBeginEventPayload(_ToolCallBeginEventPayloadOptional):
    kind: Literal['tool_call_begin']
    agent_code: str
    tool_name: str
    argument_keys: list[str]

class ToolCallEndEventPayloadToolCallCorrelationGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class ToolCallEndEventPayloadToolCallCorrelation(TypedDict):
    generation: ToolCallEndEventPayloadToolCallCorrelationGeneration
    tool_call_id: str

class _ToolCallEndEventPayloadOptional(TypedDict, total=False):
    tool_call_correlation: ToolCallEndEventPayloadToolCallCorrelation

class ToolCallEndEventPayload(_ToolCallEndEventPayloadOptional):
    kind: Literal['tool_call_end']
    agent_code: str
    tool_name: str
    result_keys: list[str]
    status: Literal['ok', 'error', 'approval_pending']
    latency_ms: int

class SubagentDoneEventPayloadUsageTotal(TypedDict):
    input_tokens: int
    output_tokens: int

class _SubagentDoneEventPayloadOptional(TypedDict, total=False):
    evidence_excerpt: str

class SubagentDoneEventPayload(_SubagentDoneEventPayloadOptional):
    kind: Literal['subagent_done']
    agent_code: str
    total_tool_calls: int
    usage_total: SubagentDoneEventPayloadUsageTotal

class SubagentFailedEventPayload(TypedDict):
    kind: Literal['subagent_failed']
    agent_code: str
    error_class: str
    error_message_safe: str

class AgentMessageEventPayloadUsage(TypedDict):
    input_tokens: int
    output_tokens: int

class _AgentMessageEventPayloadOptional(TypedDict, total=False):
    item_id: str
    response_id: str
    usage: AgentMessageEventPayloadUsage

class AgentMessageEventPayload(_AgentMessageEventPayloadOptional):
    kind: Literal['agent_message']
    text: str

class ApprovalRequestEventPayloadToolCallCorrelationGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class ApprovalRequestEventPayloadToolCallCorrelation(TypedDict):
    generation: ApprovalRequestEventPayloadToolCallCorrelationGeneration
    tool_call_id: str

class _ApprovalRequestEventPayloadOptional(TypedDict, total=False):
    tool_call_correlation: ApprovalRequestEventPayloadToolCallCorrelation

class ApprovalRequestEventPayload(_ApprovalRequestEventPayloadOptional):
    kind: Literal['approval_request']
    agent_code: str
    tool_name: str
    approval_id: str
    risk_level: Literal['low', 'medium', 'high']

class ApprovalResolvedEventPayloadToolCallCorrelationGeneration(TypedDict):
    call_id: str
    attempt: int
    step_number: int

class ApprovalResolvedEventPayloadToolCallCorrelation(TypedDict):
    generation: ApprovalResolvedEventPayloadToolCallCorrelationGeneration
    tool_call_id: str

class _ApprovalResolvedEventPayloadOptional(TypedDict, total=False):
    tool_call_correlation: ApprovalResolvedEventPayloadToolCallCorrelation

class ApprovalResolvedEventPayload(_ApprovalResolvedEventPayloadOptional):
    kind: Literal['approval_resolved']
    approval_id: str
    decision: Literal['approved', 'denied', 'expired']

class UnknownEventPayload(TypedDict):
    kind: str

KnownEventPayload: TypeAlias = (
    TokenEventPayload |
    LlmStepCompletedEventPayload |
    ThoughtEventPayload |
    ToolCallEventPayload |
    LlmDoneEventPayload |
    LlmPromptEventPayload |
    UsageEventPayload |
    RetryEventPayload |
    WarningEventPayload |
    CitationEventPayload |
    ProviderEventEventPayload |
    OperationStartEventPayload |
    OperationEndEventPayload |
    NodeOutputEventPayload |
    NodeMetricsEventPayload |
    ToolStartEventPayload |
    ToolEndEventPayload |
    PlanCreatedEventPayload |
    PlanStepStartedEventPayload |
    PlanStepCompletedEventPayload |
    PlanWorkflowEmittedEventPayload |
    WorkflowStartedEventPayload |
    WorkflowStepStartedEventPayload |
    WorkflowStepCompletedEventPayload |
    WorkflowFinishedEventPayload |
    ExecutionStartedEventPayload |
    ExecuteCompleteEventPayload |
    MemoryReadEventPayload |
    MemoryWriteEventPayload |
    CheckpointSavedEventPayload |
    CheckpointRestoredEventPayload |
    SchedulerDecisionEventPayload |
    ModelRouteDecisionEventPayload |
    AgentRouteDecisionEventPayload |
    HeadOfLineBlockEventPayload |
    GpuUtilizationEventPayload |
    TokenUsageEventPayload |
    MemoizationHitEventPayload |
    ErrorEventPayload |
    AgentSpawnedEventPayload |
    CommunicateDispatchedEventPayload |
    GraphEdgeEventPayload |
    ContextCompactedEventPayload |
    ModelReroutedEventPayload |
    CancelledEventPayload |
    LoopDetectedEventPayload |
    ContextWindowWarningEventPayload |
    SessionStartEventPayload |
    SessionEndEventPayload |
    TurnBoundaryEventPayload |
    TurnStartedEventPayload |
    TurnCompleteEventPayload |
    TurnAbortedEventPayload |
    SubagentSpawnBeginEventPayload |
    SubagentSpawnEndEventPayload |
    SubagentLlmCallBeginEventPayload |
    SubagentLlmCallEndEventPayload |
    ToolCallBeginEventPayload |
    ToolCallEndEventPayload |
    SubagentDoneEventPayload |
    SubagentFailedEventPayload |
    AgentMessageEventPayload |
    ApprovalRequestEventPayload |
    ApprovalResolvedEventPayload
)
EventPayloadValue: TypeAlias = KnownEventPayload | UnknownEventPayload

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
    payload: EventPayloadValue

CORE_EVENT_KINDS: Final[tuple[str, ...]] = (
    'token',
    'thought',
    'tool_call',
    'llm_done',
    'llm_step_completed',
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
