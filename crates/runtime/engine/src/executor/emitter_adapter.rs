//! Bridges [`ExecutionEventEmitter`] to the universal [`EventEmitter`] trait.
//!
//! Converts each typed method call on the execution emitter trait into a
//! concrete core payload struct, wraps it in an [`ApxmEvent`], and forwards it
//! through the pluggable [`EventEmitter`] sink.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use apxm_core::events::payload::*;
use apxm_core::events::{ApxmEvent, EventEmitter, EventSource, SkillEventProvenance};
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use parking_lot::RwLock;

use super::events::{EventScopeState, ExecutionEventEmitter};

/// Adapter that implements [`ExecutionEventEmitter`] by forwarding each call
/// to an [`EventEmitter`] sink as a fully-formed [`ApxmEvent`].
pub struct EmitterAdapter {
    emitter: Arc<dyn EventEmitter>,
    source: EventSource,
    trace_id: String,
    skill_provenance: Option<SkillEventProvenance>,
    seq: AtomicU64,
    /// Current parent span ID for hierarchical event nesting.
    /// Updated by the executor engine when entering/leaving node scopes.
    current_span_id: RwLock<Option<String>>,
    /// Base and overlapping active scope IDs for session isolation.
    scope_state: RwLock<EventScopeState>,
}

impl EmitterAdapter {
    pub fn new(
        emitter: Arc<dyn EventEmitter>,
        source: EventSource,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            emitter,
            source,
            trace_id: trace_id.into(),
            skill_provenance: None,
            seq: AtomicU64::new(0),
            current_span_id: RwLock::new(None),
            scope_state: RwLock::new(EventScopeState::default()),
        }
    }

    /// Attach skill provenance to every subsequently emitted event.
    pub fn with_skill_provenance(mut self, provenance: SkillEventProvenance) -> Self {
        self.skill_provenance = Some(provenance);
        self
    }

    /// Set the current span ID for subsequently emitted events.
    pub fn set_current_span_id(&self, span_id: Option<String>) {
        *self.current_span_id.write() = span_id;
    }

    /// Get the current span ID.
    pub fn current_span_id(&self) -> Option<String> {
        self.current_span_id.read().clone()
    }

    fn emit(&self, payload: impl apxm_core::events::payload::EventPayload) {
        let parent = self.current_span_id.read().clone();
        let scope = self.scope_state.read().current();
        let event = match parent {
            Some(parent_id) => {
                ApxmEvent::child_of(payload, self.source.clone(), &self.trace_id, parent_id)
            }
            None => ApxmEvent::root(payload, self.source.clone(), &self.trace_id),
        };
        let event = event
            .with_scope_id(scope)
            .with_skill_provenance(self.skill_provenance.clone())
            .with_seq(self.seq.fetch_add(1, Ordering::Relaxed));
        self.emitter.emit(event);
    }
}

impl ExecutionEventEmitter for EmitterAdapter {
    fn set_current_span_id(&self, span_id: Option<String>) {
        *self.current_span_id.write() = span_id;
    }

    fn current_span_id(&self) -> Option<String> {
        self.current_span_id.read().clone()
    }

    fn set_current_scope_id(&self, scope_id: Option<String>) {
        self.scope_state.write().set_base(scope_id);
    }

    fn current_scope_id(&self) -> Option<String> {
        self.scope_state.read().current()
    }

    fn enter_scope_id(&self, scope_id: String) {
        self.scope_state.write().enter(scope_id);
    }

    fn leave_scope_id(&self, scope_id: &str) {
        self.scope_state.write().leave(scope_id);
    }

    fn emit_llm_token(&self, content: &str) {
        self.emit(TokenPayload {
            text: content.to_string(),
            generation: None,
        });
    }

    fn emit_llm_token_for_generation(
        &self,
        _node_id: u64,
        content: &str,
        generation: Option<&GenerationIdentity>,
    ) {
        self.emit(TokenPayload {
            text: content.to_string(),
            generation: generation.cloned(),
        });
    }

    fn emit_llm_thought(&self, content: &str) {
        self.emit(ThoughtPayload {
            text: content.to_string(),
            summary: None,
            generation: None,
        });
    }

    fn emit_llm_thought_for_generation(
        &self,
        _node_id: u64,
        content: &str,
        generation: Option<&GenerationIdentity>,
    ) {
        self.emit(ThoughtPayload {
            text: content.to_string(),
            summary: None,
            generation: generation.cloned(),
        });
    }

    fn emit_llm_step_completed(&self, payload: LlmStepCompletedPayload) {
        self.emit(payload);
    }

    fn emit_llm_done(&self, payload: LlmDonePayload) {
        self.emit(payload);
    }

    fn emit_tool_call(&self, payload: ToolCallPayload) {
        self.emit(payload);
    }

    fn emit_tool_start(&self, name: &str, args: &HashMap<String, Value>) {
        let json_args: HashMap<String, serde_json::Value> = args
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.to_json()
                        .unwrap_or_else(|_| serde_json::Value::String(v.to_string())),
                )
            })
            .collect();
        self.emit(ToolStartPayload {
            name: name.to_string(),
            args: json_args,
            tool_call_correlation: None,
        });
    }

    fn emit_tool_start_with_correlation(
        &self,
        name: &str,
        args: &HashMap<String, Value>,
        correlation: Option<&ToolCallCorrelation>,
    ) {
        let json_args = args
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    value
                        .to_json()
                        .unwrap_or_else(|_| serde_json::Value::String(value.to_string())),
                )
            })
            .collect();
        self.emit(ToolStartPayload {
            name: name.to_string(),
            args: json_args,
            tool_call_correlation: correlation.cloned(),
        });
    }

    fn emit_tool_end(&self, name: &str, result: &Value) {
        let result_json = result
            .to_json()
            .unwrap_or_else(|_| serde_json::Value::String(result.to_string()));
        self.emit(ToolEndPayload {
            name: name.to_string(),
            result: result_json,
            tool_call_correlation: None,
        });
    }

    fn emit_tool_end_with_correlation(
        &self,
        name: &str,
        result: &Value,
        correlation: Option<&ToolCallCorrelation>,
    ) {
        self.emit(ToolEndPayload {
            name: name.to_string(),
            result: result
                .to_json()
                .unwrap_or_else(|_| serde_json::Value::String(result.to_string())),
            tool_call_correlation: correlation.cloned(),
        });
    }

    fn emit_operation_start(&self, node_id: u64, op_type: AISOperationType) {
        self.emit(OperationStartPayload {
            node_id,
            op_type,
            context: None,
        });
    }

    fn emit_operation_start_with_context(
        &self,
        node_id: u64,
        op_type: AISOperationType,
        context: serde_json::Value,
    ) {
        self.emit(OperationStartPayload {
            node_id,
            op_type,
            context: Some(context),
        });
    }

    fn emit_agent_spawned(
        &self,
        node_id: u64,
        agent_code: &str,
        parent_execution_id: &str,
        profile: Option<&str>,
        process_id: Option<&str>,
        scope_policy: Option<&str>,
    ) {
        self.emit(AgentSpawnedPayload {
            node_id,
            agent_code: agent_code.to_string(),
            parent_execution_id: parent_execution_id.to_string(),
            profile: profile.map(str::to_string),
            process_id: process_id.map(str::to_string),
            scope_policy: scope_policy.map(str::to_string),
        });
    }

    fn emit_communicate_dispatched(
        &self,
        node_id: u64,
        target_agent: &str,
        protocol: &str,
        message_excerpt: Option<&str>,
    ) {
        self.emit(CommunicateDispatchedPayload {
            node_id,
            target_agent: target_agent.to_string(),
            protocol: protocol.to_string(),
            message_excerpt: message_excerpt.map(str::to_string),
        });
    }

    fn emit_graph_edge(&self, from_node_id: u64, to_node_id: u64, kind: &str) {
        self.emit(GraphEdgePayload {
            from_node_id,
            to_node_id,
            edge_kind: kind.to_string(),
        });
    }

    fn emit_operation_end(
        &self,
        node_id: u64,
        op_type: AISOperationType,
        duration: Duration,
        success: bool,
        _tokens: Option<crate::executor::token_accounting::TokenUsageSummary>,
        _timing: Option<apxm_core::types::TimingBreakdown>,
    ) {
        self.emit(OperationEndPayload {
            node_id,
            op_type,
            duration_ms: duration.as_millis() as u64,
            success,
        });
    }

    fn emit_node_output(&self, node_id: u64, value: &Value) {
        self.emit_node_output_with_name(node_id, None, value);
    }

    fn emit_node_output_with_name(&self, node_id: u64, node_name: Option<&str>, value: &Value) {
        let value = value
            .to_json()
            .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
        self.emit(NodeOutputPayload {
            node_id,
            node_name: node_name.map(str::to_string),
            output: RedactedContent::from_json(&value),
        });
    }

    fn emit_node_metrics(&self, node_id: u64, metrics: &apxm_core::types::NodeMetrics) {
        self.emit_node_metrics_with_name(node_id, None, metrics);
    }

    fn emit_node_metrics_with_name(
        &self,
        node_id: u64,
        node_name: Option<&str>,
        metrics: &apxm_core::types::NodeMetrics,
    ) {
        self.emit(NodeMetricsPayload {
            node_id,
            node_name: node_name.map(str::to_string),
            metrics: metrics.clone(),
        });
    }

    fn emit_llm_prompt(&self, node_id: u64, prompt: &str) {
        self.emit_llm_prompt_with_name(node_id, None, prompt);
    }

    fn emit_llm_prompt_with_name(&self, node_id: u64, node_name: Option<&str>, prompt: &str) {
        self.emit(LlmPromptPayload {
            node_id,
            node_name: node_name.map(str::to_string),
            prompt: RedactedContent::from_text(prompt),
            generation: None,
        });
    }

    fn emit_llm_prompt_with_generation(
        &self,
        node_id: u64,
        node_name: Option<&str>,
        prompt: &str,
        generation: Option<&GenerationIdentity>,
    ) {
        self.emit(LlmPromptPayload {
            node_id,
            node_name: node_name.map(str::to_string),
            prompt: RedactedContent::from_text(prompt),
            generation: generation.cloned(),
        });
    }

    fn emit_plan_created(&self, plan_id: &str, steps: usize) {
        self.emit(PlanCreatedPayload {
            plan_id: plan_id.to_string(),
            steps,
        });
    }

    fn emit_plan_step_started(&self, plan_id: &str, step_index: usize) {
        self.emit(PlanStepStartedPayload {
            plan_id: plan_id.to_string(),
            step_index,
        });
    }

    fn emit_plan_step_completed(&self, plan_id: &str, step_index: usize, success: bool) {
        self.emit(PlanStepCompletedPayload {
            plan_id: plan_id.to_string(),
            step_index,
            success,
        });
    }

    fn emit_plan_workflow_emitted(
        &self,
        plan_id: &str,
        generating_model: &str,
        node_count: usize,
        task_ids: &[u64],
        parallel_fanout_max: usize,
    ) {
        self.emit(PlanWorkflowEmittedPayload {
            plan_id: plan_id.to_string(),
            generating_model: generating_model.to_string(),
            node_count,
            task_ids: task_ids.to_vec(),
            parallel_fanout_max,
        });
    }

    fn emit_workflow_started(&self, workflow_name: &str, session_dir: &str, step_count: usize) {
        self.emit(WorkflowStartedPayload {
            workflow_name: workflow_name.to_string(),
            session_dir: session_dir.to_string(),
            step_count,
        });
    }

    fn emit_workflow_step_started(
        &self,
        workflow_name: &str,
        workflow_session_dir: &str,
        step_id: &str,
        step_index: usize,
        step_count: usize,
    ) {
        self.emit(WorkflowStepStartedPayload {
            workflow_name: workflow_name.to_string(),
            workflow_session_dir: workflow_session_dir.to_string(),
            step_id: step_id.to_string(),
            step_index,
            step_count,
        });
    }

    fn emit_workflow_step_completed(
        &self,
        workflow_name: &str,
        workflow_session_dir: &str,
        step_id: &str,
        step_index: usize,
        status: &str,
        success: bool,
        duration: Duration,
        session_dir: Option<&str>,
        error: Option<&str>,
    ) {
        self.emit(WorkflowStepCompletedPayload {
            workflow_name: workflow_name.to_string(),
            workflow_session_dir: workflow_session_dir.to_string(),
            step_id: step_id.to_string(),
            step_index,
            status: status.to_string(),
            success,
            duration_ms: duration.as_millis() as u64,
            session_dir: session_dir.map(str::to_string),
            error: error.map(str::to_string),
        });
    }

    fn emit_workflow_finished(
        &self,
        workflow_name: &str,
        session_dir: &str,
        status: &str,
        success: bool,
        duration: Duration,
        step_count: usize,
    ) {
        self.emit(WorkflowFinishedPayload {
            workflow_name: workflow_name.to_string(),
            session_dir: session_dir.to_string(),
            status: status.to_string(),
            success,
            duration_ms: duration.as_millis() as u64,
            step_count,
        });
    }

    fn emit_memory_read(&self, scope: &str, key: &str) {
        self.emit(MemoryReadPayload {
            scope: scope.to_string(),
            key: key.to_string(),
        });
    }

    fn emit_memory_write(&self, scope: &str, key: &str) {
        self.emit(MemoryWritePayload {
            scope: scope.to_string(),
            key: key.to_string(),
        });
    }

    fn emit_checkpoint_saved(&self, checkpoint_id: &str) {
        self.emit(CheckpointSavedPayload {
            checkpoint_id: checkpoint_id.to_string(),
        });
    }

    fn emit_checkpoint_restored(&self, checkpoint_id: &str) {
        self.emit(CheckpointRestoredPayload {
            checkpoint_id: checkpoint_id.to_string(),
        });
    }

    fn emit_scheduler_decision(&self, node_id: u64, delay: Duration, reason: &str) {
        self.emit(SchedulerDecisionPayload {
            node_id,
            delay_ms: delay.as_millis() as u64,
            reason: reason.to_string(),
        });
    }

    fn emit_warning(&self, code: &str, message: &str) {
        self.emit(WarningPayload {
            code: code.to_string(),
            message: message.to_string(),
        });
    }

    fn emit_model_route_decision(
        &self,
        backend: &str,
        model: Option<&str>,
        was_failover: bool,
        reason: &str,
        rejected_candidates: &[(String, String, &'static str, String)],
    ) {
        self.emit(ModelRouteDecisionPayload {
            backend: backend.to_string(),
            model: model.map(str::to_string),
            was_failover,
            reason: reason.to_string(),
            rejected_candidates: rejected_candidates
                .iter()
                .map(
                    |(candidate, backend, reason_kind, reason)| ModelRouteRejectionPayload {
                        candidate: candidate.clone(),
                        backend: backend.clone(),
                        reason_kind: (*reason_kind).to_string(),
                        reason: reason.clone(),
                    },
                )
                .collect(),
        });
    }

    fn emit_agent_route_decision(
        &self,
        id: &str,
        profile: Option<&str>,
        source: &str,
        reason: &str,
        required_capabilities: &[String],
        rejected_candidates: &[(String, Vec<String>, String)],
    ) {
        self.emit(AgentRouteDecisionPayload {
            id: id.to_string(),
            profile: profile.map(str::to_string),
            source: source.to_string(),
            reason: reason.to_string(),
            required_capabilities: required_capabilities.to_vec(),
            rejected_candidates: rejected_candidates
                .iter()
                .map(
                    |(profile, missing_capabilities, reason)| AgentRouteRejectionPayload {
                        profile: profile.clone(),
                        missing_capabilities: missing_capabilities.clone(),
                        reason: reason.clone(),
                    },
                )
                .collect(),
        });
    }

    fn emit_head_of_line_block(
        &self,
        blocker_node: u64,
        blocked_node: u64,
        wait_ms: u64,
        reason: &str,
    ) {
        self.emit(apxm_core::events::payload::HeadOfLineBlockPayload {
            blocker_node,
            blocked_node,
            wait_ms,
            reason: reason.to_string(),
        });
    }

    fn emit_gpu_utilization(&self, gpu_id: u32, utilization_pct: f32, memory_pct: f32) {
        self.emit(GpuUtilizationPayload {
            gpu_id,
            utilization_pct,
            memory_pct,
        });
    }

    fn emit_token_usage(&self, node_id: u64, input_tokens: usize, output_tokens: usize) {
        self.emit_token_usage_with_generation(node_id, input_tokens, output_tokens, None);
    }

    fn emit_token_usage_with_generation(
        &self,
        node_id: u64,
        input_tokens: usize,
        output_tokens: usize,
        generation: Option<&GenerationIdentity>,
    ) {
        self.emit(TokenUsagePayload {
            node_id,
            input_tokens,
            output_tokens,
            generation: generation.cloned(),
        });
    }

    fn emit_memoization_hit(&self, node_id: u64) {
        self.emit(MemoizationHitPayload { node_id });
    }

    fn emit_context_compacted(&self, original_tokens: usize, new_tokens: usize) {
        self.emit(ContextCompactedPayload {
            original_tokens,
            new_tokens,
        });
    }

    fn emit_context_window_warning(
        &self,
        current_tokens: usize,
        max_tokens: usize,
        utilization_pct: f64,
    ) {
        self.emit(ContextWindowWarningPayload {
            current_tokens,
            max_tokens,
            utilization_pct,
        });
    }

    fn emit_turn_boundary(&self, payload: TurnBoundaryPayload) {
        self.emit(payload);
    }

    fn emit_approval_request(
        &self,
        agent_code: &str,
        tool_name: &str,
        approval_id: &str,
        risk_level: ApprovalRiskLevel,
    ) {
        self.emit(ApprovalRequestPayload {
            agent_code: agent_code.to_string(),
            tool_name: tool_name.to_string(),
            approval_id: approval_id.to_string(),
            risk_level,
            tool_call_correlation: None,
        });
    }

    fn emit_approval_request_with_correlation(
        &self,
        agent_code: &str,
        tool_name: &str,
        approval_id: &str,
        risk_level: ApprovalRiskLevel,
        correlation: Option<&ToolCallCorrelation>,
    ) {
        self.emit(ApprovalRequestPayload {
            agent_code: agent_code.to_string(),
            tool_name: tool_name.to_string(),
            approval_id: approval_id.to_string(),
            risk_level,
            tool_call_correlation: correlation.cloned(),
        });
    }

    fn emit_approval_resolved(
        &self,
        approval_id: &str,
        decision: apxm_core::types::consent::ApprovalResolution,
    ) {
        self.emit(ApprovalResolvedPayload {
            approval_id: approval_id.to_string(),
            decision,
            tool_call_correlation: None,
        });
    }

    fn emit_approval_resolved_with_correlation(
        &self,
        approval_id: &str,
        decision: apxm_core::types::consent::ApprovalResolution,
        correlation: Option<&ToolCallCorrelation>,
    ) {
        self.emit(ApprovalResolvedPayload {
            approval_id: approval_id.to_string(),
            decision,
            tool_call_correlation: correlation.cloned(),
        });
    }

    // ── Layer 2 — agent-layer hooks ────────────────────────────────
    //
    // Every call site in `executor/engine.rs`, `executor/dispatcher.rs`,
    // and `executor/handlers/spawn_agent.rs` already fires these; this
    // adapter is the production bridge to `ApxmEvent`/SSE, so leaving
    // them at the trait's no-op default silently drops every Layer-2
    // event before it reaches any consumer.

    fn emit_turn_started(
        &self,
        execution_id: &str,
        turn_id: Option<&str>,
        coordinator_label: Option<&str>,
    ) {
        self.emit(TurnStartedPayload {
            execution_id: execution_id.to_string(),
            turn_id: turn_id.map(str::to_string),
            coordinator_label: coordinator_label.map(str::to_string),
        });
    }

    fn emit_turn_complete(&self, execution_id: &str, duration_ms: u64, had_answer: bool) {
        self.emit(TurnCompletePayload {
            execution_id: execution_id.to_string(),
            duration_ms,
            had_answer,
        });
    }

    fn emit_turn_aborted(
        &self,
        execution_id: &str,
        duration_ms: u64,
        reason: &str,
        error_message_safe: Option<&str>,
    ) {
        self.emit(TurnAbortedPayload {
            execution_id: execution_id.to_string(),
            duration_ms,
            reason: reason.to_string(),
            error_message_safe: error_message_safe.map(str::to_string),
        });
    }

    fn emit_subagent_spawn_begin(
        &self,
        agent_code: &str,
        agent_name: Option<&str>,
        agent_type: Option<&str>,
        module_key: Option<&str>,
        autonomy_policy: Option<&str>,
        parent_span_id: Option<&str>,
    ) {
        self.emit(SubagentSpawnBeginPayload {
            agent_code: agent_code.to_string(),
            agent_name: agent_name.map(str::to_string),
            agent_type: agent_type.map(str::to_string),
            module_key: module_key.map(str::to_string),
            autonomy_policy: autonomy_policy.map(str::to_string),
            parent_span_id: parent_span_id.map(str::to_string),
        });
    }

    fn emit_subagent_spawn_end(&self, agent_code: &str) {
        self.emit(SubagentSpawnEndPayload {
            agent_code: agent_code.to_string(),
        });
    }

    fn emit_subagent_llm_call_begin(
        &self,
        agent_code: &str,
        model: &str,
        backend: &str,
        tool_manifest_count: usize,
    ) {
        self.emit(SubagentLlmCallBeginPayload {
            agent_code: agent_code.to_string(),
            model: model.to_string(),
            backend: backend.to_string(),
            tool_manifest_count,
            generation: None,
        });
    }

    fn emit_subagent_llm_call_begin_with_generation(
        &self,
        agent_code: &str,
        model: &str,
        backend: &str,
        tool_manifest_count: usize,
        generation: Option<&GenerationIdentity>,
    ) {
        self.emit(SubagentLlmCallBeginPayload {
            agent_code: agent_code.to_string(),
            model: model.to_string(),
            backend: backend.to_string(),
            tool_manifest_count,
            generation: generation.cloned(),
        });
    }

    fn emit_subagent_llm_call_end(
        &self,
        agent_code: &str,
        finish_reason: &str,
        input_tokens: usize,
        output_tokens: usize,
        content_len: usize,
    ) {
        self.emit(SubagentLlmCallEndPayload {
            agent_code: agent_code.to_string(),
            finish_reason: finish_reason.to_string(),
            usage: UsagePayload {
                input_tokens,
                output_tokens,
                generation: None,
            },
            content_len,
            generation: None,
        });
    }

    fn emit_subagent_llm_call_end_with_generation(
        &self,
        agent_code: &str,
        finish_reason: &str,
        input_tokens: usize,
        output_tokens: usize,
        content_len: usize,
        generation: Option<&GenerationIdentity>,
    ) {
        self.emit(SubagentLlmCallEndPayload {
            agent_code: agent_code.to_string(),
            finish_reason: finish_reason.to_string(),
            usage: UsagePayload {
                input_tokens,
                output_tokens,
                generation: None,
            },
            content_len,
            generation: generation.cloned(),
        });
    }

    fn emit_tool_call_begin(&self, agent_code: &str, tool_name: &str, argument_keys: &[String]) {
        self.emit(ToolCallBeginPayload {
            agent_code: agent_code.to_string(),
            tool_name: tool_name.to_string(),
            argument_keys: argument_keys.to_vec(),
            tool_call_correlation: None,
        });
    }

    fn emit_tool_call_begin_with_correlation(
        &self,
        agent_code: &str,
        tool_name: &str,
        argument_keys: &[String],
        correlation: Option<&ToolCallCorrelation>,
    ) {
        self.emit(ToolCallBeginPayload {
            agent_code: agent_code.to_string(),
            tool_name: tool_name.to_string(),
            argument_keys: argument_keys.to_vec(),
            tool_call_correlation: correlation.cloned(),
        });
    }

    fn emit_tool_call_end(
        &self,
        agent_code: &str,
        tool_name: &str,
        result_keys: &[String],
        status: ToolCallStatus,
        latency_ms: u64,
    ) {
        self.emit(ToolCallEndPayload {
            agent_code: agent_code.to_string(),
            tool_name: tool_name.to_string(),
            result_keys: result_keys.to_vec(),
            status,
            latency_ms,
            tool_call_correlation: None,
        });
    }

    fn emit_tool_call_end_with_correlation(
        &self,
        agent_code: &str,
        tool_name: &str,
        result_keys: &[String],
        status: ToolCallStatus,
        latency_ms: u64,
        correlation: Option<&ToolCallCorrelation>,
    ) {
        self.emit(ToolCallEndPayload {
            agent_code: agent_code.to_string(),
            tool_name: tool_name.to_string(),
            result_keys: result_keys.to_vec(),
            status,
            latency_ms,
            tool_call_correlation: correlation.cloned(),
        });
    }

    fn emit_subagent_done(
        &self,
        agent_code: &str,
        total_tool_calls: usize,
        input_tokens_total: usize,
        output_tokens_total: usize,
        evidence_excerpt: Option<&str>,
    ) {
        self.emit(SubagentDonePayload {
            agent_code: agent_code.to_string(),
            total_tool_calls,
            usage_total: UsagePayload {
                input_tokens: input_tokens_total,
                output_tokens: output_tokens_total,
                generation: None,
            },
            evidence_excerpt: evidence_excerpt.map(str::to_string),
        });
    }

    fn emit_subagent_failed(&self, agent_code: &str, error_class: &str, error_message_safe: &str) {
        self.emit(SubagentFailedPayload {
            agent_code: agent_code.to_string(),
            error_class: error_class.to_string(),
            error_message_safe: error_message_safe.to_string(),
        });
    }

    fn emit_agent_message(
        &self,
        text: &str,
        item_id: Option<&str>,
        response_id: Option<&str>,
        input_tokens: Option<usize>,
        output_tokens: Option<usize>,
    ) {
        let usage = match (input_tokens, output_tokens) {
            (Some(input_tokens), Some(output_tokens)) => Some(UsagePayload {
                input_tokens,
                output_tokens,
                generation: None,
            }),
            _ => None,
        };
        self.emit(AgentMessagePayload {
            text: text.to_string(),
            item_id: item_id.map(str::to_string),
            response_id: response_id.map(str::to_string),
            usage,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex as PlMutex;

    /// Captures every emitted `ApxmEvent` so tests can inspect the decoded
    /// payload without a real rollout/SSE sink.
    #[derive(Default)]
    struct CapturingEmitter {
        events: PlMutex<Vec<ApxmEvent>>,
    }

    impl EventEmitter for CapturingEmitter {
        fn emit(&self, event: ApxmEvent) {
            self.events.lock().push(event);
        }
    }

    fn adapter_with_capture() -> (EmitterAdapter, Arc<CapturingEmitter>) {
        let capture = Arc::new(CapturingEmitter::default());
        let adapter = EmitterAdapter::new(
            capture.clone() as Arc<dyn EventEmitter>,
            EventSource::Runtime,
            "trace-rtg11",
        );
        (adapter, capture)
    }

    #[test]
    fn out_of_order_scope_exit_preserves_the_newer_flow_scope() {
        let (adapter, capture) = adapter_with_capture();
        adapter.set_current_scope_id(Some("root".to_string()));
        adapter.enter_scope_id("turn-1".to_string());
        adapter.enter_scope_id("turn-2".to_string());

        adapter.leave_scope_id("turn-1");
        adapter.emit_llm_token("turn two");
        adapter.leave_scope_id("turn-2");
        adapter.emit_llm_token("root");

        let events = capture.events.lock();
        assert_eq!(events[0].meta.scope_id.as_deref(), Some("turn-2"));
        assert_eq!(events[1].meta.scope_id.as_deref(), Some("root"));
    }

    /// a model-routing decision becomes a `model_route_decision`
    /// event whose payload carries the chosen backend/model, the reason,
    /// and every rejected candidate with its own reason — nothing silently
    /// dropped.
    #[test]
    fn emit_model_route_decision_carries_chosen_reason_and_rejected_candidates() {
        let (adapter, capture) = adapter_with_capture();
        let rejected = vec![(
            "gpt-slow".to_string(),
            "openai".to_string(),
            "circuit_breaker_open",
            "backend 'openai' circuit breaker is open".to_string(),
        )];
        adapter.emit_model_route_decision(
            "vllm",
            Some("llama-70b"),
            false,
            "cost target selected 'llama-70b' (backend 'vllm')",
            &rejected,
        );

        let events = capture.events.lock();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.kind().name(), "model_route_decision");
        let payload = event
            .payload
            .downcast_ref::<ModelRouteDecisionPayload>()
            .expect("model route decision payload");
        assert_eq!(payload.backend, "vllm");
        assert_eq!(payload.model.as_deref(), Some("llama-70b"));
        assert!(!payload.was_failover);
        assert_eq!(
            payload.reason,
            "cost target selected 'llama-70b' (backend 'vllm')"
        );
        assert_eq!(payload.rejected_candidates.len(), 1);
        assert_eq!(payload.rejected_candidates[0].candidate, "gpt-slow");
        assert_eq!(
            payload.rejected_candidates[0].reason_kind,
            "circuit_breaker_open"
        );
        assert_eq!(
            payload.rejected_candidates[0].reason,
            "backend 'openai' circuit breaker is open"
        );
    }

    /// an agent-routing decision becomes an `agent_route_decision`
    /// event whose payload carries the chosen profile, the reason, and
    /// every rejected candidate with its own missing-capability reason.
    #[test]
    fn emit_agent_route_decision_carries_chosen_reason_and_rejected_candidates() {
        let (adapter, capture) = adapter_with_capture();
        let rejected = vec![(
            "reviewer-profile".to_string(),
            vec!["execute".to_string()],
            "missing required capabilities [execute]".to_string(),
        )];
        adapter.emit_agent_route_decision(
            "worker-1",
            Some("planner-profile"),
            "selected",
            "selected least-used eligible profile 'planner-profile' with capability fit score 0",
            &["planner".to_string(), "read".to_string()],
            &rejected,
        );

        let events = capture.events.lock();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.kind().name(), "agent_route_decision");
        let payload = event
            .payload
            .downcast_ref::<AgentRouteDecisionPayload>()
            .expect("agent route decision payload");
        assert_eq!(payload.id, "worker-1");
        assert_eq!(payload.profile.as_deref(), Some("planner-profile"));
        assert_eq!(payload.source, "selected");
        assert_eq!(payload.required_capabilities, vec!["planner", "read"]);
        assert_eq!(payload.rejected_candidates.len(), 1);
        assert_eq!(payload.rejected_candidates[0].profile, "reviewer-profile");
        assert_eq!(
            payload.rejected_candidates[0].missing_capabilities,
            vec!["execute".to_string()]
        );
        assert_eq!(
            payload.rejected_candidates[0].reason,
            "missing required capabilities [execute]"
        );
    }

    /// Positive: every Layer-2 hook on `EmitterAdapter` now produces a real
    /// `ApxmEvent` of the matching kind instead of silently no-op'ing —
    /// the wiring gap called out in the event taxonomy correction defect 1.
    #[test]
    fn emitter_adapter_delivers_all_layer2_kinds() {
        let (adapter, capture) = adapter_with_capture();

        adapter.emit_turn_started("exec-1", Some("turn-1"), Some("Cleo"));
        adapter.emit_turn_complete("exec-1", 100, true);
        adapter.emit_turn_boundary(TurnBoundaryPayload {
            turn_number: 1,
            direction: TurnDirection::Request,
        });
        adapter.emit_llm_step_completed(LlmStepCompletedPayload {
            node_id: 1,
            step_number: 1,
            model: "model".to_string(),
            finish_reason: FinishReasonPayload {
                reason: "stop".to_string(),
            },
            usage: LlmStepUsagePayload {
                input_tokens: 1,
                output_tokens: 2,
                cached_input_tokens: 0,
                reasoning_output_tokens: 0,
            },
            performance: LlmStepPerformancePayload {
                latency_ms: 10.0,
                prefill_ms: 4.0,
                decode_ms: 6.0,
            },
            tool_call_count: 0,
            generation: None,
        });
        adapter.emit_llm_done(LlmDonePayload {
            content: "final answer".to_string(),
            model: "model".to_string(),
            finish_reason: FinishReasonPayload {
                reason: "stop".to_string(),
            },
            usage: UsagePayload {
                input_tokens: 1,
                output_tokens: 2,
                generation: None,
            },
            tool_calls: Vec::new(),
            response_id: None,
            generation: None,
        });
        adapter.emit_subagent_spawn_begin(
            "agent-1",
            Some("Agent One"),
            None,
            None,
            None,
            Some("span-0"),
        );
        adapter.emit_subagent_spawn_end("agent-1");
        adapter.emit_tool_call_begin("agent-1", "web_search", &["q".to_string()]);
        adapter.emit_tool_call_end(
            "agent-1",
            "web_search",
            &["r".to_string()],
            ToolCallStatus::Ok,
            12,
        );
        adapter.emit_agent_message("final answer", None, None, Some(1), Some(2));

        let events = capture.events.lock();
        let kinds: Vec<&'static str> = events.iter().map(|e| e.kind().name()).collect();
        for expected in [
            "turn_started",
            "turn_complete",
            "turn_boundary",
            "llm_step_completed",
            "llm_done",
            "subagent_spawn_begin",
            "subagent_spawn_end",
            "tool_call_begin",
            "tool_call_end",
            "agent_message",
        ] {
            assert!(
                kinds.contains(&expected),
                "expected {expected} to be delivered, got {kinds:?}"
            );
        }
    }

    /// Recovery/no-regression: the approval hooks that already worked
    /// before Layer-2 wiring still deliver after adding the new overrides.
    #[test]
    fn approval_events_still_delivered_after_layer2_wiring() {
        let (adapter, capture) = adapter_with_capture();

        adapter.emit_approval_request("agent-1", "shell", "appr-1", ApprovalRiskLevel::High);
        adapter.emit_approval_resolved(
            "appr-1",
            apxm_core::types::consent::ApprovalResolution::Approved,
        );

        let events = capture.events.lock();
        let kinds: Vec<&'static str> = events.iter().map(|e| e.kind().name()).collect();
        assert_eq!(kinds, vec!["approval_request", "approval_resolved"]);
    }

    #[test]
    fn token_usage_preserves_generation_identity() {
        let (adapter, capture) = adapter_with_capture();
        let generation = GenerationIdentity::new("call-usage", 2, 4);

        adapter.emit_token_usage_with_generation(7, 11, 13, Some(&generation));

        let events = capture.events.lock();
        assert_eq!(events.len(), 1);
        let payload = events[0]
            .payload
            .downcast_ref::<TokenUsagePayload>()
            .expect("token_usage payload");
        assert_eq!(payload.node_id, 7);
        assert_eq!(payload.input_tokens, 11);
        assert_eq!(payload.output_tokens, 13);
        assert_eq!(payload.generation.as_ref(), Some(&generation));
    }

    /// **Compaction event-delivery verification:** `CONTEXT_COMPACTED`/
    /// `CONTEXT_WINDOW_WARNING` are two of the event kinds
    /// the runtime compaction invariant requires "must not silently no-op through
    /// `EmitterAdapter`" — proves both reach the sink with the REAL payload
    /// field names (`original_tokens`/`new_tokens`,
    /// `current_tokens`/`max_tokens`/`utilization_pct`), not the earlier
    /// draft's nonexistent `tokens_before`/`tokens_after`.
    #[test]
    fn context_compacted_and_window_warning_are_not_swallowed_by_emitter_adapter() {
        let (adapter, capture) = adapter_with_capture();

        adapter.emit_context_window_warning(240, 300, 80.0);
        adapter.emit_context_compacted(320, 90);

        let events = capture.events.lock();
        assert_eq!(events.len(), 2);

        assert_eq!(events[0].kind().name(), "context_window_warning");
        let warning = events[0]
            .payload
            .downcast_ref::<ContextWindowWarningPayload>()
            .expect("context_window_warning payload");
        assert_eq!(warning.current_tokens, 240);
        assert_eq!(warning.max_tokens, 300);
        assert_eq!(warning.utilization_pct, 80.0);

        assert_eq!(events[1].kind().name(), "context_compacted");
        let compacted = events[1]
            .payload
            .downcast_ref::<ContextCompactedPayload>()
            .expect("context_compacted payload");
        assert_eq!(compacted.original_tokens, 320);
        assert_eq!(compacted.new_tokens, 90);
        assert!(
            compacted.original_tokens > compacted.new_tokens,
            "compaction must reduce the token count"
        );
    }
}
