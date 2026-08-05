//! `apxm.runtime-evidence.v1` — closed consumer types and verification.
//!
//! Runtime evidence is an append-only sequence of monotonic authoritative facts.
//! Lifecycle truth is reconstructed from the latest scoped state fact; a trace,
//! stream, or delivery record cannot override it, and an uncertain external
//! effect never silently becomes success. Decode fails closed on any unknown
//! field or unknown enum member.

use std::collections::{HashMap, HashSet};

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
    #[serde(rename = "invocation.attempt_recorded")]
    InvocationAttemptRecorded,
    #[serde(rename = "capability.attempt_recorded")]
    CapabilityAttemptRecorded,
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

impl FactKind {
    #[must_use]
    pub const fn all() -> [Self; 24] {
        [
            Self::InstanceStateChanged,
            Self::InvocationStateChanged,
            Self::InstanceCreated,
            Self::InvocationAdmitted,
            Self::ChildAttached,
            Self::AttemptRecorded,
            Self::InvocationAttemptRecorded,
            Self::CapabilityAttemptRecorded,
            Self::InvocationCommitted,
            Self::InvocationFailed,
            Self::InvocationCancelled,
            Self::EventCreated,
            Self::EventAwaitRegistered,
            Self::InvocationParked,
            Self::EventTerminal,
            Self::InvocationResumed,
            Self::InstanceClosed,
            Self::InstanceCancelled,
            Self::EffectOutcomeUnknown,
            Self::DeliveryRecorded,
            Self::RegionOccurrenceStarted,
            Self::NodeExecutionRecorded,
            Self::HookExecuted,
            Self::ContextTransitioned,
        ]
    }

    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::InstanceStateChanged => "instance.state_changed",
            Self::InvocationStateChanged => "invocation.state_changed",
            Self::InstanceCreated => "instance.created",
            Self::InvocationAdmitted => "invocation.admitted",
            Self::ChildAttached => "child.attached",
            Self::AttemptRecorded => "attempt.recorded",
            Self::InvocationAttemptRecorded => "invocation.attempt_recorded",
            Self::CapabilityAttemptRecorded => "capability.attempt_recorded",
            Self::InvocationCommitted => "invocation.committed",
            Self::InvocationFailed => "invocation.failed",
            Self::InvocationCancelled => "invocation.cancelled",
            Self::EventCreated => "event.created",
            Self::EventAwaitRegistered => "event.await_registered",
            Self::InvocationParked => "invocation.parked",
            Self::EventTerminal => "event.terminal",
            Self::InvocationResumed => "invocation.resumed",
            Self::InstanceClosed => "instance.closed",
            Self::InstanceCancelled => "instance.cancelled",
            Self::EffectOutcomeUnknown => "effect.outcome_unknown",
            Self::DeliveryRecorded => "delivery.recorded",
            Self::RegionOccurrenceStarted => "region.occurrence_started",
            Self::NodeExecutionRecorded => "node_execution.recorded",
            Self::HookExecuted => "hook.executed",
            Self::ContextTransitioned => "context.transitioned",
        }
    }
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

/// One authoritative non-loop fact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFact {
    pub fact_id: String,
    pub event_sequence: u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_memberships: Option<Vec<LoopMembership>>,
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

/// One loop occurrence containing a NodeExecution.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopMembership {
    pub static_loop_id: String,
    pub loop_occurrence_id: String,
}

/// A committed loop back-edge. Every field is required at construction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopIterationCompletedFact {
    pub fact_id: String,
    pub event_sequence: u64,
    pub static_loop_id: String,
    pub loop_occurrence_id: String,
    pub iteration_index: u64,
    pub program_invocation_id: String,
    pub causal_node_execution_ids: Vec<String>,
}

impl LoopIterationCompletedFact {
    #[must_use]
    pub fn new(
        fact_id: String,
        event_sequence: u64,
        static_loop_id: String,
        loop_occurrence_id: String,
        iteration_index: u64,
        program_invocation_id: String,
        causal_node_execution_ids: Vec<String>,
    ) -> Self {
        Self {
            fact_id,
            event_sequence,
            static_loop_id,
            loop_occurrence_id,
            iteration_index,
            program_invocation_id,
            causal_node_execution_ids,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeExecutionScope {
    NonLoop,
    Loop {
        region_occurrence_id: String,
        static_region_id: String,
        loop_memberships: Vec<LoopMembership>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeExecutionRecordedFact {
    pub fact_id: String,
    pub event_sequence: u64,
    pub program_invocation_id: String,
    pub node_execution_id: String,
    pub air_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_node_execution_id: Option<String>,
    pub execution_scope: NodeExecutionScope,
}

/// One successful native model attempt committed in the runtime evidence batch.
/// Every coordinate is required so a post-commit consumer can prove exact
/// membership without trusting a parallel callback payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAttemptRecordedFact {
    pub fact_id: String,
    pub event_sequence: u64,
    pub program_invocation_id: String,
    pub node_execution_id: String,
    pub air_node_id: String,
    pub attempt_id: String,
    pub attempt_index: u32,
    pub model_effect_id: String,
    pub request_digest: String,
    pub model_target_ref: String,
    pub model_target_digest: String,
    pub model_deployment_ref: String,
    pub exact_port_binding_digest: String,
    pub target_commitment_digest: String,
    pub generation_cohort_digest: String,
    pub target_generation: u64,
    pub target_port_contract_digest: String,
    pub target_composition_digest: String,
    pub native_input_tokens: u64,
    pub native_output_tokens: u64,
}

/// Closed runtime evidence variants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fact_kind")]
pub enum Fact {
    #[serde(rename = "instance.state_changed")]
    InstanceStateChanged(RuntimeFact),
    #[serde(rename = "invocation.state_changed")]
    InvocationStateChanged(RuntimeFact),
    #[serde(rename = "instance.created")]
    InstanceCreated(RuntimeFact),
    #[serde(rename = "invocation.admitted")]
    InvocationAdmitted(RuntimeFact),
    #[serde(rename = "child.attached")]
    ChildAttached(RuntimeFact),
    #[serde(rename = "attempt.recorded")]
    AttemptRecorded(ModelAttemptRecordedFact),
    #[serde(rename = "invocation.attempt_recorded")]
    InvocationAttemptRecorded(RuntimeFact),
    #[serde(rename = "capability.attempt_recorded")]
    CapabilityAttemptRecorded(RuntimeFact),
    #[serde(rename = "invocation.committed")]
    InvocationCommitted(RuntimeFact),
    #[serde(rename = "invocation.failed")]
    InvocationFailed(RuntimeFact),
    #[serde(rename = "invocation.cancelled")]
    InvocationCancelled(RuntimeFact),
    #[serde(rename = "event.created")]
    EventCreated(RuntimeFact),
    #[serde(rename = "event.await_registered")]
    EventAwaitRegistered(RuntimeFact),
    #[serde(rename = "invocation.parked")]
    InvocationParked(RuntimeFact),
    #[serde(rename = "event.terminal")]
    EventTerminal(RuntimeFact),
    #[serde(rename = "invocation.resumed")]
    InvocationResumed(RuntimeFact),
    #[serde(rename = "instance.closed")]
    InstanceClosed(RuntimeFact),
    #[serde(rename = "instance.cancelled")]
    InstanceCancelled(RuntimeFact),
    #[serde(rename = "effect.outcome_unknown")]
    EffectOutcomeUnknown(RuntimeFact),
    #[serde(rename = "delivery.recorded")]
    DeliveryRecorded(RuntimeFact),
    #[serde(rename = "region.occurrence_started")]
    RegionOccurrenceStarted(RuntimeFact),
    #[serde(rename = "node_execution.recorded")]
    NodeExecutionRecorded(NodeExecutionRecordedFact),
    #[serde(rename = "hook.executed")]
    HookExecuted(RuntimeFact),
    #[serde(rename = "context.transitioned")]
    ContextTransitioned(RuntimeFact),
    #[serde(rename = "LoopIterationCompleted")]
    LoopIterationCompleted(LoopIterationCompletedFact),
}

impl Fact {
    pub fn from_runtime(kind: FactKind, fact: RuntimeFact) -> Self {
        match kind {
            FactKind::InstanceStateChanged => Self::InstanceStateChanged(fact),
            FactKind::InvocationStateChanged => Self::InvocationStateChanged(fact),
            FactKind::InstanceCreated => Self::InstanceCreated(fact),
            FactKind::InvocationAdmitted => Self::InvocationAdmitted(fact),
            FactKind::ChildAttached => Self::ChildAttached(fact),
            FactKind::AttemptRecorded => {
                panic!("AttemptRecorded requires its typed constructor")
            }
            FactKind::InvocationAttemptRecorded => Self::InvocationAttemptRecorded(fact),
            FactKind::CapabilityAttemptRecorded => Self::CapabilityAttemptRecorded(fact),
            FactKind::InvocationCommitted => Self::InvocationCommitted(fact),
            FactKind::InvocationFailed => Self::InvocationFailed(fact),
            FactKind::InvocationCancelled => Self::InvocationCancelled(fact),
            FactKind::EventCreated => Self::EventCreated(fact),
            FactKind::EventAwaitRegistered => Self::EventAwaitRegistered(fact),
            FactKind::InvocationParked => Self::InvocationParked(fact),
            FactKind::EventTerminal => Self::EventTerminal(fact),
            FactKind::InvocationResumed => Self::InvocationResumed(fact),
            FactKind::InstanceClosed => Self::InstanceClosed(fact),
            FactKind::InstanceCancelled => Self::InstanceCancelled(fact),
            FactKind::EffectOutcomeUnknown => Self::EffectOutcomeUnknown(fact),
            FactKind::DeliveryRecorded => Self::DeliveryRecorded(fact),
            FactKind::RegionOccurrenceStarted => Self::RegionOccurrenceStarted(fact),
            FactKind::HookExecuted => Self::HookExecuted(fact),
            FactKind::ContextTransitioned => Self::ContextTransitioned(fact),
            FactKind::NodeExecutionRecorded => {
                panic!("NodeExecutionRecorded requires its typed constructor")
            }
        }
    }

    #[must_use]
    pub fn fact_id(&self) -> &str {
        match self {
            Self::LoopIterationCompleted(fact) => &fact.fact_id,
            Self::NodeExecutionRecorded(fact) => &fact.fact_id,
            Self::AttemptRecorded(fact) => &fact.fact_id,
            _ => &self.runtime().expect("runtime variant").fact_id,
        }
    }

    #[must_use]
    pub fn event_sequence(&self) -> u64 {
        match self {
            Self::LoopIterationCompleted(fact) => fact.event_sequence,
            Self::NodeExecutionRecorded(fact) => fact.event_sequence,
            Self::AttemptRecorded(fact) => fact.event_sequence,
            _ => self.runtime().expect("runtime variant").event_sequence,
        }
    }

    #[must_use]
    pub fn runtime(&self) -> Option<&RuntimeFact> {
        runtime_fact_ref(self)
    }

    #[must_use]
    pub fn runtime_mut(&mut self) -> Option<&mut RuntimeFact> {
        runtime_fact_mut(self)
    }

    #[must_use]
    pub fn loop_iteration_completed(&self) -> Option<&LoopIterationCompletedFact> {
        match self {
            Self::LoopIterationCompleted(fact) => Some(fact),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_kind(&self, kind: FactKind) -> bool {
        match self {
            Self::LoopIterationCompleted(_) | Self::NodeExecutionRecorded(_) => false,
            _ => self.kind() == Some(kind),
        }
    }

    #[must_use]
    pub fn node_execution_recorded(&self) -> Option<&NodeExecutionRecordedFact> {
        match self {
            Self::NodeExecutionRecorded(fact) => Some(fact),
            _ => None,
        }
    }

    #[must_use]
    pub fn model_attempt_recorded(&self) -> Option<&ModelAttemptRecordedFact> {
        match self {
            Self::AttemptRecorded(fact) => Some(fact),
            _ => None,
        }
    }

    #[must_use]
    pub fn kind(&self) -> Option<FactKind> {
        Some(match self {
            Self::InstanceStateChanged(_) => FactKind::InstanceStateChanged,
            Self::InvocationStateChanged(_) => FactKind::InvocationStateChanged,
            Self::InstanceCreated(_) => FactKind::InstanceCreated,
            Self::InvocationAdmitted(_) => FactKind::InvocationAdmitted,
            Self::ChildAttached(_) => FactKind::ChildAttached,
            Self::AttemptRecorded(_) => FactKind::AttemptRecorded,
            Self::InvocationAttemptRecorded(_) => FactKind::InvocationAttemptRecorded,
            Self::CapabilityAttemptRecorded(_) => FactKind::CapabilityAttemptRecorded,
            Self::InvocationCommitted(_) => FactKind::InvocationCommitted,
            Self::InvocationFailed(_) => FactKind::InvocationFailed,
            Self::InvocationCancelled(_) => FactKind::InvocationCancelled,
            Self::EventCreated(_) => FactKind::EventCreated,
            Self::EventAwaitRegistered(_) => FactKind::EventAwaitRegistered,
            Self::InvocationParked(_) => FactKind::InvocationParked,
            Self::EventTerminal(_) => FactKind::EventTerminal,
            Self::InvocationResumed(_) => FactKind::InvocationResumed,
            Self::InstanceClosed(_) => FactKind::InstanceClosed,
            Self::InstanceCancelled(_) => FactKind::InstanceCancelled,
            Self::EffectOutcomeUnknown(_) => FactKind::EffectOutcomeUnknown,
            Self::DeliveryRecorded(_) => FactKind::DeliveryRecorded,
            Self::RegionOccurrenceStarted(_) => FactKind::RegionOccurrenceStarted,
            Self::NodeExecutionRecorded(_) => FactKind::NodeExecutionRecorded,
            Self::HookExecuted(_) => FactKind::HookExecuted,
            Self::ContextTransitioned(_) => FactKind::ContextTransitioned,
            Self::LoopIterationCompleted(_) => return None,
        })
    }
}

fn runtime_fact_ref(fact: &Fact) -> Option<&RuntimeFact> {
    match fact {
        Fact::InstanceStateChanged(value)
        | Fact::InvocationStateChanged(value)
        | Fact::InstanceCreated(value)
        | Fact::InvocationAdmitted(value)
        | Fact::ChildAttached(value)
        | Fact::InvocationAttemptRecorded(value)
        | Fact::CapabilityAttemptRecorded(value)
        | Fact::InvocationCommitted(value)
        | Fact::InvocationFailed(value)
        | Fact::InvocationCancelled(value)
        | Fact::EventCreated(value)
        | Fact::EventAwaitRegistered(value)
        | Fact::InvocationParked(value)
        | Fact::EventTerminal(value)
        | Fact::InvocationResumed(value)
        | Fact::InstanceClosed(value)
        | Fact::InstanceCancelled(value)
        | Fact::EffectOutcomeUnknown(value)
        | Fact::DeliveryRecorded(value)
        | Fact::RegionOccurrenceStarted(value)
        | Fact::HookExecuted(value)
        | Fact::ContextTransitioned(value) => Some(value),
        Fact::AttemptRecorded(_)
        | Fact::NodeExecutionRecorded(_)
        | Fact::LoopIterationCompleted(_) => None,
    }
}

fn runtime_fact_mut(fact: &mut Fact) -> Option<&mut RuntimeFact> {
    match fact {
        Fact::InstanceStateChanged(value)
        | Fact::InvocationStateChanged(value)
        | Fact::InstanceCreated(value)
        | Fact::InvocationAdmitted(value)
        | Fact::ChildAttached(value)
        | Fact::InvocationAttemptRecorded(value)
        | Fact::CapabilityAttemptRecorded(value)
        | Fact::InvocationCommitted(value)
        | Fact::InvocationFailed(value)
        | Fact::InvocationCancelled(value)
        | Fact::EventCreated(value)
        | Fact::EventAwaitRegistered(value)
        | Fact::InvocationParked(value)
        | Fact::EventTerminal(value)
        | Fact::InvocationResumed(value)
        | Fact::InstanceClosed(value)
        | Fact::InstanceCancelled(value)
        | Fact::EffectOutcomeUnknown(value)
        | Fact::DeliveryRecorded(value)
        | Fact::RegionOccurrenceStarted(value)
        | Fact::HookExecuted(value)
        | Fact::ContextTransitioned(value) => Some(value),
        Fact::AttemptRecorded(_)
        | Fact::NodeExecutionRecorded(_)
        | Fact::LoopIterationCompleted(_) => None,
    }
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
        let mut seen_fact_ids = HashSet::new();
        let mut seen_node_executions: HashMap<&str, (&str, &str, HashSet<(&str, &str)>)> =
            HashMap::new();
        let mut seen_model_attempt_ids = HashSet::new();
        let mut seen_model_attempt_indexes = HashSet::new();
        let mut completed_iterations = HashSet::new();
        let mut next_iteration: HashMap<(String, String, String), u64> = HashMap::new();
        for fact in &self.facts {
            if !seen_fact_ids.insert(fact.fact_id()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    fact.fact_id(),
                    "fact_id is not replay-stable and unique",
                ));
            }
            if let Some(prev) = previous
                && fact.event_sequence() <= prev
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::NonMonotonicEvidence,
                    fact.fact_id(),
                    "event_sequence is not strictly increasing",
                ));
            }
            previous = Some(fact.event_sequence());

            match fact {
                Fact::LoopIterationCompleted(completed) => {
                    if completed.causal_node_execution_ids.is_empty() {
                        verdict.push(Diagnostic::new(
                            DiagnosticCode::SchemaViolation,
                            &completed.fact_id,
                            "LoopIterationCompleted causality must be non-empty",
                        ));
                    }
                    let mut unique_causal_ids = HashSet::new();
                    for causal_id in &completed.causal_node_execution_ids {
                        if !unique_causal_ids.insert(causal_id.as_str()) {
                            verdict.push(Diagnostic::new(
                                DiagnosticCode::SchemaViolation,
                                &completed.fact_id,
                                "LoopIterationCompleted causality contains a duplicate",
                            ));
                        }
                        match seen_node_executions.get(causal_id.as_str()) {
                            Some((_, _, memberships))
                                if memberships.contains(&(
                                    completed.static_loop_id.as_str(),
                                    completed.loop_occurrence_id.as_str(),
                                )) => {}
                            _ => verdict.push(Diagnostic::new(
                                DiagnosticCode::SchemaViolation,
                                &completed.fact_id,
                                "LoopIterationCompleted causality is foreign or uncommitted",
                            )),
                        }
                    }
                    let identity = (
                        completed.static_loop_id.clone(),
                        completed.loop_occurrence_id.clone(),
                        completed.iteration_index,
                    );
                    if !completed_iterations.insert(identity) {
                        verdict.push(Diagnostic::new(
                            DiagnosticCode::SchemaViolation,
                            &completed.fact_id,
                            "loop iteration identity was replayed with a conflicting fact",
                        ));
                    }
                    let sequence_key = (
                        completed.static_loop_id.clone(),
                        completed.loop_occurrence_id.clone(),
                        completed.program_invocation_id.clone(),
                    );
                    let expected_index = next_iteration.entry(sequence_key).or_insert(0);
                    if completed.iteration_index == *expected_index {
                        *expected_index += 1;
                    } else {
                        verdict.push(Diagnostic::new(
                            DiagnosticCode::SchemaViolation,
                            &completed.fact_id,
                            "loop iteration index is not zero-based and contiguous",
                        ));
                    }
                }
                Fact::NodeExecutionRecorded(node) => {
                    let memberships: HashSet<(&str, &str)> = match &node.execution_scope {
                        NodeExecutionScope::NonLoop => HashSet::new(),
                        NodeExecutionScope::Loop {
                            region_occurrence_id,
                            static_region_id,
                            loop_memberships,
                        } => {
                            let memberships: HashSet<_> = loop_memberships
                                .iter()
                                .map(|membership| {
                                    (
                                        membership.static_loop_id.as_str(),
                                        membership.loop_occurrence_id.as_str(),
                                    )
                                })
                                .collect();
                            if loop_memberships.is_empty()
                                || !memberships.contains(&(
                                    static_region_id.as_str(),
                                    region_occurrence_id.as_str(),
                                ))
                            {
                                verdict.push(Diagnostic::new(
                                    DiagnosticCode::SchemaViolation,
                                    &node.fact_id,
                                    "loop NodeExecution scope must include its exact region occurrence",
                                ));
                            }
                            memberships
                        }
                    };
                    if seen_node_executions
                        .insert(
                            &node.node_execution_id,
                            (&node.program_invocation_id, &node.air_node_id, memberships),
                        )
                        .is_some()
                    {
                        verdict.push(Diagnostic::new(
                            DiagnosticCode::SchemaViolation,
                            &node.fact_id,
                            "node_execution_id conflicts with replay",
                        ));
                    }
                }
                Fact::AttemptRecorded(attempt) => {
                    match seen_node_executions.get(attempt.node_execution_id.as_str()) {
                        None => verdict.push(Diagnostic::new(
                            DiagnosticCode::SchemaViolation,
                            &attempt.fact_id,
                            "a model attempt requires a preceding committed NodeExecution",
                        )),
                        Some((program_invocation_id, _, _))
                            if *program_invocation_id != attempt.program_invocation_id =>
                        {
                            verdict.push(Diagnostic::new(
                                DiagnosticCode::SchemaViolation,
                                &attempt.fact_id,
                                "a model attempt must match its NodeExecution Program Invocation",
                            ));
                        }
                        Some((_, air_node_id, _)) if *air_node_id != attempt.air_node_id => {
                            verdict.push(Diagnostic::new(
                                DiagnosticCode::SchemaViolation,
                                &attempt.fact_id,
                                "a model attempt must match its NodeExecution AIR node",
                            ));
                        }
                        Some(_) => {}
                    }
                    let attempt_id_identity = (
                        attempt.program_invocation_id.as_str(),
                        attempt.node_execution_id.as_str(),
                        attempt.attempt_id.as_str(),
                    );
                    if !seen_model_attempt_ids.insert(attempt_id_identity) {
                        verdict.push(Diagnostic::new(
                            DiagnosticCode::SchemaViolation,
                            &attempt.fact_id,
                            "model attempt identity conflicts with replay",
                        ));
                    }
                    let attempt_index_identity = (
                        attempt.program_invocation_id.as_str(),
                        attempt.node_execution_id.as_str(),
                        attempt.attempt_index,
                    );
                    if !seen_model_attempt_indexes.insert(attempt_index_identity) {
                        verdict.push(Diagnostic::new(
                            DiagnosticCode::SchemaViolation,
                            &attempt.fact_id,
                            "model attempt index conflicts with replay",
                        ));
                    }
                    for (path, value) in [
                        ("request_digest", attempt.request_digest.as_str()),
                        (
                            "exact_port_binding_digest",
                            attempt.exact_port_binding_digest.as_str(),
                        ),
                    ] {
                        if !is_digest(value) {
                            verdict.push(Diagnostic::new(
                                DiagnosticCode::InvalidDigest,
                                format!("{}.{}", attempt.fact_id, path),
                                "model attempt digest is not a lowercase sha256 value",
                            ));
                        }
                    }
                }
                runtime_variant => {
                    let runtime = runtime_variant
                        .runtime()
                        .expect("exhaustive runtime variant");
                    let kind = runtime_variant.kind().expect("runtime variant kind");
                    if kind == FactKind::EffectOutcomeUnknown {
                        let claims_success = runtime.model_outcome
                            == Some(ModelOutcome::CommittedSuccess)
                            || runtime
                                .invocation_state
                                .is_some_and(InvocationState::is_committed);
                        if claims_success {
                            verdict.push(Diagnostic::new(
                                DiagnosticCode::OutcomeUnknownClaimsSuccess,
                                &runtime.fact_id,
                                "an uncertain effect must not claim a committed/success outcome",
                            ));
                        }
                    }
                    let missing = match kind {
                        FactKind::RegionOccurrenceStarted => {
                            runtime.region_occurrence_id.is_none()
                                || runtime.static_region_id.is_none()
                        }
                        FactKind::HookExecuted => {
                            runtime.hook_execution_id.is_none()
                                || runtime.hook_id.is_none()
                                || runtime.hook_scope.is_none()
                                || runtime.hook_phase.is_none()
                                || runtime.context_before_ref.is_none()
                                || runtime.context_after_ref.is_none()
                        }
                        FactKind::ContextTransitioned => {
                            runtime.context_transition_id.is_none()
                                || runtime.context_before_ref.is_none()
                                || runtime.context_after_ref.is_none()
                        }
                        _ => false,
                    };
                    if missing {
                        verdict.push(Diagnostic::new(
                            DiagnosticCode::SchemaViolation,
                            &runtime.fact_id,
                            "evidence join fact is missing a required typed join",
                        ));
                    }
                }
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
