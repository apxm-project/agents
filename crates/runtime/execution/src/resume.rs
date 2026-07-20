//! Durable park/resume for the canonical driver.
//!
//! The single-shot [`crate::driver::execute`] walks an AIR once and commits. A
//! conversational session instead parks at an `await.event` for each user turn
//! and resumes when input is delivered. This module adds that capability without
//! changing the single-shot path: [`crate::driver::execute_resumable`] suspends
//! at a parked `await.event`, persists a [`Continuation`] through the injected
//! [`ContinuationPort`], and [`crate::driver::resume`] re-drives from the parked
//! node when a value is delivered. Suspend/resume is the only added semantic;
//! effect dispatch, hooks, and the one atomic commit are unchanged.
//!
//! The port is the plane-correct replacement for the legacy engine's global
//! `park_registry`: it is injected, holds no static registry, and its durable
//! implementation is owned by the lifecycle/persistence plane (Server).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use apxm_inference::{ModelBindingAdmission, Usage};
use apxm_kernel::AtomicWriteSet;
use apxm_program::air::AirModule;
use apxm_program::external_agent::ExternalAgentEvidence;
use apxm_program::runtime_evidence::Fact;

/// A suspended execution, captured at a parked `await.event`. It carries exactly
/// the state required to resume: the AIR, the index of the next operation, the
/// threaded Context, the accumulated native usage and External Agent evidence,
/// the in-progress evidence batch and sequence, and the commit scope/write-set
/// used by the one atomic commit performed when the resumed run completes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Continuation {
    pub air: AirModule,
    /// The exact admitted model binding for this invocation, so a `model.call`
    /// after resume validates the same binding with no re-resolution.
    pub model_admission: ModelBindingAdmission,
    pub next_op_index: usize,
    pub context: Value,
    pub native_usage: Usage,
    pub external_agent_evidence: Vec<ExternalAgentEvidence>,
    pub evidence_batch: Vec<Fact>,
    pub event_sequence: u64,
    pub version_scope: String,
    pub commit_id: String,
    pub write_set: AtomicWriteSet,
    /// The exact wait key this execution is parked on (the `await.event`
    /// selector). Input delivered to this key resumes exactly this execution.
    pub wait_key: String,
}

/// The result of driving a resumable execution: either it ran to completion and
/// committed, or it parked at an `await.event` and persisted its continuation.
#[derive(Debug)]
pub enum RunOutcome {
    Completed(crate::driver::RunReport),
    Suspended { wait_key: String },
}

/// Whether a delivered input woke a parked execution or was queued for one that
/// has not parked yet (wake-before-register safe, matching the legacy park
/// registry's contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeOutcome {
    Woken,
    Queued,
}

/// Why a continuation operation failed. Every variant fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContinuationError {
    /// No continuation is persisted for the given wait key.
    NotParked { wait_key: String },
    /// This port does not support durable resume (e.g. [`NoResume`]).
    ResumeUnsupported,
    /// The durable store failed.
    Durable { message: String },
}

impl std::fmt::Display for ContinuationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotParked { wait_key } => {
                write!(f, "no parked continuation for wait key {wait_key}")
            }
            Self::ResumeUnsupported => write!(f, "continuation port does not support resume"),
            Self::Durable { message } => write!(f, "continuation store failed: {message}"),
        }
    }
}

impl std::error::Error for ContinuationError {}

/// The durable park/resume seam. A resumable execution persists its continuation
/// here on park; input delivery wakes it; resume takes it back. The
/// implementation owns durability and wake-before-register queueing; the runtime
/// plane never assumes a global registry.
#[async_trait]
pub trait ContinuationPort: Send + Sync {
    /// Persist a suspended continuation keyed by its `wait_key`.
    ///
    /// # Errors
    ///
    /// Returns [`ContinuationError`] when the store rejects the write.
    async fn persist(&self, continuation: Continuation) -> Result<(), ContinuationError>;

    /// Remove and return the continuation parked on `wait_key`, if any.
    ///
    /// # Errors
    ///
    /// Returns [`ContinuationError`] when the store read fails.
    async fn take(&self, wait_key: &str) -> Result<Option<Continuation>, ContinuationError>;

    /// Deliver `value` to `wait_key`. Returns [`WakeOutcome::Woken`] when a
    /// continuation is currently parked, else [`WakeOutcome::Queued`]. The value
    /// is retained (queued) so a resume can consume it via [`Self::take_pending`].
    ///
    /// # Errors
    ///
    /// Returns [`ContinuationError`] when the store rejects the write.
    async fn wake(&self, wait_key: &str, value: Value) -> Result<WakeOutcome, ContinuationError>;

    /// Take one queued delivered value for `wait_key`, if any (FIFO).
    ///
    /// # Errors
    ///
    /// Returns [`ContinuationError`] when the store read fails.
    async fn take_pending(&self, wait_key: &str) -> Result<Option<Value>, ContinuationError>;
}

/// A continuation port that supports no durable resume. It exists so a caller
/// that guarantees no parks can still use the resumable API surface; any attempt
/// to persist a continuation fails closed. The single-shot [`crate::driver::execute`]
/// never parks and never touches a port at all.
pub struct NoResume;

#[async_trait]
impl ContinuationPort for NoResume {
    async fn persist(&self, _continuation: Continuation) -> Result<(), ContinuationError> {
        Err(ContinuationError::ResumeUnsupported)
    }

    async fn take(&self, _wait_key: &str) -> Result<Option<Continuation>, ContinuationError> {
        Ok(None)
    }

    async fn wake(&self, _wait_key: &str, _value: Value) -> Result<WakeOutcome, ContinuationError> {
        Ok(WakeOutcome::Queued)
    }

    async fn take_pending(&self, _wait_key: &str) -> Result<Option<Value>, ContinuationError> {
        Ok(None)
    }
}
