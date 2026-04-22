//! Bridges [`ExecutionEventEmitter`] to the universal [`EventEmitter`] trait.
//!
//! Converts each typed method call on the legacy emitter trait into a concrete
//! core payload struct, wraps it in an [`ApxmEvent`], and forwards it through
//! the pluggable [`EventEmitter`] sink.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use apxm_core::events::payload::*;
use apxm_core::events::{ApxmEvent, EventEmitter, EventSource};
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

    fn emit_operation_start(&self, node_id: u64, op_type: &str) {
        self.emit(OperationStartPayload {
            node_id,
            op_type: op_type.to_string(),
        });
    }

    fn emit_operation_end(
        &self,
        node_id: u64,
        op_type: &str,
        duration: Duration,
        success: bool,
        _tokens: Option<crate::executor::token_accounting::TokenUsageSummary>,
        _timing: Option<apxm_core::types::TimingBreakdown>,
    ) {
        self.emit(OperationEndPayload {
            node_id,
            op_type: op_type.to_string(),
            duration_ms: duration.as_millis() as u64,
            success,
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
