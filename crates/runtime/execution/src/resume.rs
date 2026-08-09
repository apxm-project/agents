//! Durable park/resume state for the canonical driver.
//!
//! A parked continuation is serialized into the same Execution Commit tuple as
//! Context, event wait registration, effects, usage, output references, and
//! evidence. The runtime never owns a second continuation persistence port.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::driver::CapabilityInvocationAdmission;
use crate::ports::EventRef;
use apxm_inference::{ModelBindingAdmission, Usage};
use apxm_kernel::{AtomicWriteSet, ProgramInstanceRef, ProgramInvocationRef};
use apxm_program::air::AirModule;
use apxm_program::external_agent::ExternalAgentEvidence;
use apxm_program::frontend_graph::HookBinding;
use apxm_program::runtime_evidence::Fact;
use std::collections::BTreeMap;

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
/// sequence, and the exact instance/invocation identities used by the next
/// atomic commit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Continuation {
    pub air: AirModule,
    pub hook_bindings: Vec<HookBinding>,
    /// The exact admitted model binding for this invocation, so a `model.call`
    /// after resume validates the same binding with no re-resolution.
    pub model_admission: ModelBindingAdmission,
    /// Exact per-node Capability inputs and authority survive park/resume
    /// without re-resolution or ambient reconstruction.
    pub capability_invocations: BTreeMap<String, CapabilityInvocationAdmission>,
    pub next_schedule_position: usize,
    pub loop_frames: Vec<DurableLoopFrame>,
    pub parked_node_execution_id: Option<String>,
    pub parked_loop_path: Vec<String>,
    /// Selected conditional arms that remain active across an inner park.
    pub branch_decisions: BTreeMap<String, usize>,
    /// Exact SSA destination bound by a structural yield delivery.
    pub resume_value_id: Option<String>,
    pub context: Value,
    /// Runtime SSA application values needed by predicates after resume.
    pub values: BTreeMap<String, Value>,
    pub last_result: Value,
    pub last_result_value_id: Option<String>,
    pub native_usage: Usage,
    pub external_agent_evidence: Vec<ExternalAgentEvidence>,
    pub evidence_batch: Vec<Fact>,
    pub event_sequence: u64,
    pub program_instance_ref: ProgramInstanceRef,
    pub program_invocation_ref: ProgramInvocationRef,
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
    NotCommitted {
        program_instance_ref: ProgramInstanceRef,
    },
    /// The commit port returned a continuation for a different Program Instance
    /// than the one used as its continuation key.
    InstanceScopeMismatch {
        requested: ProgramInstanceRef,
        committed: ProgramInstanceRef,
    },
    /// The authoritative commit record contained an invalid continuation payload.
    InvalidCommittedState { message: String },
}

impl std::fmt::Display for ContinuationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCommitted {
                program_instance_ref,
            } => {
                write!(
                    f,
                    "no committed continuation for Program Instance {}",
                    program_instance_ref.as_str()
                )
            }
            Self::InstanceScopeMismatch {
                requested,
                committed,
            } => write!(
                f,
                "continuation key mismatch: requested Program Instance {}, committed Program Instance {}",
                requested.as_str(),
                committed.as_str()
            ),
            Self::InvalidCommittedState { message } => {
                write!(f, "invalid committed continuation state: {message}")
            }
        }
    }
}

impl std::error::Error for ContinuationError {}
