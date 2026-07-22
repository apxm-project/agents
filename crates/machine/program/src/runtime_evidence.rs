//! `apxm.runtime-evidence.v1` — closed consumer types and verification.
//!
//! Runtime evidence is an append-only sequence of monotonic authoritative facts.
//! Lifecycle truth is reconstructed from the latest scoped state fact; a trace,
//! stream, or delivery record cannot override it, and an uncertain external
//! effect never silently becomes success. Decode fails closed on any unknown
//! field or unknown enum member.

use serde::{Deserialize, Serialize};

use crate::common::{TypedErrorEnvelope, TypedRef};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::grammar::{is_digest, is_identifier};

/// The single accepted `schema_version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeEvidenceVersion {
    #[serde(rename = "apxm.runtime-evidence.v1")]
    V1,
}

/// The closed fact-kind set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    #[serde(rename = "instance.state_changed")]
    InstanceStateChanged,
    #[serde(rename = "invocation.state_changed")]
    InvocationStateChanged,
    #[serde(rename = "instance.created")]
    InstanceCreated,
    #[serde(rename = "invocation.admitted")]
    InvocationAdmitted,
    #[serde(rename = "child.attached")]
    ChildAttached,
    #[serde(rename = "attempt.recorded")]
    AttemptRecorded,
    #[serde(rename = "invocation.committed")]
    InvocationCommitted,
    #[serde(rename = "invocation.failed")]
    InvocationFailed,
    #[serde(rename = "invocation.cancelled")]
    InvocationCancelled,
    #[serde(rename = "event.created")]
    EventCreated,
    #[serde(rename = "event.await_registered")]
    EventAwaitRegistered,
    #[serde(rename = "invocation.parked")]
    InvocationParked,
    #[serde(rename = "event.terminal")]
    EventTerminal,
    #[serde(rename = "invocation.resumed")]
    InvocationResumed,
    #[serde(rename = "instance.closed")]
    InstanceClosed,
    #[serde(rename = "instance.cancelled")]
    InstanceCancelled,
    #[serde(rename = "effect.outcome_unknown")]
    EffectOutcomeUnknown,
    #[serde(rename = "delivery.recorded")]
    DeliveryRecorded,
    #[serde(rename = "region.occurrence_started")]
    RegionOccurrenceStarted,
    #[serde(rename = "node_execution.recorded")]
    NodeExecutionRecorded,
    #[serde(rename = "hook.executed")]
    HookExecuted,
    #[serde(rename = "context.transitioned")]
    ContextTransitioned,
}

/// The closed Program Instance state set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    AdmissionPending,
    Ready,
    Invoking,
    Completed,
    Closing,
    Closed,
    Cancelling,
    Cancelled,
}

/// The closed Invocation state set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationState {
    AdmissionPending,
    Running,
    WaitingEvent,
    CommittedYield,
    CommittedReturn,
    Failed,
    Cancelling,
    Cancelled,
    CancellationUnconfirmed,
}

impl InvocationState {
    /// Whether this state asserts a committed invocation outcome.
    #[must_use]
    pub fn is_committed(self) -> bool {
        matches!(self, Self::CommittedYield | Self::CommittedReturn)
    }
}

/// The closed event state set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventState {
    Pending,
    Fulfilled,
    Expired,
    Cancelled,
}

/// The closed model-outcome set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelOutcome {
    CommittedSuccess,
    TypedFailure,
    Cancelled,
    ModelOutcomeUnknown,
}

/// The program identity an evidence sequence is scoped to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramIdentity {
    pub artifact_digest: String,
    pub entrypoint: String,
    pub agent_identity_binding: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_instance_id: Option<String>,
}

/// One authoritative fact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub fact_id: String,
    pub event_sequence: u64,
    pub fact_kind: FactKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership_epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_state: Option<InstanceState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_state: Option<InvocationState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_state: Option<EventState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_outcome: Option<ModelOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_occurrence_id: Option<String>,
    /// Compiler-owned static region joined to one dynamic occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_region_id: Option<String>,
    /// The exact static AIR node visited by this dynamic NodeExecution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_node_id: Option<String>,
    /// The authored parent NodeExecution for explicitly nested work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_node_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_scope: Option<HookScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_phase: Option<HookPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_transition_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_before_ref: Option<TypedRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_after_ref: Option<TypedRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect_outcome_ref: Option<TypedRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed_error: Option<TypedErrorEnvelope>,
}

/// The static scope of a Hook execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookScope {
    Agent,
    Loop,
    Node,
    Model,
    Capability,
}

/// The closed before/after Hook phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    Before,
    After,
}

/// A decoded runtime-evidence sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEvidence {
    pub schema_version: RuntimeEvidenceVersion,
    pub program_identity: ProgramIdentity,
    pub facts: Vec<Fact>,
}

impl RuntimeEvidence {
    /// Verify a decoded evidence sequence: at least one fact, strictly monotonic
    /// event sequence, grammar, and the honesty rule that an uncertain effect
    /// never claims success.
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();

        if !is_digest(&self.program_identity.artifact_digest) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::InvalidDigest,
                "program_identity.artifact_digest",
                "artifact_digest is not a lowercase sha256 value",
            ));
        }
        if !is_identifier(&self.program_identity.agent_identity_binding) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::InvalidIdentifier,
                "program_identity.agent_identity_binding",
                "agent_identity_binding is not a contract identifier",
            ));
        }

        if self.facts.is_empty() {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                "facts",
                "an evidence sequence has at least one fact",
            ));
        }

        let mut previous: Option<u64> = None;
        for fact in &self.facts {
            if let Some(prev) = previous
                && fact.event_sequence <= prev
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::NonMonotonicEvidence,
                    fact.fact_id.clone(),
                    "event_sequence is not strictly increasing",
                ));
            }
            previous = Some(fact.event_sequence);

            if fact.fact_kind == FactKind::EffectOutcomeUnknown {
                let claims_success = fact.model_outcome == Some(ModelOutcome::CommittedSuccess)
                    || fact
                        .invocation_state
                        .is_some_and(InvocationState::is_committed);
                if claims_success {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::OutcomeUnknownClaimsSuccess,
                        fact.fact_id.clone(),
                        "an uncertain effect must not claim a committed/success outcome",
                    ));
                }
            }
            let missing = match fact.fact_kind {
                FactKind::RegionOccurrenceStarted => {
                    fact.region_occurrence_id.is_none() || fact.static_region_id.is_none()
                }
                FactKind::NodeExecutionRecorded => {
                    fact.node_execution_id.is_none()
                        || fact.air_node_id.is_none()
                        || fact.region_occurrence_id.is_none()
                        || fact.static_region_id.is_none()
                }
                FactKind::HookExecuted => {
                    fact.hook_execution_id.is_none()
                        || fact.hook_id.is_none()
                        || fact.hook_scope.is_none()
                        || fact.hook_phase.is_none()
                        || fact.context_before_ref.is_none()
                        || fact.context_after_ref.is_none()
                }
                FactKind::ContextTransitioned => {
                    fact.context_transition_id.is_none()
                        || fact.context_before_ref.is_none()
                        || fact.context_after_ref.is_none()
                }
                _ => false,
            };
            if missing {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    fact.fact_id.clone(),
                    "evidence join fact is missing a required typed join",
                ));
            }
        }

        verdict.finish()
    }
}

/// Verify runtime evidence presented as JSON, failing closed on decode errors.
#[must_use]
pub fn verify_runtime_evidence_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<RuntimeEvidence>(value.clone()) {
        Ok(evidence) => evidence.verify(),
        Err(error) => schema_violation("runtime_evidence", &error),
    }
}
