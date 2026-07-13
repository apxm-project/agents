//! Execution event emission hooks.

use std::collections::HashMap;
use std::time::Duration;

use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use serde::{Deserialize, Serialize};

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
    PlanWorkflowEmitted {
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

// `ExecutionEventEmitter` (and its `TokenUsageSummary` payload type) moved to
// `apxm-capability-iface` — capability's interceptor pipeline calls into this
// trait, so both capability and executor now depend on the trait's home crate
// instead of on each other. Re-exported here so `crate::executor::events::*`
// and `crate::ExecutionEventEmitter` keep working unchanged.
pub use apxm_capability_iface::events::{EventScopeState, ExecutionEventEmitter};
