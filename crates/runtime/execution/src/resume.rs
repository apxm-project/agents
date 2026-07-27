//! Durable park/resume state for the canonical driver.
//!
//! A parked continuation is serialized into the same Execution Commit tuple as
//! Context, event wait registration, effects, usage, output references, and
//! evidence. The runtime never owns a second continuation persistence port.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ports::EventRef;
use apxm_inference::{ModelBindingAdmission, Usage};
use apxm_kernel::AtomicWriteSet;
use apxm_program::air::AirModule;
use apxm_program::external_agent::ExternalAgentEvidence;
use apxm_program::frontend_graph::HookBinding;
use apxm_program::runtime_evidence::Fact;

/// One active structural loop frame persisted in exact nesting order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableLoopFrame {
    pub static_loop_id: String,
    pub dynamic_occurrence_id: String,
    pub iteration_index: u64,
    pub causal_node_execution_ids: Vec<String>,
    pub failed: bool,
    pub parked: bool,
}

/// A suspended execution, captured at a structural yield or parked
/// `await.event`. It carries exactly the state required to resume: the AIR, the
/// index of the next operation, the threaded Context, the accumulated native
/// usage and External Agent evidence, the in-progress evidence batch and
/// sequence, and the commit scope/write-set used by the next atomic commit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Continuation {
    pub air: AirModule,
    pub hook_bindings: Vec<HookBinding>,
    /// The exact admitted model binding for this invocation, so a `model.call`
    /// after resume validates the same binding with no re-resolution.
    pub model_admission: ModelBindingAdmission,
    pub next_schedule_position: usize,
    pub loop_frames: Vec<DurableLoopFrame>,
    pub parked_node_execution_id: Option<String>,
    pub parked_loop_path: Vec<String>,
    pub context: Value,
    pub native_usage: Usage,
    pub external_agent_evidence: Vec<ExternalAgentEvidence>,
    pub evidence_batch: Vec<Fact>,
    pub event_sequence: u64,
    pub invocation_ref: String,
    pub version_scope: String,
    pub commit_id: String,
    pub write_set: AtomicWriteSet,
    /// The compiler-provided identity of the structural continuation.
    pub continuation_id: String,
    /// The exact durable event identity registered by the suspended wait, if
    /// this continuation parked at `await.event`.
    pub event_ref: Option<EventRef>,
}

/// The result of driving a resumable execution: either it ran to completion and
/// committed, or it parked at a structural yield or `await.event` and persisted
/// its continuation.
#[derive(Debug)]
pub enum RunOutcome {
    Completed(crate::driver::RunReport),
    Suspended {
        continuation_id: String,
        event_ref: Option<EventRef>,
        operational_usage: crate::operational_usage::CommittedNativeModelUsageOutcome,
    },
}

/// Why a continuation operation failed. Every variant fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContinuationError {
    /// No committed continuation is available for the requested Program Instance.
    NotCommitted { invocation_ref: String },
    /// The authoritative commit record contained an invalid continuation payload.
    InvalidCommittedState { message: String },
}

impl std::fmt::Display for ContinuationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCommitted { invocation_ref } => {
                write!(
                    f,
                    "no committed continuation for invocation {invocation_ref}"
                )
            }
            Self::InvalidCommittedState { message } => {
                write!(f, "invalid committed continuation state: {message}")
            }
        }
    }
}

impl std::error::Error for ContinuationError {}
