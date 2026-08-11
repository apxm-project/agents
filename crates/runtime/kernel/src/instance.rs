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
    RuntimeEvidenceVersion, RuntimeFact,
};

use crate::bundle::PortBundle;
use crate::commit::{
    AtomicWriteSet, ExecutionCommitRequest, ExecutionCommitResult, ExecutionCommitTuple,
    ProgramInstanceRef, ProgramInvocationRef,
};

/// One invocation request against an instance.
pub struct Invocation {
    pub commit_id: String,
    pub program_invocation_ref: ProgramInvocationRef,
    pub write_set: AtomicWriteSet,
}

/// The result of driving one invocation to the commit boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvocationReport {
    Committed { new_program_state_version: u64 },
    CompareConflict { current_program_state_version: u64 },
    OutcomeUnknown { reconciliation_ref: String },
    Cancelled,
    Failed { message: String },
}

/// Why an invocation could not even start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstanceError {
    /// Runtime evidence omitted the Program Instance identity required to scope
    /// the atomic commit boundary.
    MissingProgramInstanceIdentity,
    /// The supplied Program Instance key and runtime-evidence identity differ.
    ProgramInstanceIdentityMismatch {
        expected: ProgramInstanceRef,
        actual: String,
    },
    /// The instance is single-flight and already has an invocation in flight.
    Busy,
    /// The request did not satisfy the complete atomic commit boundary.
    InvalidCommitRequest { message: String },
    /// The invocation needs a port the bundle does not carry.
    MissingPort(crate::bundle::PortSlot),
}

impl std::fmt::Display for InstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingProgramInstanceIdentity => {
                write!(f, "Program Instance identity is required")
            }
            Self::ProgramInstanceIdentityMismatch { expected, actual } => write!(
                f,
                "Program Instance identity mismatch: expected {}, got {actual}",
                expected.as_str()
            ),
            Self::Busy => write!(f, "instance is single-flight and busy"),
            Self::InvalidCommitRequest { message } => {
                write!(f, "invalid atomic commit request: {message}")
            }
            Self::MissingPort(slot) => write!(f, "bundle has no {} port", slot.as_str()),
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
    instance_ref: ProgramInstanceRef,
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
    /// bundle. The Program Instance reference keys compare-and-commit and
    /// continuation reads for this instance.
    pub fn new(
        identity: ProgramIdentity,
        program_instance_ref: ProgramInstanceRef,
        bundle: PortBundle,
    ) -> Result<Self, InstanceError> {
        let actual = identity
            .program_instance_id
            .as_deref()
            .ok_or(InstanceError::MissingProgramInstanceIdentity)?;
        if actual != program_instance_ref.as_str() {
            return Err(InstanceError::ProgramInstanceIdentityMismatch {
                expected: program_instance_ref,
                actual: actual.to_string(),
            });
        }
        Ok(Self {
            identity,
            instance_ref: program_instance_ref,
            bundle,
            busy: AtomicBool::new(false),
            inner: Mutex::new(Inner {
                instance_state: InstanceState::Ready,
                durable_seq: 0,
                created_recorded: false,
                durable_facts: Vec::new(),
            }),
        })
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
            facts: self
                .inner
                .lock()
                .expect("instance inner")
                .durable_facts
                .clone(),
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
            .current_version(&self.instance_ref)
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
            batch.push(lifecycle_fact(
                seq,
                FactKind::InstanceCreated,
                Some(InstanceState::Ready),
                None,
                None,
            ));
        }
        seq += 1;
        batch.push(lifecycle_fact(
            seq,
            FactKind::InvocationAdmitted,
            None,
            Some(InvocationState::Running),
            None,
        ));
        seq += 1;
        batch.push(lifecycle_fact(
            seq,
            FactKind::InvocationAttemptRecorded,
            None,
            None,
            None,
        ));
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
            program_instance_ref: self.instance_ref.clone(),
            program_invocation_ref: invocation.program_invocation_ref,
            idempotency_key: format!("idem.{}", invocation.commit_id),
            expected_program_state_version: expected,
            write_set: invocation.write_set,
            tuple: ExecutionCommitTuple::empty(batch.clone()),
            evidence_batch: batch.clone(),
        };
        request
            .validate()
            .map_err(|error| InstanceError::InvalidCommitRequest {
                message: error.to_string(),
            })?;

        let result = self.bundle.execution_commit().commit(request).await;

        match result {
            ExecutionCommitResult::Committed {
                new_program_state_version,
                ..
            } => {
                let mut inner = self.inner.lock().expect("instance inner");
                inner.created_recorded = true;
                inner.durable_seq = last_seq;
                inner.instance_state = InstanceState::Ready;
                inner.durable_facts.extend(batch);
                Ok(InvocationReport::Committed {
                    new_program_state_version,
                })
            }
            ExecutionCommitResult::CompareConflict {
                current_program_state_version,
            } => {
                // Nothing durable: the prepared batch is discarded.
                Ok(InvocationReport::CompareConflict {
                    current_program_state_version,
                })
            }
            ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => {
                // Nothing durable and never success: the caller reconciles.
                Ok(InvocationReport::OutcomeUnknown { reconciliation_ref })
            }
        }
    }

    /// Commit cancellation for this instance, including while another
    /// invocation is in flight. Cancellation competes with that invocation at
    /// the atomic compare-and-commit boundary: whichever exact commit wins is
    /// authoritative, while the loser observes a conflict and publishes
    /// nothing. This keeps cancellation able to interrupt a blocked commit
    /// without creating a second writer or a partial lifecycle transition.
    pub async fn cancel(
        &self,
        commit_id: impl Into<String>,
        program_invocation_ref: ProgramInvocationRef,
        write_set: AtomicWriteSet,
    ) -> Result<InvocationReport, InstanceError> {
        let commit_id = commit_id.into();
        let expected = self
            .bundle
            .execution_commit()
            .current_version(&self.instance_ref)
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
            program_instance_ref: self.instance_ref.clone(),
            program_invocation_ref,
            idempotency_key: format!("idem.{commit_id}"),
            expected_program_state_version: expected,
            write_set,
            tuple: ExecutionCommitTuple::empty(batch.clone()),
            evidence_batch: batch.clone(),
        };
        request
            .validate()
            .map_err(|error| InstanceError::InvalidCommitRequest {
                message: error.to_string(),
            })?;

        match self.bundle.execution_commit().commit(request).await {
            ExecutionCommitResult::Committed { .. } => {
                let mut inner = self.inner.lock().expect("instance inner");
                inner.durable_seq = cancel_fact_seq;
                inner.durable_facts.extend(batch);
                Ok(InvocationReport::Cancelled)
            }
            ExecutionCommitResult::CompareConflict {
                current_program_state_version,
            } => Ok(InvocationReport::CompareConflict {
                current_program_state_version,
            }),
            ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => {
                Ok(InvocationReport::OutcomeUnknown { reconciliation_ref })
            }
        }
    }
}

/// One External Agent capability invocation: the outer `capability.invoke`
/// NodeExecution id, the ACP prompt request, and the prepared write set.
pub struct CapabilityInvocation {
    pub commit_id: String,
    pub program_invocation_ref: ProgramInvocationRef,
    pub capability_node_execution_id: String,
    pub request: crate::external_agent::AcpPromptRequest,
    pub write_set: AtomicWriteSet,
}

/// The result of one External Agent capability invocation: the nested attributed
/// evidence (including peer usage provenance) and the commit outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityReport {
    pub evidence: apxm_program::external_agent::ExternalAgentEvidence,
    pub commit: InvocationReport,
}

impl ProgramInstance {
    /// Execute one External Agent capability through the bundle's external-agent
    /// port and commit it atomically. The peer loop becomes ordered attributed
    /// nested evidence under one Capability NodeExecution; peer usage stays in
    /// that evidence as provenance and never enters the native usage facts.
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::Busy`] if an invocation is in flight, or
    /// [`InstanceError::MissingPort`] if the bundle carries no external-agent port.
    pub async fn invoke_capability(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityReport, InstanceError> {
        use crate::external_agent::{PromptEffectState, assemble_evidence};

        let _guard = self.acquire()?;

        let port = self
            .bundle
            .external_agent_capability()
            .ok_or(InstanceError::MissingPort(
                crate::bundle::PortSlot::ExternalAgentCapability,
            ))?
            .clone();

        // Validate immutable commit members before the external effect can run.
        // The final request is validated again after its evidence batch is
        // assembled, so an adapter never sees a malformed atomic boundary.
        let preflight = ExecutionCommitRequest {
            commit_id: invocation.commit_id.clone(),
            program_instance_ref: self.instance_ref.clone(),
            program_invocation_ref: invocation.program_invocation_ref.clone(),
            idempotency_key: format!("idem.{}", invocation.commit_id),
            expected_program_state_version: 0,
            write_set: invocation.write_set.clone(),
            tuple: ExecutionCommitTuple::empty(Vec::new()),
            evidence_batch: Vec::new(),
        };
        preflight
            .validate()
            .map_err(|error| InstanceError::InvalidCommitRequest {
                message: error.to_string(),
            })?;

        let node_exec = invocation.capability_node_execution_id.clone();
        let outcome = port.prompt(invocation.request).await;
        let evidence = assemble_evidence(node_exec.clone(), &outcome);

        let expected = self
            .bundle
            .execution_commit()
            .current_version(&self.instance_ref)
            .await;
        let target_version = expected + 1;

        let (mut seq, created_recorded) = {
            let inner = self.inner.lock().expect("instance inner");
            (inner.durable_seq, inner.created_recorded)
        };
        let mut batch: Vec<Fact> = Vec::new();
        if !created_recorded {
            seq += 1;
            batch.push(lifecycle_fact(
                seq,
                FactKind::InstanceCreated,
                Some(InstanceState::Ready),
                None,
                None,
            ));
        }
        seq += 1;
        batch.push(lifecycle_fact(
            seq,
            FactKind::InvocationAdmitted,
            None,
            Some(InvocationState::Running),
            None,
        ));
        seq += 1;
        batch.push(capability_attempt_fact(seq, &node_exec));
        seq += 1;
        let last_seq = seq;
        batch.push(capability_terminal_fact(
            last_seq,
            &node_exec,
            &outcome.state,
            target_version,
        ));

        let request = ExecutionCommitRequest {
            commit_id: invocation.commit_id.clone(),
            program_instance_ref: self.instance_ref.clone(),
            program_invocation_ref: invocation.program_invocation_ref,
            idempotency_key: format!("idem.{}", invocation.commit_id),
            expected_program_state_version: expected,
            write_set: invocation.write_set,
            tuple: ExecutionCommitTuple::empty(batch.clone()),
            evidence_batch: batch.clone(),
        };

        request
            .validate()
            .map_err(|error| InstanceError::InvalidCommitRequest {
                message: error.to_string(),
            })?;

        let commit = match self.bundle.execution_commit().commit(request).await {
            ExecutionCommitResult::Committed {
                new_program_state_version,
                ..
            } => {
                let mut inner = self.inner.lock().expect("instance inner");
                inner.created_recorded = true;
                inner.durable_seq = last_seq;
                inner.durable_facts.extend(batch);
                match &outcome.state {
                    PromptEffectState::Completed { .. } => InvocationReport::Committed {
                        new_program_state_version,
                    },
                    PromptEffectState::Cancelled => InvocationReport::Cancelled,
                    PromptEffectState::Failed { message } => InvocationReport::Failed {
                        message: message.clone(),
                    },
                    PromptEffectState::OutcomeUnknown { message } => {
                        InvocationReport::OutcomeUnknown {
                            reconciliation_ref: message.clone(),
                        }
                    }
                }
            }
            ExecutionCommitResult::CompareConflict {
                current_program_state_version,
            } => InvocationReport::CompareConflict {
                current_program_state_version,
            },
            ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => {
                InvocationReport::OutcomeUnknown { reconciliation_ref }
            }
        };

        Ok(CapabilityReport { evidence, commit })
    }
}

fn capability_attempt_fact(seq: u64, node_execution_id: &str) -> Fact {
    let mut fact = lifecycle_fact(seq, FactKind::CapabilityAttemptRecorded, None, None, None);
    runtime_fact_mut(&mut fact).node_execution_id = Some(node_execution_id.to_string());
    fact
}

fn capability_terminal_fact(
    seq: u64,
    node_execution_id: &str,
    state: &crate::external_agent::PromptEffectState,
    target_version: u64,
) -> Fact {
    use crate::external_agent::PromptEffectState;
    let mut fact = match state {
        PromptEffectState::Completed { .. } => lifecycle_fact(
            seq,
            FactKind::InvocationCommitted,
            None,
            Some(InvocationState::CommittedReturn),
            Some(target_version),
        ),
        PromptEffectState::Cancelled => lifecycle_fact(
            seq,
            FactKind::InvocationCancelled,
            None,
            Some(InvocationState::Cancelled),
            None,
        ),
        PromptEffectState::Failed { message } => {
            let mut fact = lifecycle_fact(
                seq,
                FactKind::InvocationFailed,
                None,
                Some(InvocationState::Failed),
                None,
            );
            runtime_fact_mut(&mut fact).typed_error =
                Some(apxm_program::common::TypedErrorEnvelope {
                    error_id: "agents.external_agent_failed".to_string(),
                    category: apxm_program::common::ErrorCategory::Unavailable,
                    code_ref: "ExternalAgentFailed".to_string(),
                    message: message.clone(),
                    details_digest: None,
                });
            fact
        }
        PromptEffectState::OutcomeUnknown { .. } => {
            let mut fact = lifecycle_fact(seq, FactKind::EffectOutcomeUnknown, None, None, None);
            runtime_fact_mut(&mut fact).effect_outcome_ref = Some(apxm_program::common::TypedRef {
                ref_type: "ExternalAgentEffectRef".to_string(),
                target: node_execution_id.to_string(),
                digest: None,
            });
            fact
        }
    };
    runtime_fact_mut(&mut fact).node_execution_id = Some(node_execution_id.to_string());
    fact
}

fn lifecycle_fact(
    seq: u64,
    kind: FactKind,
    instance_state: Option<InstanceState>,
    invocation_state: Option<InvocationState>,
    commit_sequence: Option<u64>,
) -> Fact {
    Fact::from_runtime(
        kind,
        RuntimeFact {
            fact_id: format!("fact.{seq}"),
            event_sequence: seq,
            ownership_epoch: None,
            instance_state,
            invocation_state,
            event_state: None,
            model_outcome: None,
            commit_sequence,
            node_execution_id: None,
            attempt_id: None,
            region_occurrence_id: None,
            static_region_id: None,
            loop_memberships: None,
            air_node_id: None,
            parent_node_execution_id: None,
            hook_execution_id: None,
            hook_id: None,
            hook_scope: None,
            hook_phase: None,
            context_transition_id: None,
            context_before_ref: None,
            context_after_ref: None,
            effect_outcome_ref: None,
            typed_error: None,
        },
    )
}

fn runtime_fact_mut(fact: &mut Fact) -> &mut RuntimeFact {
    fact.runtime_mut()
        .expect("lifecycle helpers construct only runtime facts")
}
