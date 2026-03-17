//! Execution event emission hooks.

use std::collections::HashMap;
use std::time::Duration;

use apxm_core::types::values::Value;
use serde::{Deserialize, Serialize};

/// Runtime execution event.
///
/// Covers ~15 event categories: LLM tokens, tool lifecycle, operation
/// lifecycle, planning, memory, checkpoints, scheduler decisions, and
/// GPU hardware utilization.
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
        op_type: String,
    },
    OperationEnd {
        node_id: u64,
        op_type: String,
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

    // ── Hardware events (Phase 2 prep) ──────────────────────────────
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
    // ── Existing ────────────────────────────────────────────────────
    fn emit_llm_token(&self, content: &str);
    fn emit_tool_start(&self, name: &str, args: &HashMap<String, Value>);
    fn emit_tool_end(&self, name: &str, result: &Value);

    // ── Operation lifecycle ─────────────────────────────────────────
    fn emit_operation_start(&self, _node_id: u64, _op_type: &str) {}
    fn emit_operation_end(
        &self,
        _node_id: u64,
        _op_type: &str,
        _duration: Duration,
        _success: bool,
    ) {
    }

    // ── Planning ────────────────────────────────────────────────────
    fn emit_plan_created(&self, _plan_id: &str, _steps: usize) {}
    fn emit_plan_step_started(&self, _plan_id: &str, _step_index: usize) {}
    fn emit_plan_step_completed(&self, _plan_id: &str, _step_index: usize, _success: bool) {}

    // ── Memory ──────────────────────────────────────────────────────
    fn emit_memory_read(&self, _scope: &str, _key: &str) {}
    fn emit_memory_write(&self, _scope: &str, _key: &str) {}

    // ── Checkpoints ─────────────────────────────────────────────────
    fn emit_checkpoint_saved(&self, _checkpoint_id: &str) {}
    fn emit_checkpoint_restored(&self, _checkpoint_id: &str) {}

    // ── Scheduler ───────────────────────────────────────────────────
    fn emit_scheduler_decision(&self, _node_id: u64, _delay: Duration, _reason: &str) {}

    // ── Hardware ────────────────────────────────────────────────────
    fn emit_gpu_utilization(&self, _gpu_id: u32, _utilization_pct: f32, _memory_pct: f32) {}

    // ── Token accounting ──────────────────────────────────────────
    fn emit_token_usage(&self, _node_id: u64, _input_tokens: usize, _output_tokens: usize) {}

    // ── Memoization ───────────────────────────────────────────────
    fn emit_memoization_hit(&self, _node_id: u64) {}
}
