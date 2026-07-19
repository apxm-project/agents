//! Program Instance runtime lifecycle.
//!
//! An instance is single-flight and fail-busy: at most one invocation is in
//! flight; a second concurrent invocation is rejected (queueing lives outside
//! the instance). The only way state and evidence become durable is the atomic
//! Execution Commit port — a conflict or an ambiguous outcome publishes nothing,
//! so a partial step is never observable. Instances are isolated: each owns its
//! own state, version, and evidence mirror.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use apxm_program::runtime_evidence::{
    Fact, FactKind, InstanceState, InvocationState, ProgramIdentity, RuntimeEvidence,
    RuntimeEvidenceVersion,
};

use crate::bundle::PortBundle;
use crate::commit::{AtomicWriteSet, ExecutionCommitRequest, ExecutionCommitResult};

/// One invocation request against an instance.
pub struct Invocation {
    pub commit_id: String,
    pub write_set: AtomicWriteSet,
}

/// The result of driving one invocation to the commit boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvocationReport {
    Committed { new_program_state_version: u64 },
    CompareConflict { current_program_state_version: u64 },
    OutcomeUnknown { reconciliation_ref: String },
    Cancelled,
}

/// Why an invocation could not even start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstanceError {
    /// The instance is single-flight and already has an invocation in flight.
    Busy,
}

impl std::fmt::Display for InstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => write!(f, "instance is single-flight and busy"),
        }
    }
}

impl std::error::Error for InstanceError {}

struct Inner {
    instance_state: InstanceState,
    durable_seq: u64,
    created_recorded: bool,
    durable_facts: Vec<Fact>,
}

/// A running Program Instance.
pub struct ProgramInstance {
    identity: ProgramIdentity,
    version_scope: String,
    bundle: PortBundle,
    busy: AtomicBool,
    inner: Mutex<Inner>,
}

/// Resets the single-flight flag even if the invocation body unwinds.
struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl ProgramInstance {
    /// Create a ready instance bound to an already-constructed, validated port
    /// bundle. The version scope keys the compare-and-commit for this instance.
    #[must_use]
    pub fn new(identity: ProgramIdentity, version_scope: impl Into<String>, bundle: PortBundle) -> Self {
        Self {
            identity,
            version_scope: version_scope.into(),
            bundle,
            busy: AtomicBool::new(false),
            inner: Mutex::new(Inner {
                instance_state: InstanceState::Ready,
                durable_seq: 0,
                created_recorded: false,
                durable_facts: Vec::new(),
            }),
        }
    }

    #[must_use]
    pub fn instance_state(&self) -> InstanceState {
        self.inner.lock().expect("instance inner").instance_state
    }

    /// Whether an invocation is currently in flight (single-flight guard held).
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// The durable evidence this live instance has observed committing.
    #[must_use]
    pub fn evidence(&self) -> RuntimeEvidence {
        RuntimeEvidence {
            schema_version: RuntimeEvidenceVersion::V1,
            program_identity: self.identity.clone(),
            facts: self.inner.lock().expect("instance inner").durable_facts.clone(),
        }
    }

    fn acquire(&self) -> Result<BusyGuard<'_>, InstanceError> {
        match self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => Ok(BusyGuard(&self.busy)),
            Err(_) => Err(InstanceError::Busy),
        }
    }

    /// Drive one invocation to the commit boundary, single-flight.
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::Busy`] if an invocation is already in flight.
    pub async fn invoke(&self, invocation: Invocation) -> Result<InvocationReport, InstanceError> {
        let _guard = self.acquire()?;

        let expected = self
            .bundle
            .execution_commit()
            .current_version(&self.version_scope)
            .await;

        // Build the full evidence batch this atomic commit will publish,
        // including the terminal committed fact whose commit sequence is the
        // version this compare-and-commit targets. Sequences and the batch are
        // tentative until the atomic commit makes them durable.
        let (mut seq, created_recorded) = {
            let inner = self.inner.lock().expect("instance inner");
            (inner.durable_seq, inner.created_recorded)
        };
        let target_version = expected + 1;
        let mut batch: Vec<Fact> = Vec::new();
        if !created_recorded {
            seq += 1;
            batch.push(lifecycle_fact(seq, FactKind::InstanceCreated, Some(InstanceState::Ready), None, None));
        }
        seq += 1;
        batch.push(lifecycle_fact(seq, FactKind::InvocationAdmitted, None, Some(InvocationState::Running), None));
        seq += 1;
        batch.push(lifecycle_fact(seq, FactKind::AttemptRecorded, None, None, None));
        seq += 1;
        let last_seq = seq;
        batch.push(lifecycle_fact(
            last_seq,
            FactKind::InvocationCommitted,
            None,
            Some(InvocationState::CommittedReturn),
            Some(target_version),
        ));

        let request = ExecutionCommitRequest {
            commit_id: invocation.commit_id.clone(),
            invocation_ref: self.version_scope.clone(),
            idempotency_key: format!("idem.{}", invocation.commit_id),
            expected_program_state_version: expected,
            write_set: invocation.write_set,
            evidence_batch: batch.clone(),
        };

        let result = self.bundle.execution_commit().commit(request).await;

        match result {
            ExecutionCommitResult::Committed { new_program_state_version, .. } => {
                let mut inner = self.inner.lock().expect("instance inner");
                inner.created_recorded = true;
                inner.durable_seq = last_seq;
                inner.instance_state = InstanceState::Ready;
                inner.durable_facts.extend(batch);
                Ok(InvocationReport::Committed { new_program_state_version })
            }
            ExecutionCommitResult::CompareConflict { current_program_state_version } => {
                // Nothing durable: the prepared batch is discarded.
                Ok(InvocationReport::CompareConflict { current_program_state_version })
            }
            ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => {
                // Nothing durable and never success: the caller reconciles.
                Ok(InvocationReport::OutcomeUnknown { reconciliation_ref })
            }
        }
    }

    /// Commit a cancellation for this instance's in-flight invocation. The
    /// cancellation is durable only if the atomic commit succeeds.
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::Busy`] if an invocation is already in flight.
    pub async fn cancel(&self, commit_id: impl Into<String>, write_set: AtomicWriteSet) -> Result<InvocationReport, InstanceError> {
        let _guard = self.acquire()?;
        let commit_id = commit_id.into();
        let expected = self
            .bundle
            .execution_commit()
            .current_version(&self.version_scope)
            .await;

        let mut seq = { self.inner.lock().expect("instance inner").durable_seq };
        seq += 1;
        let cancel_fact_seq = seq;
        let batch = vec![lifecycle_fact(
            cancel_fact_seq,
            FactKind::InvocationCancelled,
            None,
            Some(InvocationState::Cancelled),
            None,
        )];

        let request = ExecutionCommitRequest {
            commit_id: commit_id.clone(),
            invocation_ref: self.version_scope.clone(),
            idempotency_key: format!("idem.{commit_id}"),
            expected_program_state_version: expected,
            write_set,
            evidence_batch: batch.clone(),
        };

        match self.bundle.execution_commit().commit(request).await {
            ExecutionCommitResult::Committed { .. } => {
                let mut inner = self.inner.lock().expect("instance inner");
                inner.durable_seq = cancel_fact_seq;
                inner.durable_facts.extend(batch);
                Ok(InvocationReport::Cancelled)
            }
            ExecutionCommitResult::CompareConflict { current_program_state_version } => {
                Ok(InvocationReport::CompareConflict { current_program_state_version })
            }
            ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => {
                Ok(InvocationReport::OutcomeUnknown { reconciliation_ref })
            }
        }
    }
}

fn lifecycle_fact(
    seq: u64,
    kind: FactKind,
    instance_state: Option<InstanceState>,
    invocation_state: Option<InvocationState>,
    commit_sequence: Option<u64>,
) -> Fact {
    Fact {
        fact_id: format!("fact.{seq}"),
        event_sequence: seq,
        fact_kind: kind,
        ownership_epoch: None,
        instance_state,
        invocation_state,
        event_state: None,
        model_outcome: None,
        commit_sequence,
        node_execution_id: None,
        attempt_id: None,
        region_occurrence_id: None,
        effect_outcome_ref: None,
        typed_error: None,
    }
}
