//! Execution event emission hooks.

use std::collections::HashMap;
use std::time::Duration;

use apxm_core::types::NodeMetrics;
use apxm_core::types::TimingBreakdown;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use serde::{Deserialize, Serialize};

use crate::executor::token_accounting::TokenUsageSummary;

/// Runtime execution event enum for simple emitters and tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEvent {
    // ── Existing (3) ────────────────────────────────────────────────
    LlmToken {
        content: String,
    },
    ToolStart {
        name: String,
        args: HashMap<String, Value>,
    },
    ToolEnd {
        name: String,
        result: Value,
    },

    // ── Operation lifecycle ─────────────────────────────────────────
    OperationStart {
        node_id: u64,
        op_type: AISOperationType,
    },
    OperationEnd {
        node_id: u64,
        op_type: AISOperationType,
        #[serde(with = "duration_millis")]
        duration: Duration,
        success: bool,
    },

    // ── Planning events ─────────────────────────────────────────────
    PlanCreated {
        plan_id: String,
        steps: usize,
    },
    PlanStepStarted {
        plan_id: String,
        step_index: usize,
    },
    PlanStepCompleted {
        plan_id: String,
        step_index: usize,
        success: bool,
    },
    PlanGraphEmitted {
        plan_id: String,
        generating_model: String,
        node_count: usize,
        task_ids: Vec<u64>,
        parallel_fanout_max: usize,
    },

    // ── Memory events ───────────────────────────────────────────────
    MemoryRead {
        scope: String,
        key: String,
    },
    MemoryWrite {
        scope: String,
        key: String,
    },

    // ── Checkpoint events ───────────────────────────────────────────
    CheckpointSaved {
        checkpoint_id: String,
    },
    CheckpointRestored {
        checkpoint_id: String,
    },

    // ── Scheduler / timing events ───────────────────────────────────
    SchedulerDecision {
        node_id: u64,
        #[serde(with = "duration_millis")]
        delay: Duration,
        reason: String,
    },

    // ── Hardware events ───────────────────────────────────────────────
    GpuUtilization {
        gpu_id: u32,
        utilization_pct: f32,
        memory_pct: f32,
    },

    // ── Token accounting events ────────────────────────────────────
    TokenUsage {
        node_id: u64,
        input_tokens: usize,
        output_tokens: usize,
    },
    // ── Memoization events ─────────────────────────────────────────
    MemoizationHit {
        node_id: u64,
    },
}

/// Serde helper: serialize/deserialize `Duration` as integer milliseconds.
mod duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

/// Optional observer for execution events.
///
/// Implementors receive fine-grained lifecycle callbacks during DAG
/// execution.  Every method has a default no-op implementation so
/// consumers only need to override the events they care about.
pub trait ExecutionEventEmitter: Send + Sync {
    // ── Span hierarchy ─────────────────────────────────────────────
    /// Set the current parent span ID for subsequently emitted events.
    fn set_current_span_id(&self, _span_id: Option<String>) {}

    /// Get the current parent span ID.
    fn current_span_id(&self) -> Option<String> {
        None
    }

    // ── Scope isolation ───────────────────────────────────────────
    /// Set the current scope ID for subsequently emitted events.
    fn set_current_scope_id(&self, _scope_id: Option<String>) {}

    /// Get the current scope ID.
    fn current_scope_id(&self) -> Option<String> {
        None
    }

    // ── Existing ────────────────────────────────────────────────────
    fn emit_llm_token(&self, content: &str);
    fn emit_tool_start(&self, name: &str, args: &HashMap<String, Value>);
    fn emit_tool_end(&self, name: &str, result: &Value);

    // ── Graph lifecycle ─────────────────────────────────────────────
    fn emit_graph_start(&self, _execution_id: &str, _node_count: usize) {}
    fn emit_graph_end(&self, _execution_id: &str, _node_count: usize, _success: bool) {}

    // ── Operation lifecycle ─────────────────────────────────────────
    fn emit_operation_start(&self, _node_id: u64, _op_type: AISOperationType) {}
    fn emit_operation_end(
        &self,
        _node_id: u64,
        _op_type: AISOperationType,
        _duration: Duration,
        _success: bool,
        _tokens: Option<TokenUsageSummary>,
        _timing: Option<TimingBreakdown>,
    ) {
    }
    fn emit_node_output(&self, _node_id: u64, _value: &Value) {}
    fn emit_node_output_with_name(&self, node_id: u64, _node_name: Option<&str>, value: &Value) {
        self.emit_node_output(node_id, value);
    }
    fn emit_node_metrics(&self, _node_id: u64, _metrics: &NodeMetrics) {}
    fn emit_node_metrics_with_name(
        &self,
        node_id: u64,
        _node_name: Option<&str>,
        metrics: &NodeMetrics,
    ) {
        self.emit_node_metrics(node_id, metrics);
    }
    fn emit_llm_prompt(&self, _node_id: u64, _prompt: &str) {}
    fn emit_llm_prompt_with_name(&self, node_id: u64, _node_name: Option<&str>, prompt: &str) {
        self.emit_llm_prompt(node_id, prompt);
    }
    fn emit_llm_token_for_node(&self, _node_id: u64, content: &str) {
        self.emit_llm_token(content);
    }

    // ── Planning ────────────────────────────────────────────────────
    fn emit_plan_created(&self, _plan_id: &str, _steps: usize) {}
    fn emit_plan_step_started(&self, _plan_id: &str, _step_index: usize) {}
    fn emit_plan_step_completed(&self, _plan_id: &str, _step_index: usize, _success: bool) {}
    fn emit_plan_graph_emitted(
        &self,
        _plan_id: &str,
        _generating_model: &str,
        _node_count: usize,
        _task_ids: &[u64],
        _parallel_fanout_max: usize,
    ) {
    }

    // ── Memory ──────────────────────────────────────────────────────
    fn emit_memory_read(&self, _scope: &str, _key: &str) {}
    fn emit_memory_write(&self, _scope: &str, _key: &str) {}

    // ── Checkpoints ─────────────────────────────────────────────────
    fn emit_checkpoint_saved(&self, _checkpoint_id: &str) {}
    fn emit_checkpoint_restored(&self, _checkpoint_id: &str) {}

    // ── Scheduler ───────────────────────────────────────────────────
    fn emit_scheduler_decision(&self, _node_id: u64, _delay: Duration, _reason: &str) {}
    fn emit_head_of_line_block(
        &self,
        _blocker_node: u64,
        _blocked_node: u64,
        _wait_ms: u64,
        _reason: &str,
    ) {
    }

    // ── Hardware ────────────────────────────────────────────────────
    fn emit_gpu_utilization(&self, _gpu_id: u32, _utilization_pct: f32, _memory_pct: f32) {}

    // ── Token accounting ──────────────────────────────────────────
    fn emit_token_usage(&self, _node_id: u64, _input_tokens: usize, _output_tokens: usize) {}

    // ── Memoization ───────────────────────────────────────────────
    fn emit_memoization_hit(&self, _node_id: u64) {}
}
