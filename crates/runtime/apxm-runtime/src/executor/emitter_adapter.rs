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
use apxm_core::events::{ApxmEvent, EventEmitter, EventSource};
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
            seq: AtomicU64::new(0),
            current_span_id: RwLock::new(None),
            current_scope_id: RwLock::new(None),
        }
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
        self.emit(OperationStartPayload { node_id, op_type });
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
        let value = value
            .to_json()
            .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
        self.emit(NodeOutputPayload { node_id, value });
    }

    fn emit_node_metrics(&self, node_id: u64, metrics: &apxm_core::types::NodeMetrics) {
        self.emit(NodeMetricsPayload {
            node_id,
            metrics: metrics.clone(),
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

    use apxm_core::events::payload::{NodeMetricsPayload, NodeOutputPayload};
    use apxm_core::events::{ChannelEmitter, EventSource};
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

        adapter.emit_node_output(42, &Value::String("ok".to_string()));

        let event = rx.recv().expect("node output event");
        assert_eq!(event.kind().name(), "node_output");
        assert_eq!(event.meta.seq, 0);
        assert_eq!(event.meta.trace_id, "trace-node-output");
        assert_eq!(event.meta.parent_span_id.as_deref(), Some("parent-span"));
        assert_eq!(event.meta.scope_id.as_deref(), Some("scope-1"));

        let payload = event
            .payload
            .downcast_ref::<NodeOutputPayload>()
            .expect("node output payload");
        assert_eq!(payload.node_id, 42);
        assert_eq!(payload.value, serde_json::json!("ok"));
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

        adapter.emit_node_metrics(42, &metrics);

        let event = rx.recv().expect("node metrics event");
        assert_eq!(event.kind().name(), "node_metrics");
        assert_eq!(event.meta.seq, 0);
        assert_eq!(event.meta.trace_id, "trace-node-metrics");
        assert_eq!(event.meta.parent_span_id.as_deref(), Some("parent-span"));
        assert_eq!(event.meta.scope_id.as_deref(), Some("scope-1"));

        let payload = event
            .payload
            .downcast_ref::<NodeMetricsPayload>()
            .expect("node metrics payload");
        assert_eq!(payload.node_id, 42);
        assert_eq!(payload.metrics.operation.attempts, 1);
        assert_eq!(payload.metrics.operation.successes, 1);
    }
}
