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

use super::events::ExecutionEventEmitter;

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
    /// Current scope ID for session isolation.
    /// Updated when entering/leaving scoped sub-flows.
    current_scope_id: RwLock<Option<String>>,
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
            current_scope_id: RwLock::new(None),
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
        let scope = self.current_scope_id.read().clone();
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
        *self.current_scope_id.write() = scope_id;
    }

    fn current_scope_id(&self) -> Option<String> {
        self.current_scope_id.read().clone()
    }

    fn emit_llm_token(&self, content: &str) {
        self.emit(TokenPayload {
            text: content.to_string(),
        });
    }

    fn emit_llm_thought(&self, content: &str) {
        self.emit(ThoughtPayload {
            text: content.to_string(),
            summary: None,
        });
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
        });
    }

    fn emit_tool_end(&self, name: &str, result: &Value) {
        let result_json = result
            .to_json()
            .unwrap_or_else(|_| serde_json::Value::String(result.to_string()));
        self.emit(ToolEndPayload {
            name: name.to_string(),
            result: result_json,
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
            kind: kind.to_string(),
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

    fn emit_plan_graph_emitted(
        &self,
        plan_id: &str,
        generating_model: &str,
        node_count: usize,
        task_ids: &[u64],
        parallel_fanout_max: usize,
    ) {
        self.emit(PlanGraphEmittedPayload {
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
        self.emit(TokenUsagePayload {
            node_id,
            input_tokens,
            output_tokens,
        });
    }

    fn emit_memoization_hit(&self, node_id: u64) {
        self.emit(MemoizationHitPayload { node_id });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use apxm_core::events::payload::{LlmPromptPayload, NodeMetricsPayload, NodeOutputPayload};
    use apxm_core::events::{ChannelEmitter, EventSource, SkillEventProvenance, kind};
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::{NodeMetrics, OperationMetric};

    use super::*;

    #[test]
    fn emitter_adapter_forwards_node_output_events() {
        let (tx, rx) = mpsc::channel();
        let adapter = EmitterAdapter::new(
            Arc::new(ChannelEmitter::new(tx)),
            EventSource::Runtime,
            "trace-node-output",
        );
        adapter.set_current_span_id(Some("parent-span".to_string()));
        adapter.set_current_scope_id(Some("scope-1".to_string()));

        adapter.emit_node_output_with_name(
            42,
            Some("format_result"),
            &Value::String("ok".to_string()),
        );

        let event = rx.recv().expect("node output event");
        assert_eq!(event.kind(), kind::NODE_OUTPUT);
        assert_eq!(event.meta.seq, 0);
        assert_eq!(event.meta.trace_id, "trace-node-output");
        assert_eq!(event.meta.parent_span_id.as_deref(), Some("parent-span"));
        assert_eq!(event.meta.scope_id.as_deref(), Some("scope-1"));

        let payload = event
            .payload
            .downcast_ref::<NodeOutputPayload>()
            .expect("node output payload");
        assert_eq!(payload.node_id, 42);
        assert_eq!(payload.node_name.as_deref(), Some("format_result"));
        assert!(payload.output.redacted);
        assert_eq!(payload.output.summary, "string(chars=2)");
        assert!(!payload.output.hash.is_empty());
        let event_json = serde_json::to_string(&event).expect("serialize event");
        assert!(!event_json.contains("ok"));
    }

    #[test]
    fn emitter_adapter_forwards_node_metrics_events() {
        let (tx, rx) = mpsc::channel();
        let adapter = EmitterAdapter::new(
            Arc::new(ChannelEmitter::new(tx)),
            EventSource::Runtime,
            "trace-node-metrics",
        );
        adapter.set_current_span_id(Some("parent-span".to_string()));
        adapter.set_current_scope_id(Some("scope-1".to_string()));
        let mut metrics = NodeMetrics::new(42);
        metrics.record_operation(OperationMetric {
            node_id: 42,
            op_type: AISOperationType::ConstStr,
            duration_ms: 9,
            success: true,
        });

        adapter.emit_node_metrics_with_name(42, Some("const_node"), &metrics);

        let event = rx.recv().expect("node metrics event");
        assert_eq!(event.kind(), kind::NODE_METRICS);
        assert_eq!(event.meta.seq, 0);
        assert_eq!(event.meta.trace_id, "trace-node-metrics");
        assert_eq!(event.meta.parent_span_id.as_deref(), Some("parent-span"));
        assert_eq!(event.meta.scope_id.as_deref(), Some("scope-1"));

        let payload = event
            .payload
            .downcast_ref::<NodeMetricsPayload>()
            .expect("node metrics payload");
        assert_eq!(payload.node_id, 42);
        assert_eq!(payload.node_name.as_deref(), Some("const_node"));
        assert_eq!(payload.metrics.operation.attempts, 1);
        assert_eq!(payload.metrics.operation.successes, 1);
    }

    #[test]
    fn emitter_adapter_forwards_redacted_llm_prompt_events() {
        let (tx, rx) = mpsc::channel();
        let adapter = EmitterAdapter::new(
            Arc::new(ChannelEmitter::new(tx)),
            EventSource::Runtime,
            "trace-llm-prompt",
        )
        .with_skill_provenance(SkillEventProvenance {
            skill_id: "checkout-context-triage".to_string(),
            skill_version: "0.1.0".to_string(),
            parent_skill_id: None,
            parent_execution_id: None,
            flow_name: Some("main".to_string()),
        });
        adapter.set_current_span_id(Some("parent-span".to_string()));
        adapter.set_current_scope_id(Some("scope-1".to_string()));

        adapter.emit_llm_prompt_with_name(7, Some("ask_node"), "secret customer prompt");

        let event = rx.recv().expect("llm prompt event");
        assert_eq!(event.kind(), kind::LLM_PROMPT);
        assert_eq!(event.meta.seq, 0);
        assert_eq!(event.meta.trace_id, "trace-llm-prompt");
        assert_eq!(event.meta.parent_span_id.as_deref(), Some("parent-span"));
        assert_eq!(event.meta.scope_id.as_deref(), Some("scope-1"));
        let skill = event.meta.skill.as_ref().expect("skill provenance");
        assert_eq!(skill.skill_id, "checkout-context-triage");
        assert_eq!(skill.skill_version, "0.1.0");
        assert_eq!(skill.flow_name.as_deref(), Some("main"));

        let payload = event
            .payload
            .downcast_ref::<LlmPromptPayload>()
            .expect("llm prompt payload");
        assert_eq!(payload.node_id, 7);
        assert_eq!(payload.node_name.as_deref(), Some("ask_node"));
        assert!(payload.prompt.redacted);
        assert_eq!(payload.prompt.summary, "text(chars=22)");
        let event_json = serde_json::to_string(&event).expect("serialize event");
        assert!(!event_json.contains("secret customer prompt"));
    }
}
