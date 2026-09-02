//! The canonical runtime driver: execute a five-operation AIR node graph through
//! injected kernel ports and commit atomically.
//!
//! The driver derives an execution schedule from structural AIR when present,
//! interleaving compiled Hook callsites and explicit Context commits around the
//! authored semantic operation order. It dispatches each semantic operation to
//! its exact injected port — `model.call` to the model inference port,
//! `capability.invoke` to the generic Capability port,
//! `program.new`/`program.invoke` to the composition port, `await.event` to the
//! event port — threading explicit Context through typed Hooks. It builds the
//! runtime-evidence batch from the real effects and commits the whole write set
//! atomically through the one Execution Commit port. There is no router, no
//! fallback, and no first-available selection: a model effect receives one
//! materialized binding and fails closed if it does not match the authored
//! target.
//!
//! Two entry points share one op-dispatch core ([`drive_from`]):
//! [`execute`] walks once and commits (single-shot); [`execute_resumable`] and
//! [`resume`] add durable park/resume so a conversational session can suspend at
//! an `await.event` and continue when input is delivered. The suspend/resume
//! behavior is the only difference — effect dispatch, hooks, and the one atomic
//! commit are identical.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

use apxm_inference::{
    BindingError, CommittedInferenceDispatch, InferenceTargetCommitment, InferenceUsageLineage,
    ModelBindingAdmission, ModelCallPreparation, ModelCallRequest, ModelCallRequestError,
    ModelCallRequestMetadataPort, ModelInferencePort, ModelOutcome, ModelTargetRef, RetryPolicy,
    TargetCommitmentError, TypedError, Usage, dispatch_committed_inference_async,
};
use apxm_kernel::{
    AtomicWriteSet, CommittedContinuation, EventApplicationResult, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExecutionCommitTuple, PortSlot,
    PrecommitEvidenceRef, PreparedSessionOutputRef, ProgramInstanceRef, ProgramInvocationRef,
    ResourceCeilings, SESSION_OUTPUT_REF_CONTRACT, SessionOutputPreparation,
    SessionOutputVisibility, canonical_json_bytes, continuation_digest,
    runtime_evidence_and_observation_digest,
};
use apxm_program::air::{
    AirModule, ControlPredicate, PredicateComparator, PredicateLiteral, SemanticOp, SemanticOpKind,
};
use apxm_core::types::host_capability::{
    HostCapabilityOutcomeKind, HostCapabilitySettlement, host_capability_request_id,
    is_host_capability_ref,
};
use apxm_program::capability::{CapabilityInvocationAuthority, CapabilityRequestError};
use apxm_program::common::{ErrorCategory as EvidenceErrorCategory, TypedErrorEnvelope, TypedRef};
use apxm_program::external_agent::ExternalAgentEvidence;
use apxm_program::frontend_graph::ValueExpression;
use apxm_program::frontend_graph::{HookBinding, HookReturnMode};
use apxm_program::grammar::is_digest;
use apxm_program::runtime_evidence::{
    Fact, FactKind, HookPhase as EvidenceHookPhase, HookScope as EvidenceHookScope, InstanceState,
    InvocationState, LoopIterationCompletedFact, LoopMembership, ModelAttemptRecordedFact,
    NodeExecutionRecordedFact, NodeExecutionScope, ResolvedPermission, RuntimeFact,
};

use crate::ExecutionPortBundle;
use crate::observe::{
    ObservationFailurePolicy, ObservationSink, ObservationSinkError, make_observation, unix_time_ms,
};
use crate::operational_usage::{
    CommittedNativeModelUsage, CommittedNativeModelUsageError, CommittedNativeModelUsageOutcome,
    CommittedNativeModelUsagePort, EvidencePositionRef, EvidencePositionRefType,
};
use crate::ports::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionReceiver, CompositionRequest, EventAwait, EventOutcome, EventPort, EventRef,
    EventRefError,
};
use crate::resume::{
    Continuation, ContinuationError, DurableLoopFrame, HookTargetSnapshot, RunOutcome,
};
use crate::structural::{ScheduleStep, build_schedule};

/// Structural limits protect schedule construction and recursive value
/// materialization before an admitted artifact can consume runtime memory.
pub const MAX_SEMANTIC_OPERATIONS: usize = 4096;
pub const MAX_STRUCTURAL_REGIONS: usize = 1024;
pub const MAX_VALUE_ASSEMBLIES: usize = 8192;
pub const MAX_HOOK_BINDINGS: usize = 1024;
pub const MAX_INITIAL_VALUES: usize = 4096;
pub const MAX_SCHEDULE_STEPS: usize = 16_384;
pub const MAX_EXPRESSION_DEPTH: usize = 64;

/// The exact set of injected ports the driver drives. Every port is a single
/// admitted implementation; the driver holds no registry and does no discovery.
pub struct ExecutionPorts {
    model_inference: Arc<dyn ModelInferencePort + Send + Sync>,
    model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
    capability: Arc<dyn CapabilityPort>,
    events: Arc<dyn EventPort>,
    composition: Arc<dyn CompositionPort>,
    execution_commit: Arc<dyn ExecutionCommitPort>,
    hook_handlers: Arc<dyn StaticHookHandlerPort>,
    operational_usage: Option<Arc<dyn CommittedNativeModelUsagePort>>,
    resource_ceilings: Option<ResourceCeilings>,
    observation_sink: Option<Arc<dyn ObservationSink>>,
    observation_failure_policy: ObservationFailurePolicy,
    cancellation: CancellationToken,
}

/// Cooperative cancellation shared by a profile and its active driver.
#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<CancellationState>);

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    /// Wait until cancellation is requested. The check around registration
    /// closes the race where cancellation arrives between the initial check
    /// and parking on the notification.
    pub async fn cancelled(&self) {
        let notified = self.0.notify.notified();
        if !self.is_cancelled() {
            notified.await;
        }
    }
}

/// Why the canonical driver cannot be constructed from a port bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionPortsError {
    MissingAdmittedPort(PortSlot),
}

impl std::fmt::Display for ExecutionPortsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ExecutionPortsError {}

impl ExecutionPorts {
    /// Construct the driver ports from one complete admitted execution bundle.
    ///
    /// Ordinary capability, model, and commit effects are copied only from
    /// their exact admitted slots. Durable event and Program
    /// composition ports have already been joined to their exact bindings by
    /// [`ExecutionPortBundle::construct`].
    pub fn from_admitted_bundle(
        bundle: &ExecutionPortBundle,
        model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
        hook_handlers: Arc<dyn StaticHookHandlerPort>,
    ) -> Result<Self, ExecutionPortsError> {
        let kernel = bundle.kernel();
        let model_inference =
            kernel
                .model_inference()
                .cloned()
                .ok_or(ExecutionPortsError::MissingAdmittedPort(
                    PortSlot::ModelInference,
                ))?;
        let capability =
            kernel
                .capability()
                .cloned()
                .ok_or(ExecutionPortsError::MissingAdmittedPort(
                    PortSlot::Capability,
                ))?;
        Ok(Self {
            model_inference,
            model_call_request_metadata,
            capability,
            events: bundle.events().clone(),
            composition: bundle.composition().clone(),
            execution_commit: kernel.execution_commit().clone(),
            hook_handlers,
            operational_usage: None,
            resource_ceilings: None,
            observation_sink: None,
            observation_failure_policy: ObservationFailurePolicy::FailOpen,
            cancellation: CancellationToken::new(),
        })
    }

    /// Attach the immutable ceilings carried by the verified admission.
    #[must_use]
    pub fn with_resource_ceilings(mut self, resource_ceilings: ResourceCeilings) -> Self {
        self.resource_ceilings = Some(resource_ceilings);
        self
    }

    /// Attach the single committed-native-model-usage publisher admitted by
    /// composition. The driver owns the closed measurement but does not
    /// discover a destination or add Server-owned authority or pricing data.
    pub fn with_committed_native_model_usage_port(
        mut self,
        operational_usage: Arc<dyn CommittedNativeModelUsagePort>,
    ) -> Self {
        self.operational_usage = Some(operational_usage);
        self
    }

    /// Attach a bounded/non-authoritative live observation sink.
    #[must_use]
    pub fn with_observation_sink(mut self, sink: Arc<dyn ObservationSink>) -> Self {
        self.observation_sink = Some(sink);
        self
    }

    /// Choose what a sink failure means for this execution. The default is
    /// fail-open because live observations cannot establish execution truth.
    #[must_use]
    pub fn with_observation_failure_policy(mut self, policy: ObservationFailurePolicy) -> Self {
        self.observation_failure_policy = policy;
        self
    }

    /// Attach the caller-owned cancellation signal for this active invocation.
    #[must_use]
    pub fn with_cancellation_token(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

/// The result of executing one exact statically bound Hook handler.
#[derive(Clone, Debug, PartialEq)]
pub enum StaticHookResult {
    Keep {
        assigned_context: Option<Value>,
    },
    Replace {
        assigned_context: Option<Value>,
        result: Value,
    },
}

/// A compiled Hook binding could not be executed by the injected handler port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticHookExecutionError {
    pub hook_id: String,
    pub message: String,
}

/// One Hook's turn to act, after its captured body has already run.
pub struct StaticHookInvocation<'a> {
    pub binding: &'a HookBinding,
    pub context: &'a Value,
    pub result: &'a Value,
    /// The Context value the Hook's captured body assigned, materialized from
    /// the Hook's own AIR region. `None` when the body assigns nothing.
    pub captured_context: Option<Value>,
}

/// Apply one compiled Hook binding's declared return contract.
///
/// The port receives the artifact binding directly. It cannot discover a Hook by
/// event name or register callbacks dynamically, and it never performs the
/// Hook's effects: those are captured operations in the Hook's own AIR region
/// and have already been dispatched through the ordinary admitted ports.
#[async_trait]
pub trait StaticHookHandlerPort: Send + Sync {
    async fn execute(
        &self,
        invocation: StaticHookInvocation<'_>,
    ) -> Result<StaticHookResult, StaticHookExecutionError>;
}

/// The canonical handler for Hooks whose bodies are captured as AIR.
///
/// A captured Hook has no hidden behavior left to run, so the whole contract is
/// the one the compiler derived from the body: an observing Hook changes
/// nothing, and a replacing Hook installs exactly the Context value its body
/// assigned. Nothing here can perform an effect the artifact does not declare —
/// which is the point of capturing bodies instead of resolving opaque handlers.
pub struct CapturedHookBodyHandler;

#[async_trait]
impl StaticHookHandlerPort for CapturedHookBodyHandler {
    async fn execute(
        &self,
        invocation: StaticHookInvocation<'_>,
    ) -> Result<StaticHookResult, StaticHookExecutionError> {
        match invocation.binding.return_mode {
            HookReturnMode::Observe => Ok(StaticHookResult::Keep {
                assigned_context: None,
            }),
            HookReturnMode::ReplaceResult => {
                let assigned_context =
                    invocation
                        .captured_context
                        .ok_or_else(|| StaticHookExecutionError {
                            hook_id: invocation.binding.hook_id.clone(),
                            message: "a replacing Hook's captured body assigned no Context value"
                                .to_string(),
                        })?;
                Ok(StaticHookResult::Keep {
                    assigned_context: Some(assigned_context),
                })
            }
        }
    }
}

/// One canonical execution request: the AIR to run, the materialized
/// model-binding admission, per-node Capability invocation admissions,
/// immutable instance and invocation identities, and prepared write set.
pub struct ExecutionRequest {
    pub air: AirModule,
    /// Exact externally supplied SSA values (entry parameters, admitted inputs).
    pub initial_values: BTreeMap<String, Value>,
    pub hook_bindings: Vec<HookBinding>,
    pub model_admission: ModelBindingAdmission,
    pub capability_invocations: BTreeMap<String, CapabilityInvocationAdmission>,
    pub program_instance_ref: ProgramInstanceRef,
    pub program_invocation_ref: ProgramInvocationRef,
    pub commit_id: String,
    pub write_set: AtomicWriteSet,
}

/// Admitted Auth/Server references for one exact `capability.invoke` AIR node.
/// Application arguments come exclusively from the authored SSA operand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityInvocationAdmission {
    pub capability_ref: String,
    pub authority: CapabilityInvocationAuthority,
    /// The decision the resolution layer stack reached for this capability.
    /// Anything short of an outright allow refuses the effect before an
    /// argument reaches an implementation.
    ///
    /// Required, with no serde default: an admission that carries no decision
    /// is neither an allow nor a refusal, so a wire payload that omits the
    /// field fails to deserialize rather than executing undecided. That covers
    /// a `Continuation` rehydrated from persisted bytes as well as a
    /// composition root that fills this map in directly.
    pub permission: ResolvedPermission,
}

/// Where a [`CapabilityGrantSet`]'s names came from.
///
/// The two origins exist because two kinds of graph reach this boundary and
/// only one of them is authored by a person. An author's Capability reference
/// has to name something the machine can actually dispatch; a lowering-shape
/// conformance corpus deliberately carries opaque references it never
/// dispatches, to pin AIR structure rather than effect behaviour. Recording
/// which one minted a grant set is what lets the gate hold authors to the
/// catalogue without breaking the corpora — and the corpus exemption is stated
/// by name, at the one call site that takes it, rather than inferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityGrantOrigin {
    /// Names read off a live capability registry or an authored package's
    /// declared implementations. Every name resolves to something dispatchable.
    RegisteredImplementations,
    /// Opaque names stated by a lowering-shape conformance corpus. They are
    /// never dispatched, so they are exempt from catalogue resolution.
    ConformanceCorpus,
}

impl CapabilityGrantOrigin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RegisteredImplementations => "registered implementations",
            Self::ConformanceCorpus => "conformance corpus",
        }
    }
}

/// An authored Capability reference that no grant in the set covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityNotGranted {
    pub authored: String,
    pub granted: Vec<String>,
    pub origin: CapabilityGrantOrigin,
}

impl std::fmt::Display for CapabilityNotGranted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "capability '{}' is granted by nothing this invocation admits: the grant set was \
             derived from {} and covers [{}]",
            self.authored,
            self.origin.as_str(),
            self.granted.join(", ")
        )
    }
}

impl std::error::Error for CapabilityNotGranted {}

/// The finite set of Capability references one invocation may resolve.
///
/// This is the Capability analogue of [`ModelBindingAdmission`]: a composition
/// root states the exact set once, from a source that is not the program, and
/// every authored reference is resolved against it. The reference an admission
/// carries is then owned by the grant set, not copied out of the AIR operand
/// it will later be compared against — which is what makes the driver's
/// authored-versus-admitted check a real comparison rather than `x == x`.
#[derive(Clone, Debug)]
pub struct CapabilityGrantSet {
    granted: BTreeSet<String>,
    origin: CapabilityGrantOrigin,
}

impl CapabilityGrantSet {
    /// Grants derived from a live capability registry or an authored package's
    /// declared implementations.
    #[must_use]
    pub fn from_registered_implementations(
        names: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            granted: names.into_iter().map(Into::into).collect(),
            origin: CapabilityGrantOrigin::RegisteredImplementations,
        }
    }

    /// Grants stated by a lowering-shape conformance corpus, whose references
    /// are opaque by design and never dispatched.
    ///
    /// Calling this is the corpus declaring itself. Nothing infers the
    /// exemption from the shape of the reference.
    #[must_use]
    pub fn for_conformance_corpus(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            granted: names.into_iter().map(Into::into).collect(),
            origin: CapabilityGrantOrigin::ConformanceCorpus,
        }
    }

    /// Resolve an authored reference to the granted name that covers it.
    fn resolve(&self, authored: &str) -> Result<&str, CapabilityNotGranted> {
        self.granted
            .get(authored)
            .map(String::as_str)
            .ok_or_else(|| CapabilityNotGranted {
                authored: authored.to_string(),
                granted: self.granted.iter().cloned().collect(),
                origin: self.origin,
            })
    }

    /// Admit one authored Capability reference under this grant set. The
    /// caller must have a resolved decision to hand over; there is no way to
    /// admit a reference the permission stack never ruled on.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityNotGranted`] when no grant covers the reference.
    pub fn admit(
        &self,
        authored: &str,
        authority: CapabilityInvocationAuthority,
        permission: ResolvedPermission,
    ) -> Result<CapabilityInvocationAdmission, CapabilityNotGranted> {
        Ok(CapabilityInvocationAdmission {
            capability_ref: self.resolve(authored)?.to_string(),
            authority,
            permission,
        })
    }
}

/// The typed outcome of one executed node.
#[derive(Debug)]
pub enum NodeOutcome {
    Model {
        node_id: String,
        outcome: ModelOutcome,
        result: Value,
        replaced: bool,
    },
    Capability {
        node_id: String,
        outcome: CapabilityOutcome,
        replaced: bool,
    },
    ExternalAgent {
        node_id: String,
        evidence: ExternalAgentEvidence,
    },
    ProgramNew {
        node_id: String,
        outcome: CompositionOutcome,
    },
    ProgramInvoke {
        node_id: String,
        outcome: CompositionOutcome,
    },
    AwaitEvent {
        node_id: String,
        outcome: EventOutcome,
    },
}

/// The result of one canonical run: per-node outcomes, the native model usage
/// accumulated from `model.call` effects, the External Agent evidence (peer
/// usage stays here as provenance, never in native usage), the threaded-out
/// Context, and the atomic commit outcome.
#[derive(Debug)]
pub struct RunReport {
    pub node_outcomes: Vec<NodeOutcome>,
    pub native_usage: Usage,
    pub external_agent_evidence: Vec<ExternalAgentEvidence>,
    pub final_context: Value,
    pub commit: ExecutionCommitResult,
    /// Terminal truth derived from the same evidence batch sent to the
    /// commit port. A successful commit therefore cannot be mistaken for a
    /// successful invocation when the batch records failure, cancellation, or
    /// uncertain effect outcome.
    pub terminal_status: RunTerminalStatus,
    /// The outcome of publishing the exact committed native model attempts.
    /// External-agent/ACP usage never enters this Agents-owned contract.
    pub operational_usage: CommittedNativeModelUsageOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunTerminalStatus {
    CommittedReturn,
    Failed,
    Cancelled,
    OutcomeUnknown,
}

/// Why a canonical run could not be driven.
#[derive(Debug)]
pub enum ExecutionError {
    InvalidAir {
        message: String,
    },
    MissingOperand {
        node_id: String,
        operand: &'static str,
    },
    MissingControlValue {
        region_id: String,
        value_id: String,
    },
    InvalidControlPredicate {
        region_id: String,
        message: String,
    },
    InvalidValueExpression {
        value_id: String,
        message: String,
    },
    ProgramThrew {
        region_id: String,
    },
    MissingModelOutput {
        node_id: String,
    },
    MissingCapabilityInvocationAdmission {
        node_id: String,
    },
    CapabilityInvocationAdmissionMismatch {
        node_id: String,
        authored: String,
        admitted: String,
    },
    CapabilityRequest(CapabilityRequestError),
    Binding(BindingError),
    TargetCommitment(TargetCommitmentError),
    Lineage(apxm_inference::LineageError),
    ModelRequest(ModelCallRequestError),
    ModelRequestMetadata(TypedError),
    StaticHook(StaticHookExecutionError),
    Continuation(ContinuationError),
    InvalidEventRef {
        node_id: String,
        source: EventRefError,
    },
    EventRefMismatch {
        expected: EventRef,
        delivered: EventRef,
    },
    EventDeliveryRequiresRef {
        program_instance_ref: ProgramInstanceRef,
    },
    UnprovenEventWake {
        program_instance_ref: ProgramInstanceRef,
    },
    InvalidCommitRequest {
        message: String,
    },
    Commit(ExecutionCommitResult),
    ResourceLimitExceeded {
        resource: &'static str,
        limit: u64,
        observed: u64,
    },
    Observation(ObservationSinkError),
    OutputPreparation(String),
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAir { message } => write!(f, "AIR rejected before execution: {message}"),
            Self::MissingOperand { node_id, operand } => {
                write!(f, "node {node_id} is missing operand {operand}")
            }
            Self::MissingControlValue {
                region_id,
                value_id,
            } => write!(
                f,
                "structural region {region_id} reads unavailable typed value {value_id}"
            ),
            Self::InvalidControlPredicate { region_id, message } => {
                write!(
                    f,
                    "structural region {region_id} has invalid predicate: {message}"
                )
            }
            Self::InvalidValueExpression { value_id, message } => {
                write!(
                    f,
                    "value {value_id} has invalid authored expression: {message}"
                )
            }
            Self::ProgramThrew { region_id } => {
                write!(
                    f,
                    "program exited through authored throw region {region_id}"
                )
            }
            Self::MissingModelOutput { node_id } => {
                write!(
                    f,
                    "successful model node {node_id} returned no typed output"
                )
            }
            Self::MissingCapabilityInvocationAdmission { node_id } => {
                write!(f, "node {node_id} has no admitted Capability invocation")
            }
            Self::CapabilityInvocationAdmissionMismatch {
                node_id,
                authored,
                admitted,
            } => write!(
                f,
                "node {node_id} authored Capability {authored} but admission carries {admitted}"
            ),
            Self::CapabilityRequest(error) => write!(f, "Capability request error: {error}"),
            Self::Binding(error) => write!(f, "model binding error: {error}"),
            Self::TargetCommitment(error) => write!(f, "target commitment error: {error}"),
            Self::Lineage(error) => write!(f, "usage lineage error: {error}"),
            Self::ModelRequest(error) => write!(f, "model request error: {error}"),
            Self::ModelRequestMetadata(error) => {
                write!(f, "model request metadata error: {}", error.message)
            }
            Self::StaticHook(error) => {
                write!(
                    f,
                    "Hook {} execution failed: {}",
                    error.hook_id, error.message
                )
            }
            Self::Continuation(error) => write!(f, "continuation error: {error}"),
            Self::InvalidEventRef { node_id, source } => {
                write!(f, "node {node_id} has an invalid event_ref: {source}")
            }
            Self::EventRefMismatch {
                expected,
                delivered,
            } => {
                write!(
                    f,
                    "event reference mismatch: expected {expected}, delivered {delivered}"
                )
            }
            Self::EventDeliveryRequiresRef {
                program_instance_ref,
            } => {
                write!(
                    f,
                    "continuation for Program Instance {} requires an EventRef delivery",
                    program_instance_ref.as_str()
                )
            }
            Self::UnprovenEventWake {
                program_instance_ref,
            } => {
                write!(
                    f,
                    "Event wake for Program Instance {} requires a fulfilled application",
                    program_instance_ref.as_str()
                )
            }
            Self::InvalidCommitRequest { message } => {
                write!(f, "invalid atomic commit request: {message}")
            }
            Self::Commit(result) => write!(f, "atomic execution commit failed: {}", result.label()),
            Self::ResourceLimitExceeded {
                resource,
                limit,
                observed,
            } => write!(
                f,
                "admitted {resource} ceiling exceeded: observed {observed}, limit {limit}"
            ),
            Self::Observation(error) => write!(f, "execution observation failed: {error}"),
            Self::OutputPreparation(error) => {
                write!(f, "session output preparation failed: {error}")
            }
        }
    }
}

impl std::error::Error for ExecutionError {}

/// Read a typed SSA operand by slot name. For exact-reference slots
/// (`model_ref`, `capability_ref`, `program_ref`, `event_ref`, …) the
/// compiler places the exact reference string as the operand's `value_id`, so a
/// slot lookup returns that reference directly.
fn operand_str(op: &SemanticOp, key: &str) -> Option<String> {
    op.operands
        .iter()
        .find(|operand| operand.slot == key)
        .map(|operand| operand.value_id.clone())
}

/// Resolve the composition receiver from typed operands. The compiler encodes
/// the receiver kind in the operand `type_ref` (`ProgramRef` vs
/// `ProgramInstanceRef`) and the exact reference string in its `value_id`.
fn composition_receiver(op: &SemanticOp) -> Result<CompositionReceiver, ExecutionError> {
    let receiver = op
        .operands
        .iter()
        .find(|operand| operand.slot == "receiver")
        .ok_or(ExecutionError::MissingOperand {
            node_id: op.node_id.clone(),
            operand: "receiver",
        })?;

    if receiver.type_ref == "ProgramInstanceRef" {
        Ok(CompositionReceiver::Instance {
            program_instance_ref: receiver.value_id.clone(),
        })
    } else if receiver.type_ref == "ProgramRef" {
        Ok(CompositionReceiver::Program {
            program_ref: receiver.value_id.clone(),
        })
    } else {
        Err(ExecutionError::MissingOperand {
            node_id: op.node_id.clone(),
            operand: "receiver",
        })
    }
}

fn fact(
    program_invocation_id: &str,
    seq: u64,
    kind: FactKind,
    instance_state: Option<InstanceState>,
    invocation_state: Option<InvocationState>,
    node_execution_id: Option<String>,
    commit_sequence: Option<u64>,
) -> Fact {
    Fact::from_runtime(
        kind,
        RuntimeFact {
            fact_id: format!("fact.{program_invocation_id}.{seq}"),
            event_sequence: seq,
            ownership_epoch: None,
            instance_state,
            invocation_state,
            event_state: None,
            model_outcome: None,
            commit_sequence,
            node_execution_id,
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
            capability_ref: None,
            permission_decision: None,
            typed_error: None,
        },
    )
}

fn runtime_fact_mut(fact: &mut Fact) -> &mut RuntimeFact {
    fact.runtime_mut()
        .expect("runtime helper constructed a non-loop fact")
}

fn evidence_error_category(category: apxm_inference::ErrorCategory) -> EvidenceErrorCategory {
    match category {
        apxm_inference::ErrorCategory::Validation => EvidenceErrorCategory::Validation,
        apxm_inference::ErrorCategory::Admission => EvidenceErrorCategory::Admission,
        apxm_inference::ErrorCategory::Authority => EvidenceErrorCategory::Authority,
        apxm_inference::ErrorCategory::Configuration => EvidenceErrorCategory::Configuration,
        apxm_inference::ErrorCategory::Unavailable => EvidenceErrorCategory::Unavailable,
        apxm_inference::ErrorCategory::Conflict => EvidenceErrorCategory::Conflict,
        apxm_inference::ErrorCategory::OutcomeUnknown => EvidenceErrorCategory::OutcomeUnknown,
        apxm_inference::ErrorCategory::Internal => EvidenceErrorCategory::Internal,
    }
}

fn model_failure_envelope(node_execution_id: &str, error: &TypedError) -> TypedErrorEnvelope {
    TypedErrorEnvelope {
        error_id: format!("model-failure.{node_execution_id}"),
        category: evidence_error_category(error.category),
        code_ref: error.code.clone(),
        message: error.message.clone(),
        details_digest: None,
    }
}

fn unavailable_failure_envelope(
    kind: &str,
    node_execution_id: &str,
    message: &str,
) -> TypedErrorEnvelope {
    TypedErrorEnvelope {
        error_id: format!("{kind}-failure.{node_execution_id}"),
        category: EvidenceErrorCategory::Unavailable,
        code_ref: format!("{kind}.failed"),
        message: message.to_owned(),
        details_digest: None,
    }
}

/// The mutable run accumulators threaded through op dispatch. Shared by the
/// single-shot and resumable paths so both build identical evidence.
struct DriveState {
    node_outcomes: Vec<NodeOutcome>,
    native_usage: Usage,
    committed_model_attempts: Vec<ModelAttemptRecordedFact>,
    committed_model_lineages: Vec<InferenceUsageLineage>,
    external_agent_evidence: Vec<ExternalAgentEvidence>,
    context: Value,
    last_result: Value,
    values: BTreeMap<String, Value>,
    last_result_value_id: Option<String>,
    hook_target_snapshots: BTreeMap<String, HookTargetSnapshot>,
    branch_decisions: BTreeMap<String, usize>,
    last_operation_succeeded: bool,
    batch: Vec<Fact>,
    /// Scheduler-owned observations retained for the atomic commit tuple.
    /// This is authoritative only once the surrounding Execution Commit
    /// succeeds; the live sink remains a bounded, non-authoritative view.
    durable_observations: Vec<apxm_runtime_protocol::ExecutionObservation>,
    /// Exact event identity for the next wait/resume observation.
    pending_event_ref: Option<String>,
    /// The host-fulfilled Capability request or settlement the next
    /// `capability_requested`/`capability_settled` observation carries. Staged
    /// the way `pending_event_ref` is, so no observation call site that has
    /// nothing to say about a host capability has to say so.
    pending_host_capability: Option<apxm_runtime_protocol::HostCapabilityObservation>,
    /// Observation position is independent from the evidence/fact sequence;
    /// publishing a live observation must never perturb scheduler identities.
    observation_seq: u64,
    seq: u64,
    program_invocation_id: String,
    active_loops: Vec<DurableLoopFrame>,
    last_model_node_execution_id: Option<String>,
    last_program_new_node_execution_id: Option<String>,
    committed_output_node_execution_id: Option<String>,
    committed_output_occurrence_id: Option<String>,
    committed_output_region_occurrence_id: Option<String>,
    current_node_execution_id: Option<String>,
    current_occurrence_id: Option<String>,
    current_region_occurrence_id: Option<String>,
    terminal_node_execution_id: Option<String>,
    terminal_occurrence_id: Option<String>,
    terminal_region_occurrence_id: Option<String>,
}

impl DriveState {
    fn note_terminal_coordinates(&mut self, node_execution_id: Option<&str>) {
        let Some(node_execution_id) = node_execution_id else {
            return;
        };
        self.terminal_node_execution_id = Some(node_execution_id.to_owned());
        if self.current_node_execution_id.as_deref() == Some(node_execution_id) {
            self.terminal_occurrence_id = self.current_occurrence_id.clone();
            self.terminal_region_occurrence_id = self.current_region_occurrence_id.clone();
        }
    }

    fn append_invocation_failure(&mut self, node_execution_id: &str, error: TypedErrorEnvelope) {
        self.note_terminal_coordinates(Some(node_execution_id));
        self.seq += 1;
        let mut failure = fact(
            &self.program_invocation_id,
            self.seq,
            FactKind::InvocationFailed,
            None,
            Some(InvocationState::Failed),
            Some(node_execution_id.to_owned()),
            None,
        );
        runtime_fact_mut(&mut failure).typed_error = Some(error);
        self.batch.push(failure);
    }

    fn append_invocation_cancelled(&mut self, node_execution_id: Option<&str>) {
        self.note_terminal_coordinates(node_execution_id);
        self.seq += 1;
        self.batch.push(fact(
            &self.program_invocation_id,
            self.seq,
            FactKind::InvocationCancelled,
            None,
            Some(InvocationState::Cancelled),
            node_execution_id.map(str::to_owned),
            None,
        ));
    }

    fn append_effect_outcome_unknown(&mut self, node_execution_id: &str, effect_id: &str) {
        self.note_terminal_coordinates(Some(node_execution_id));
        self.seq += 1;
        let mut unknown = fact(
            &self.program_invocation_id,
            self.seq,
            FactKind::EffectOutcomeUnknown,
            None,
            None,
            Some(node_execution_id.to_owned()),
            None,
        );
        runtime_fact_mut(&mut unknown).effect_outcome_ref = Some(TypedRef {
            ref_type: "EffectOutcomeRef".to_owned(),
            target: effect_id.to_owned(),
            digest: None,
        });
        self.batch.push(unknown);
    }

    fn has_terminal_non_success(&self) -> bool {
        self.batch.iter().any(|fact| {
            fact.is_kind(FactKind::InvocationFailed)
                || fact.is_kind(FactKind::InvocationCancelled)
                || fact.is_kind(FactKind::EffectOutcomeUnknown)
                || matches!(
                    fact.runtime().and_then(|runtime| runtime.invocation_state),
                    Some(InvocationState::Failed | InvocationState::Cancelled)
                )
        }) || self
            .node_outcomes
            .iter()
            .any(|outcome| node_outcome_terminal_status(outcome).is_some())
    }

    fn has_unknown_outcome(&self) -> bool {
        self.durable_observations.iter().any(|observation| {
            observation.observation_kind == apxm_runtime_protocol::ObservationKind::OutcomeUnknown
        }) || self.node_outcomes.iter().any(|outcome| match outcome {
            NodeOutcome::Model { outcome, .. } => {
                matches!(outcome, ModelOutcome::ModelOutcomeUnknown { .. })
            }
            NodeOutcome::Capability { outcome, .. } => {
                matches!(outcome, CapabilityOutcome::OutcomeUnknown { .. })
            }
            NodeOutcome::ExternalAgent { .. }
            | NodeOutcome::ProgramNew { .. }
            | NodeOutcome::ProgramInvoke { .. }
            | NodeOutcome::AwaitEvent { .. } => false,
        })
    }

    /// Publish one redacted live observation using the same monotonic sequence
    /// that is carried through a parked continuation. A sink cannot mutate
    /// state; fail-closed is an explicit composition choice.
    #[allow(clippy::too_many_arguments)]
    fn observe(
        &mut self,
        ports: &ExecutionPorts,
        kind: apxm_runtime_protocol::ObservationKind,
        commitment: apxm_runtime_protocol::Commitment,
        node_execution_id: Option<&str>,
        occurrence_id: Option<&str>,
        attempt_id: Option<&str>,
        region_occurrence_id: Option<&str>,
        content_ref: Option<&str>,
        output_ref: Option<&str>,
        evidence_ref: Option<&str>,
        started_at: Option<Instant>,
    ) -> Result<(), ExecutionError> {
        let observation = self.stage_observation(
            ports,
            kind,
            commitment,
            node_execution_id,
            occurrence_id,
            attempt_id,
            region_occurrence_id,
            content_ref,
            output_ref,
            evidence_ref,
            started_at,
        )?;
        let Some(sink) = ports.observation_sink.as_ref() else {
            return Ok(());
        };
        match sink.publish(observation) {
            Ok(()) => Ok(()),
            Err(_error)
                if ports.observation_failure_policy == ObservationFailurePolicy::FailOpen =>
            {
                Ok(())
            }
            Err(error) => Err(ExecutionError::Observation(error)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_observation(
        &mut self,
        _ports: &ExecutionPorts,
        kind: apxm_runtime_protocol::ObservationKind,
        commitment: apxm_runtime_protocol::Commitment,
        node_execution_id: Option<&str>,
        occurrence_id: Option<&str>,
        attempt_id: Option<&str>,
        region_occurrence_id: Option<&str>,
        content_ref: Option<&str>,
        output_ref: Option<&str>,
        evidence_ref: Option<&str>,
        started_at: Option<Instant>,
    ) -> Result<apxm_runtime_protocol::ExecutionObservation, ExecutionError> {
        self.observation_seq = self.observation_seq.saturating_add(1);
        let duration_ms = started_at.map(|started| started.elapsed().as_millis() as u64);
        let event_ref = if matches!(
            kind,
            apxm_runtime_protocol::ObservationKind::EventWaiting
                | apxm_runtime_protocol::ObservationKind::EventResumed
        ) {
            self.pending_event_ref.take()
        } else {
            None
        };
        let host_capability = if matches!(
            kind,
            apxm_runtime_protocol::ObservationKind::CapabilityRequested
                | apxm_runtime_protocol::ObservationKind::CapabilitySettled
        ) {
            self.pending_host_capability.take()
        } else {
            None
        };
        let observation = make_observation(
            &self.program_invocation_id,
            self.observation_seq,
            apxm_runtime_protocol::ObservationTiming {
                observed_at_unix_ms: unix_time_ms(),
                duration_ms,
            },
            kind,
            commitment,
            node_execution_id,
            occurrence_id,
            attempt_id,
            region_occurrence_id,
            content_ref,
            output_ref,
            evidence_ref,
            event_ref.as_deref(),
            host_capability,
        );
        let observation = match observation {
            Ok(observation) => observation,
            Err(error) => return Err(ExecutionError::Observation(error)),
        };
        self.durable_observations.push(observation.clone());
        Ok(observation)
    }

    /// Record how one host-fulfilled Capability request settled, and append the
    /// evidence its outcome obliges.
    #[allow(clippy::too_many_arguments)]
    fn settle_host_capability(
        &mut self,
        ports: &ExecutionPorts,
        capability_ref: &str,
        settlement: &HostCapabilitySettlement,
        node_execution_id: &str,
        occurrence_id: &str,
        region_occurrence_id: Option<&str>,
        node_started_at: Instant,
    ) -> Result<(), ExecutionError> {
        self.pending_host_capability =
            Some(apxm_runtime_protocol::HostCapabilityObservation {
                capability_request_id: settlement.capability_request_id.clone(),
                capability_ref: capability_ref.to_owned(),
                input: None,
                authored_permission: None,
                outcome: Some(settlement.outcome),
                receipt_ref: settlement.receipt_ref.clone(),
            });
        self.observe(
            ports,
            apxm_runtime_protocol::ObservationKind::CapabilitySettled,
            apxm_runtime_protocol::Commitment::Provisional,
            Some(node_execution_id),
            Some(occurrence_id),
            None,
            region_occurrence_id,
            None,
            None,
            None,
            Some(node_started_at),
        )
    }

    /// Deliver observations after the atomic commit on a best-effort basis.
    /// The commit has already established execution truth, so a fail-closed
    /// live sink policy must not turn a successful commit into an error result.
    fn deliver_post_commit(
        ports: &ExecutionPorts,
        observation: &apxm_runtime_protocol::ExecutionObservation,
    ) {
        let Some(sink) = ports.observation_sink.as_ref() else {
            return;
        };
        let _ = sink.publish(observation.clone());
    }

    fn deliver_precommit(
        ports: &ExecutionPorts,
        observation: &apxm_runtime_protocol::ExecutionObservation,
    ) -> Result<(), ExecutionError> {
        let Some(sink) = ports.observation_sink.as_ref() else {
            return Ok(());
        };
        match sink.publish(observation.clone()) {
            Ok(()) => Ok(()),
            Err(_error)
                if ports.observation_failure_policy == ObservationFailurePolicy::FailOpen =>
            {
                Ok(())
            }
            Err(error) => Err(ExecutionError::Observation(error)),
        }
    }
}

fn node_outcome_terminal_status(outcome: &NodeOutcome) -> Option<RunTerminalStatus> {
    match outcome {
        NodeOutcome::Model { outcome, .. } => match outcome {
            ModelOutcome::CommittedSuccess { .. } => None,
            ModelOutcome::TypedFailure { .. } => Some(RunTerminalStatus::Failed),
            ModelOutcome::Cancelled => Some(RunTerminalStatus::Cancelled),
            ModelOutcome::ModelOutcomeUnknown { .. } => Some(RunTerminalStatus::OutcomeUnknown),
        },
        NodeOutcome::Capability { outcome, .. } => match outcome {
            CapabilityOutcome::Completed { .. } => None,
            CapabilityOutcome::Failed { .. } => Some(RunTerminalStatus::Failed),
            CapabilityOutcome::OutcomeUnknown { .. } => Some(RunTerminalStatus::OutcomeUnknown),
        },
        NodeOutcome::ProgramNew { outcome, .. } | NodeOutcome::ProgramInvoke { outcome, .. } => {
            matches!(outcome, CompositionOutcome::Failed { .. })
                .then_some(RunTerminalStatus::Failed)
        }
        NodeOutcome::AwaitEvent { outcome, .. } => {
            matches!(outcome, EventOutcome::Cancelled).then_some(RunTerminalStatus::Cancelled)
        }
        NodeOutcome::ExternalAgent { .. } => None,
    }
}

/// Canonical dynamic request identity at the native model boundary. Sensitive
/// Context content is represented only by its digest.
#[derive(Serialize)]
struct CanonicalModelRequestEnvelope<'a> {
    schema_version: &'static str,
    program_invocation_id: &'a str,
    node_execution_id: &'a str,
    operation: &'a SemanticOp,
    context_digest: String,
    authored_request: &'a Value,
}

fn model_effect_identity(program_invocation_id: &str, node_execution_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"apxm.model-effect\0");
    hasher.update(program_invocation_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(node_execution_id.as_bytes());
    format!("model-effect.{:x}", hasher.finalize())
}

fn model_request_digest(
    program_invocation_id: &str,
    node_execution_id: &str,
    operation: &SemanticOp,
    context_digest: &str,
    authored_request: &Value,
) -> String {
    let envelope = CanonicalModelRequestEnvelope {
        schema_version: "apxm.model-request-identity",
        program_invocation_id,
        node_execution_id,
        operation,
        context_digest: context_digest.to_string(),
        authored_request,
    };
    let request_bytes = serde_json::to_vec(&envelope)
        .expect("canonical model request envelope serializes deterministically");
    format!("sha256:{:x}", Sha256::digest(request_bytes))
}

fn model_context_digest(context: &Value) -> String {
    let context_bytes = serde_json::to_vec(context)
        .expect("canonical runtime Context serializes deterministically");
    format!("sha256:{:x}", Sha256::digest(context_bytes))
}

impl DriveState {
    /// A fresh run: emit the instance-created and invocation-admitted lifecycle
    /// facts (sequences 1 and 2), exactly as the single-shot path always has.
    fn new(
        initial_context: Value,
        initial_values: BTreeMap<String, Value>,
        _air: &AirModule,
        program_invocation_id: &str,
    ) -> Self {
        let mut batch = Vec::new();
        let mut seq = 0u64;
        seq += 1;
        batch.push(fact(
            program_invocation_id,
            seq,
            FactKind::InstanceCreated,
            Some(InstanceState::Ready),
            None,
            None,
            None,
        ));
        seq += 1;
        batch.push(fact(
            program_invocation_id,
            seq,
            FactKind::InvocationAdmitted,
            None,
            Some(InvocationState::Running),
            None,
            None,
        ));
        Self {
            node_outcomes: Vec::new(),
            native_usage: Usage::default(),
            committed_model_attempts: Vec::new(),
            committed_model_lineages: Vec::new(),
            external_agent_evidence: Vec::new(),
            context: initial_context,
            last_result: Value::Null,
            values: initial_values,
            last_result_value_id: None,
            hook_target_snapshots: BTreeMap::new(),
            branch_decisions: BTreeMap::new(),
            last_operation_succeeded: true,
            batch,
            durable_observations: Vec::new(),
            pending_event_ref: None,
            pending_host_capability: None,
            observation_seq: 0,
            seq,
            program_invocation_id: program_invocation_id.to_string(),
            active_loops: Vec::new(),
            last_model_node_execution_id: None,
            last_program_new_node_execution_id: None,
            committed_output_node_execution_id: None,
            committed_output_occurrence_id: None,
            committed_output_region_occurrence_id: None,
            current_node_execution_id: None,
            current_occurrence_id: None,
            current_region_occurrence_id: None,
            terminal_node_execution_id: None,
            terminal_occurrence_id: None,
            terminal_region_occurrence_id: None,
        }
    }

    fn enter_loop(&mut self, static_loop_id: &str) {
        let occurrence = format!(
            "loop-occurrence.{static_loop_id}.{}",
            self.program_invocation_id
        );
        if self
            .active_loops
            .iter()
            .any(|frame| frame.static_loop_id == static_loop_id)
        {
            return;
        }
        self.seq += 1;
        self.batch.push(join_fact(JoinFactSpec {
            program_invocation_id: &self.program_invocation_id,
            seq: self.seq,
            kind: FactKind::RegionOccurrenceStarted,
            region_occurrence_id: Some(occurrence.clone()),
            static_region_id: Some(static_loop_id.to_string()),
            node_execution_id: None,
            air_node_id: None,
            parent_node_execution_id: None,
        }));
        self.active_loops.push(DurableLoopFrame {
            static_loop_id: static_loop_id.to_string(),
            dynamic_occurrence_id: occurrence,
            iteration_index: 0,
            failed: false,
            parked: false,
            causal_node_execution_ids: Vec::new(),
        });
    }

    fn record_node_outcome(
        &mut self,
        loop_path: &[String],
        node_execution_id: &str,
        succeeded: bool,
    ) {
        for static_loop_id in loop_path {
            if let Some(active) = self
                .active_loops
                .iter_mut()
                .find(|frame| frame.static_loop_id == *static_loop_id)
            {
                if succeeded {
                    active
                        .causal_node_execution_ids
                        .push(node_execution_id.to_string());
                } else {
                    active.failed = true;
                }
            }
        }
    }

    fn park_active_loops(&mut self) {
        for active in &mut self.active_loops {
            active.parked = true;
        }
    }

    fn fail_active_loops(&mut self) {
        for active in &mut self.active_loops {
            active.failed = true;
        }
    }

    fn exit_loop(&mut self, static_loop_id: &str) {
        self.active_loops
            .retain(|frame| frame.static_loop_id != static_loop_id);
    }

    fn complete_loop_iteration(&mut self, static_loop_id: &str) -> bool {
        let Some(position) = self
            .active_loops
            .iter()
            .position(|frame| frame.static_loop_id == static_loop_id)
        else {
            return false;
        };
        if self.active_loops[position].failed
            || self.active_loops[position]
                .causal_node_execution_ids
                .is_empty()
        {
            self.active_loops.remove(position);
            return false;
        }
        let active = &mut self.active_loops[position];
        let loop_occurrence_id = active.dynamic_occurrence_id.clone();
        let iteration_index = active.iteration_index;
        let causal_node_execution_ids = std::mem::take(&mut active.causal_node_execution_ids);
        active.iteration_index += 1;
        self.seq += 1;
        let completed = Fact::LoopIterationCompleted(LoopIterationCompletedFact::new(
            format!("loop-iteration.{loop_occurrence_id}.{iteration_index}"),
            self.seq,
            static_loop_id.to_string(),
            loop_occurrence_id,
            iteration_index,
            self.program_invocation_id.clone(),
            causal_node_execution_ids,
        ));
        self.batch.push(completed);
        true
    }
}

fn enforce_runtime_limits(
    state: &DriveState,
    resource_ceilings: Option<&ResourceCeilings>,
) -> Result<(), ExecutionError> {
    let Some(ceilings) = resource_ceilings else {
        return Ok(());
    };
    let memory_bytes = serde_json::to_vec(&(
        &state.context,
        &state.last_result,
        &state.values,
        &state.active_loops,
        &state.batch,
    ))
    .map_err(|error| ExecutionError::InvalidAir {
        message: format!("runtime memory accounting failed: {error}"),
    })?
    .len() as u64;
    let outcome_values = state
        .node_outcomes
        .iter()
        .map(node_outcome_value)
        .collect::<Vec<_>>();
    let effect_bytes = serde_json::to_vec(&(
        &state.batch,
        &outcome_values,
        &state.last_result,
        &state.values,
    ))
    .map_err(|error| ExecutionError::InvalidAir {
        message: format!("runtime effect accounting failed: {error}"),
    })?
    .len() as u64;
    for (resource, observed, limit) in [
        ("memory_bytes", memory_bytes, ceilings.max_memory_bytes),
        ("effect_bytes", effect_bytes, ceilings.max_effect_bytes),
    ] {
        if limit == 0 || observed > limit {
            return Err(ExecutionError::ResourceLimitExceeded {
                resource,
                limit,
                observed,
            });
        }
    }
    Ok(())
}

struct JoinFactSpec<'a> {
    program_invocation_id: &'a str,
    seq: u64,
    kind: FactKind,
    region_occurrence_id: Option<String>,
    static_region_id: Option<String>,
    node_execution_id: Option<String>,
    air_node_id: Option<String>,
    parent_node_execution_id: Option<String>,
}

fn join_fact(spec: JoinFactSpec<'_>) -> Fact {
    let mut fact = fact(
        spec.program_invocation_id,
        spec.seq,
        spec.kind,
        None,
        None,
        None,
        None,
    );
    let runtime = runtime_fact_mut(&mut fact);
    runtime.region_occurrence_id = spec.region_occurrence_id;
    runtime.static_region_id = spec.static_region_id;
    runtime.node_execution_id = spec.node_execution_id;
    runtime.air_node_id = spec.air_node_id;
    runtime.parent_node_execution_id = spec.parent_node_execution_id;
    fact
}

fn context_ref(id: String) -> TypedRef {
    TypedRef {
        ref_type: "ProgramContextEvidenceRef".to_string(),
        target: id,
        digest: None,
    }
}

fn materialize_ssa_value(
    air: &AirModule,
    state: &DriveState,
    region_id: &str,
    value_id: &str,
    visiting: &mut BTreeSet<String>,
) -> Result<Value, ExecutionError> {
    if let Some(value) = state.values.get(value_id) {
        return Ok(value.clone());
    }
    if !visiting.insert(value_id.to_string()) {
        return Err(ExecutionError::InvalidControlPredicate {
            region_id: region_id.to_string(),
            message: format!("value assembly cycle at {value_id}"),
        });
    }
    let assembly = air
        .value_assemblies
        .iter()
        .find(|assembly| assembly.value_id == value_id)
        .ok_or_else(|| ExecutionError::MissingControlValue {
            region_id: region_id.to_string(),
            value_id: value_id.to_string(),
        })?;
    let value = evaluate_value_expression(
        air,
        state,
        region_id,
        value_id,
        &assembly.expression,
        visiting,
        0,
    )?;
    visiting.remove(value_id);
    Ok(value)
}

fn evaluate_value_expression(
    air: &AirModule,
    state: &DriveState,
    region_id: &str,
    owner: &str,
    expression: &ValueExpression,
    visiting: &mut BTreeSet<String>,
    depth: usize,
) -> Result<Value, ExecutionError> {
    if depth > MAX_EXPRESSION_DEPTH {
        return Err(ExecutionError::InvalidValueExpression {
            value_id: owner.to_string(),
            message: format!("expression depth exceeds {MAX_EXPRESSION_DEPTH}"),
        });
    }
    let project = |mut value: Value, path: &[String]| -> Result<Value, ExecutionError> {
        for segment in path {
            value = value.get(segment).cloned().ok_or_else(|| {
                ExecutionError::InvalidValueExpression {
                    value_id: owner.to_string(),
                    message: format!("property path segment '{segment}' is absent"),
                }
            })?;
        }
        Ok(value)
    };
    match expression {
        ValueExpression::Ssa { value_id } => {
            materialize_ssa_value(air, state, region_id, value_id, visiting)
        }
        ValueExpression::Context { property_path } => project(state.context.clone(), property_path),
        ValueExpression::Projection {
            root,
            property_path,
        } => project(
            evaluate_value_expression(air, state, region_id, owner, root, visiting, depth + 1)?,
            property_path,
        ),
        ValueExpression::Object { fields } => {
            let mut object = serde_json::Map::new();
            for field in fields {
                if object.contains_key(&field.name) {
                    return Err(ExecutionError::InvalidValueExpression {
                        value_id: owner.to_string(),
                        message: format!("duplicate object field '{}'", field.name),
                    });
                }
                object.insert(
                    field.name.clone(),
                    evaluate_value_expression(
                        air,
                        state,
                        region_id,
                        owner,
                        &field.value,
                        visiting,
                        depth + 1,
                    )?,
                );
            }
            Ok(Value::Object(object))
        }
        ValueExpression::Array { items } => items
            .iter()
            .map(|item| {
                evaluate_value_expression(air, state, region_id, owner, item, visiting, depth + 1)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        ValueExpression::String { value } => Ok(Value::String(value.clone())),
        ValueExpression::Integer { value } => Ok(Value::Number((*value).into())),
        ValueExpression::Boolean { value } => Ok(Value::Bool(*value)),
        ValueExpression::Null => Ok(Value::Null),
    }
}

fn evaluate_predicate(
    air: &AirModule,
    state: &DriveState,
    region_id: &str,
    predicate: &ControlPredicate,
) -> Result<bool, ExecutionError> {
    // Predicate roots may be permitted pure value assemblies rather than
    // state entries. Materialize them at the control boundary so runtime
    // evaluation observes the same SSA value shape that invocation operands
    // receive.
    let materialized = materialize_ssa_value(
        air,
        state,
        region_id,
        &predicate.root_value_id,
        &mut BTreeSet::new(),
    )?;
    let mut value = &materialized;
    for segment in &predicate.property_path {
        value = value
            .get(segment)
            .ok_or_else(|| ExecutionError::InvalidControlPredicate {
                region_id: region_id.to_string(),
                message: format!("property path segment '{segment}' is absent"),
            })?;
    }
    match predicate.comparator {
        PredicateComparator::Truthy => {
            value
                .as_bool()
                .ok_or_else(|| ExecutionError::InvalidControlPredicate {
                    region_id: region_id.to_string(),
                    message: "truthy comparator requires a boolean value".to_string(),
                })
        }
        PredicateComparator::Equals | PredicateComparator::NotEquals => {
            let literal = predicate.literal.as_ref().ok_or_else(|| {
                ExecutionError::InvalidControlPredicate {
                    region_id: region_id.to_string(),
                    message: "equals comparator is missing its typed scalar literal".to_string(),
                }
            })?;
            let expected = match literal {
                PredicateLiteral::Boolean(value) => Value::Bool(*value),
                PredicateLiteral::String(value) => Value::String(value.clone()),
                PredicateLiteral::Integer(value) => Value::Number((*value).into()),
                PredicateLiteral::Null => Value::Null,
            };
            if !value.is_null()
                && !value.is_boolean()
                && !value.is_string()
                && !value.is_i64()
                && !value.is_u64()
            {
                return Err(ExecutionError::InvalidControlPredicate {
                    region_id: region_id.to_string(),
                    message: "equals comparator requires a scalar runtime value".to_string(),
                });
            }
            Ok(if predicate.comparator == PredicateComparator::Equals {
                *value == expected
            } else {
                *value != expected
            })
        }
    }
}

fn matching_loop_back_edge(
    schedule: &[ScheduleStep],
    start: usize,
    static_loop_id: &str,
) -> Result<usize, ExecutionError> {
    schedule
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, step)| match step {
            ScheduleStep::LoopBackEdge {
                static_loop_id: candidate,
            } if candidate == static_loop_id => Some(index),
            _ => None,
        })
        .ok_or_else(|| {
            ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
                message: format!("loop {static_loop_id} has no schedule back edge"),
            })
        })
}

fn loop_value_pairs<'a>(
    region: &'a apxm_program::air::StructuralNode,
    slot: &str,
) -> Result<
    Vec<(
        &'a apxm_program::air::SsaValue,
        &'a apxm_program::air::Operand,
    )>,
    ExecutionError,
> {
    let operands: Vec<_> = region
        .operands
        .iter()
        .filter(|operand| operand.slot == slot)
        .collect();
    if operands.len() != region.block_arguments.len()
        || region
            .block_arguments
            .iter()
            .zip(&operands)
            .any(|(argument, operand)| argument.type_ref != operand.type_ref)
    {
        return Err(ExecutionError::InvalidControlPredicate {
            region_id: region.region_id.clone(),
            message: format!("loop has invalid {slot} carried-value signature"),
        });
    }
    Ok(region.block_arguments.iter().zip(operands).collect())
}

fn matching_branch_arm_end(
    schedule: &[ScheduleStep],
    start: usize,
    static_branch_id: &str,
    arm_index: usize,
) -> Result<usize, ExecutionError> {
    schedule
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, step)| match step {
            ScheduleStep::BranchArmEnd {
                static_branch_id: candidate,
                arm_index: candidate_index,
            } if candidate == static_branch_id && *candidate_index == arm_index => Some(index),
            _ => None,
        })
        .ok_or_else(|| {
            ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
                message: format!("branch {static_branch_id} arm {arm_index} has no schedule end"),
            })
        })
}

/// The result of driving the op loop from a start index: either it reached the
/// end, or it parked at an `await.event` (only when `suspend_on_park`).
enum DriveEnd {
    RanToEnd(DriveState),
    Parked {
        state: DriveState,
        continuation_id: String,
        event_ref: Option<EventRef>,
        next_schedule_position: usize,
        parked_node_execution_id: Option<String>,
        parked_loop_path: Vec<String>,
        resume_value_id: Option<String>,
    },
}

#[derive(Clone, Copy)]
struct DriveOptions {
    suspend_on_park: bool,
    yield_at_loop: bool,
}

struct DriveInputs<'a> {
    ports: &'a ExecutionPorts,
    air: &'a AirModule,
    hook_bindings: &'a [HookBinding],
    model_admission: &'a ModelBindingAdmission,
    capability_invocations: &'a BTreeMap<String, CapabilityInvocationAdmission>,
    resource_ceilings: Option<&'a ResourceCeilings>,
}

/// Walk the structural execution schedule from `start_index`, dispatching semantic
/// operations to their exact injected ports and threading Context through Hooks.
/// When `suspend_on_park` is set, a parked `await.event` or loop yield stops the
/// walk and returns [`DriveEnd::Parked`]; otherwise a parked outcome is recorded
/// and the walk continues (the single-shot contract).
async fn drive_from(
    inputs: DriveInputs<'_>,
    start_schedule_position: usize,
    mut state: DriveState,
    options: DriveOptions,
) -> Result<DriveEnd, ExecutionError> {
    let DriveInputs {
        ports,
        air,
        hook_bindings,
        model_admission,
        capability_invocations,
        resource_ceilings,
    } = inputs;
    let schedule = build_schedule(air, hook_bindings);
    if schedule.len() > MAX_SCHEDULE_STEPS {
        return Err(ExecutionError::InvalidAir {
            message: format!(
                "execution schedule has {} steps, limit is {MAX_SCHEDULE_STEPS}",
                schedule.len()
            ),
        });
    }
    enforce_runtime_limits(&state, resource_ceilings)?;
    if start_schedule_position > schedule.len() {
        return Err(ExecutionError::Continuation(
            ContinuationError::InvalidCommittedState {
                message: format!(
                    "schedule position {start_schedule_position} exceeds {}",
                    schedule.len()
                ),
            },
        ));
    }

    let mut schedule_position = start_schedule_position;
    while schedule_position < schedule.len() {
        if ports.cancellation.is_cancelled() {
            if !state.has_terminal_non_success() {
                state.observe(
                    ports,
                    apxm_runtime_protocol::ObservationKind::InvocationCancelled,
                    apxm_runtime_protocol::Commitment::Provisional,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )?;
                state.append_invocation_cancelled(None);
            }
            return Ok(DriveEnd::RanToEnd(state));
        }
        enforce_runtime_limits(&state, resource_ceilings)?;
        let step = &schedule[schedule_position];
        match step {
            ScheduleStep::HookBodyBegin { binding } => {
                // Capture the target state before any operation in the Hook's
                // own region runs. In particular, an after-Hook must not see
                // the result or success flag of its captured body.
                state.hook_target_snapshots.insert(
                    binding.body_region_id.clone(),
                    HookTargetSnapshot {
                        result: state.last_result.clone(),
                        succeeded: state.last_operation_succeeded,
                        result_value_id: state.last_result_value_id.clone(),
                    },
                );
            }
            ScheduleStep::HookBefore { binding } => {
                let (before, after) = apply_static_hook(&mut state, ports, air, binding).await?;
                state.seq += 1;
                let mut fact = fact(
                    &state.program_invocation_id,
                    state.seq,
                    FactKind::HookExecuted,
                    None,
                    None,
                    None,
                    None,
                );
                let runtime = runtime_fact_mut(&mut fact);
                runtime.hook_execution_id =
                    Some(format!("hook-execution.{}.{}", binding.hook_id, state.seq));
                runtime.hook_id = Some(binding.hook_id.clone());
                runtime.hook_scope = Some(evidence_hook_scope(binding.scope));
                runtime.hook_phase = Some(EvidenceHookPhase::Before);
                runtime.context_before_ref =
                    Some(context_ref(format!("context.{}.before", state.seq)));
                runtime.context_after_ref =
                    Some(context_ref(format!("context.{}.after", state.seq)));
                state.batch.push(fact);
                if before != after {
                    state.seq += 1;
                    state.batch.push(context_transition_fact(
                        &state.program_invocation_id,
                        state.seq,
                    ));
                }
            }
            ScheduleStep::HookAfter { binding } => {
                let target_succeeded = state
                    .hook_target_snapshots
                    .get(&binding.body_region_id)
                    .map_or(state.last_operation_succeeded, |snapshot| {
                        snapshot.succeeded
                    });
                if target_succeeded {
                    let (before, after) =
                        apply_static_hook(&mut state, ports, air, binding).await?;
                    state.seq += 1;
                    let mut fact = fact(
                        &state.program_invocation_id,
                        state.seq,
                        FactKind::HookExecuted,
                        None,
                        None,
                        None,
                        None,
                    );
                    let runtime = runtime_fact_mut(&mut fact);
                    runtime.hook_execution_id =
                        Some(format!("hook-execution.{}.{}", binding.hook_id, state.seq));
                    runtime.hook_id = Some(binding.hook_id.clone());
                    runtime.hook_scope = Some(evidence_hook_scope(binding.scope));
                    runtime.hook_phase = Some(EvidenceHookPhase::After);
                    runtime.context_before_ref =
                        Some(context_ref(format!("context.{}.before", state.seq)));
                    runtime.context_after_ref =
                        Some(context_ref(format!("context.{}.after", state.seq)));
                    state.batch.push(fact);
                    if before != after {
                        state.seq += 1;
                        state.batch.push(context_transition_fact(
                            &state.program_invocation_id,
                            state.seq,
                        ));
                    }
                } else if let Some(snapshot) =
                    state.hook_target_snapshots.get(&binding.body_region_id)
                {
                    // Preserve the target status for any outer after-Hooks;
                    // the captured body is not the target operation.
                    state.last_result = snapshot.result.clone();
                    state
                        .last_result_value_id
                        .clone_from(&snapshot.result_value_id);
                    state.last_operation_succeeded = snapshot.succeeded;
                }
            }
            ScheduleStep::ContextEdge {
                from_node,
                to_node,
                value_id,
            } => {
                state.context =
                    materialize_ssa_value(air, &state, to_node, value_id, &mut BTreeSet::new())?;
                state.seq += 1;
                let mut fact = context_transition_fact(&state.program_invocation_id, state.seq);
                let runtime = runtime_fact_mut(&mut fact);
                runtime.air_node_id = Some(to_node.clone());
                runtime.parent_node_execution_id = Some(from_node.clone());
                state.batch.push(fact);
            }
            ScheduleStep::EnterLoop {
                static_loop_id,
                predicate,
            } => {
                let region = air
                    .structural_ir
                    .iter()
                    .find(|region| region.region_id == *static_loop_id)
                    .expect("schedule loop references its AIR structural node");
                let already_active = state
                    .active_loops
                    .iter()
                    .any(|frame| frame.static_loop_id == *static_loop_id);
                let initial_pairs = loop_value_pairs(region, "initial")?;
                let _ = loop_value_pairs(region, "carried")?;
                if !already_active {
                    for (argument, initial) in initial_pairs {
                        let value =
                            state
                                .values
                                .get(&initial.value_id)
                                .cloned()
                                .ok_or_else(|| ExecutionError::MissingControlValue {
                                    region_id: static_loop_id.clone(),
                                    value_id: initial.value_id.clone(),
                                })?;
                        state.values.insert(argument.value_id.clone(), value);
                    }
                }
                if let Some(predicate) = predicate
                    && !evaluate_predicate(air, &state, static_loop_id, predicate)?
                {
                    state.observe(
                        ports,
                        apxm_runtime_protocol::ObservationKind::LoopTransition,
                        apxm_runtime_protocol::Commitment::Provisional,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    )?;
                    state.exit_loop(static_loop_id);
                    schedule_position =
                        matching_loop_back_edge(&schedule, schedule_position, static_loop_id)? + 1;
                    continue;
                }
                state.enter_loop(static_loop_id);
                let region_occurrence_id = state
                    .active_loops
                    .iter()
                    .find(|frame| frame.static_loop_id == *static_loop_id)
                    .map(|frame| frame.dynamic_occurrence_id.clone());
                state.observe(
                    ports,
                    apxm_runtime_protocol::ObservationKind::LoopTransition,
                    apxm_runtime_protocol::Commitment::Provisional,
                    None,
                    None,
                    None,
                    region_occurrence_id.as_deref(),
                    None,
                    None,
                    None,
                    None,
                )?;
            }
            ScheduleStep::BranchDecision {
                static_branch_id,
                predicate,
            } => {
                let predicate =
                    predicate
                        .as_ref()
                        .ok_or_else(|| ExecutionError::InvalidControlPredicate {
                            region_id: static_branch_id.clone(),
                            message: "branch is missing its typed predicate".into(),
                        })?;
                let selected = usize::from(!evaluate_predicate(
                    air,
                    &state,
                    static_branch_id,
                    predicate,
                )?);
                state
                    .branch_decisions
                    .insert(static_branch_id.clone(), selected);
                // A static branch id is not a dynamic occurrence. Bind the
                // selected arm to this invocation and active loop region so
                // repeated/nested branch joins cannot alias one another.
                let region_occurrence_id = state
                    .active_loops
                    .last()
                    .map(|frame| frame.dynamic_occurrence_id.clone());
                let iteration_occurrence_id = state.active_loops.last().map(|frame| {
                    format!(
                        "{}.iteration.{}",
                        frame.dynamic_occurrence_id, frame.iteration_index
                    )
                });
                let occurrence_id = format!(
                    "branch-occurrence.{}.{}.{}.{}",
                    state.program_invocation_id,
                    iteration_occurrence_id.as_deref().unwrap_or("root"),
                    static_branch_id,
                    selected
                );
                state.observe(
                    ports,
                    apxm_runtime_protocol::ObservationKind::BranchTransition,
                    apxm_runtime_protocol::Commitment::Provisional,
                    None,
                    Some(&occurrence_id),
                    None,
                    region_occurrence_id.as_deref(),
                    None,
                    None,
                    None,
                    None,
                )?;
            }
            ScheduleStep::BranchArm {
                static_branch_id,
                arm_index,
            } => {
                let selected = state
                    .branch_decisions
                    .get(static_branch_id)
                    .ok_or_else(|| {
                        ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
                            message: format!(
                                "branch {static_branch_id} arm encountered without a decision"
                            ),
                        })
                    })?;
                if selected != arm_index {
                    schedule_position = matching_branch_arm_end(
                        &schedule,
                        schedule_position,
                        static_branch_id,
                        *arm_index,
                    )? + 1;
                    continue;
                }
            }
            ScheduleStep::BranchArmEnd { .. } => {}
            ScheduleStep::BranchEnd { static_branch_id } => {
                let region_occurrence_id = state
                    .active_loops
                    .last()
                    .map(|frame| frame.dynamic_occurrence_id.clone());
                let iteration_occurrence_id = state.active_loops.last().map(|frame| {
                    format!(
                        "{}.iteration.{}",
                        frame.dynamic_occurrence_id, frame.iteration_index
                    )
                });
                let occurrence_id = state
                    .branch_decisions
                    .get(static_branch_id)
                    .map(|selected| {
                        format!(
                            "branch-occurrence.{}.{}.{}.{}",
                            state.program_invocation_id,
                            iteration_occurrence_id.as_deref().unwrap_or("root"),
                            static_branch_id,
                            selected
                        )
                    });
                state.observe(
                    ports,
                    apxm_runtime_protocol::ObservationKind::JoinTransition,
                    apxm_runtime_protocol::Commitment::Provisional,
                    None,
                    occurrence_id.as_deref(),
                    None,
                    region_occurrence_id.as_deref(),
                    None,
                    None,
                    None,
                    None,
                )?;
                state.branch_decisions.remove(static_branch_id);
            }
            ScheduleStep::Semantic { index, loop_path } => {
                let op = &air.semantic_operations[*index];
                state.seq += 1;
                let node_execution_id = format!(
                    "node-execution.{}.{}.{}",
                    state.program_invocation_id, op.node_id, state.seq
                );
                let innermost_loop = loop_path.last().and_then(|loop_id| {
                    state
                        .active_loops
                        .iter()
                        .find(|frame| frame.static_loop_id == *loop_id)
                });
                let loop_memberships = loop_path
                    .iter()
                    .filter_map(|loop_id| {
                        state
                            .active_loops
                            .iter()
                            .find(|frame| frame.static_loop_id == *loop_id)
                            .map(|active| LoopMembership {
                                static_loop_id: loop_id.clone(),
                                loop_occurrence_id: active.dynamic_occurrence_id.clone(),
                            })
                    })
                    .collect();
                let execution_scope = match (loop_path.last(), innermost_loop) {
                    (Some(static_region_id), Some(active)) => NodeExecutionScope::Loop {
                        region_occurrence_id: active.dynamic_occurrence_id.clone(),
                        static_region_id: static_region_id.clone(),
                        loop_memberships,
                    },
                    (None, None) => NodeExecutionScope::NonLoop,
                    _ => unreachable!("typed schedule loop path and active frames agree"),
                };
                let region_occurrence_id =
                    innermost_loop.map(|active| active.dynamic_occurrence_id.clone());
                let occurrence_id = format!("occurrence.{node_execution_id}");
                let node_started_at = Instant::now();
                state.current_node_execution_id = Some(node_execution_id.clone());
                state.current_occurrence_id = Some(occurrence_id.clone());
                state
                    .current_region_occurrence_id
                    .clone_from(&region_occurrence_id);
                state.observe(
                    ports,
                    apxm_runtime_protocol::ObservationKind::NodeStarted,
                    apxm_runtime_protocol::Commitment::Provisional,
                    Some(&node_execution_id),
                    Some(&occurrence_id),
                    None,
                    region_occurrence_id.as_deref(),
                    None,
                    None,
                    None,
                    None,
                )?;
                let node_fact = Fact::NodeExecutionRecorded(NodeExecutionRecordedFact {
                    fact_id: format!("fact.{}.{}", state.program_invocation_id, state.seq),
                    event_sequence: state.seq,
                    program_invocation_id: state.program_invocation_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                    air_node_id: op.node_id.clone(),
                    parent_node_execution_id: match op.op {
                        SemanticOpKind::CapabilityInvoke => {
                            state.last_model_node_execution_id.clone()
                        }
                        SemanticOpKind::ProgramInvoke => {
                            state.last_program_new_node_execution_id.clone()
                        }
                        _ => None,
                    },
                    execution_scope,
                });
                state.batch.push(node_fact);
                // Final output provenance is one atomic state value. A later
                // node, including a failed/unknown effect, must not inherit
                // the previous node's readable bytes or coordinates.
                state.committed_output_node_execution_id = None;
                state.committed_output_occurrence_id = None;
                state.committed_output_region_occurrence_id = None;

                match op.op {
                    SemanticOpKind::ModelCall => {
                        let target = operand_str(op, "model_ref").ok_or_else(|| {
                            ExecutionError::MissingOperand {
                                node_id: op.node_id.clone(),
                                operand: "model_ref",
                            }
                        })?;
                        let authored_target = ModelTargetRef(target);
                        let request_value_id = operand_str(op, "request").ok_or_else(|| {
                            ExecutionError::MissingOperand {
                                node_id: op.node_id.clone(),
                                operand: "request",
                            }
                        })?;
                        let authored_request = materialize_ssa_value(
                            air,
                            &state,
                            &op.node_id,
                            &request_value_id,
                            &mut BTreeSet::new(),
                        )?;
                        let effect_id =
                            model_effect_identity(&state.program_invocation_id, &node_execution_id);
                        let context_digest = model_context_digest(&state.context);
                        let request_digest = model_request_digest(
                            &state.program_invocation_id,
                            &node_execution_id,
                            op,
                            &context_digest,
                            &authored_request,
                        );
                        let preparation = ModelCallPreparation::authorize(
                            effect_id,
                            node_execution_id.clone(),
                            request_digest,
                            context_digest,
                            authored_request,
                            &authored_target,
                            model_admission,
                        )
                        .map_err(ExecutionError::Binding)?;
                        let metadata = ports
                            .model_call_request_metadata
                            .materialize(&preparation)
                            .map_err(ExecutionError::ModelRequestMetadata)?;
                        let call = ModelCallRequest::prepare(preparation, metadata)
                            .map_err(ExecutionError::ModelRequest)?;
                        let dispatch_started = Instant::now();
                        let target_commitment =
                            InferenceTargetCommitment::from_resolved(call.resolved_binding())
                                .map_err(ExecutionError::TargetCommitment)?;
                        let dispatch = tokio::select! {
                            result = dispatch_committed_inference_async(CommittedInferenceDispatch {
                                target_commitment: &target_commitment,
                                authored_target: &authored_target,
                                request: &call,
                                backend: &*ports.model_inference,
                                duration_ms: dispatch_started.elapsed().as_millis() as u64,
                                policy: RetryPolicy::default(),
                            }) => Some(result),
                            () = ports.cancellation.cancelled() => None,
                        };
                        let Some(dispatch) = dispatch else {
                            // The effect future was cancelled before it
                            // reported an outcome. Commit uncertainty, never
                            // a guessed return or a fabricated attempt.
                            state.observe(
                                ports,
                                apxm_runtime_protocol::ObservationKind::OutcomeUnknown,
                                apxm_runtime_protocol::Commitment::Provisional,
                                Some(&node_execution_id),
                                Some(&occurrence_id),
                                None,
                                region_occurrence_id.as_deref(),
                                None,
                                None,
                                None,
                                Some(node_started_at),
                            )?;
                            state.append_effect_outcome_unknown(
                                &node_execution_id,
                                call.effect_id(),
                            );
                            state.node_outcomes.push(NodeOutcome::Model {
                                node_id: op.node_id.clone(),
                                outcome: ModelOutcome::ModelOutcomeUnknown {
                                    uncertain_usage: None,
                                },
                                result: Value::Null,
                                replaced: false,
                            });
                            state.last_operation_succeeded = false;
                            state.last_result = Value::Null;
                            state.last_model_node_execution_id = Some(node_execution_id.clone());
                            return Ok(DriveEnd::RanToEnd(state));
                        };
                        let committed_dispatch = match dispatch {
                            Ok(dispatch) => dispatch,
                            Err(error) => {
                                // A dispatch rejection is an invocation
                                // failure, not a reason to drop the evidence
                                // batch. The exact error is retained in the
                                // typed envelope and no output is staged.
                                let typed_error = TypedError {
                                    category: apxm_inference::ErrorCategory::Unavailable,
                                    code: "model_dispatch_failed".to_owned(),
                                    message: error.to_string(),
                                };
                                state.append_invocation_failure(
                                    &node_execution_id,
                                    model_failure_envelope(&node_execution_id, &typed_error),
                                );
                                state.node_outcomes.push(NodeOutcome::Model {
                                    node_id: op.node_id.clone(),
                                    outcome: ModelOutcome::TypedFailure { error: typed_error },
                                    result: Value::Null,
                                    replaced: false,
                                });
                                state.last_operation_succeeded = false;
                                state.last_result = Value::Null;
                                state.last_model_node_execution_id =
                                    Some(node_execution_id.clone());
                                return Ok(DriveEnd::RanToEnd(state));
                            }
                        };
                        let execution = committed_dispatch.execution;
                        let outcome = execution.outcome;
                        state.last_operation_succeeded =
                            matches!(&outcome, ModelOutcome::CommittedSuccess { .. });
                        let result = match (state.last_operation_succeeded, execution.output) {
                            (true, Some(output)) => output,
                            (true, None) => {
                                return Err(ExecutionError::MissingModelOutput {
                                    node_id: op.node_id.clone(),
                                });
                            }
                            (false, _) => Value::Null,
                        };
                        if let (ModelOutcome::CommittedSuccess { usage }, Some(attempt_index)) =
                            (&outcome, execution.committed_attempt)
                        {
                            state.native_usage.input_tokens += usage.input_tokens;
                            state.native_usage.output_tokens += usage.output_tokens;
                            state.seq += 1;
                            let attempt = ModelAttemptRecordedFact {
                                fact_id: format!(
                                    "fact.{}.{}",
                                    state.program_invocation_id, state.seq
                                ),
                                event_sequence: state.seq,
                                program_invocation_id: state.program_invocation_id.clone(),
                                node_execution_id: node_execution_id.clone(),
                                air_node_id: op.node_id.clone(),
                                attempt_id: format!(
                                    "model-attempt.{node_execution_id}.{attempt_index}"
                                ),
                                attempt_index,
                                model_effect_id: call.effect_id().to_string(),
                                request_digest: call.request_digest().to_string(),
                                model_target_ref: call
                                    .resolved_binding()
                                    .model_target
                                    .reference
                                    .0
                                    .clone(),
                                model_target_digest: target_commitment.target_digest.clone(),
                                model_deployment_ref: call
                                    .resolved_binding()
                                    .model_deployment_ref
                                    .0
                                    .clone(),
                                exact_port_binding_digest: call
                                    .resolved_binding()
                                    .exact_port_binding
                                    .binding_digest
                                    .clone(),
                                target_commitment_digest: target_commitment.commit_digest,
                                generation_cohort_digest: target_commitment
                                    .generation_cohort_digest,
                                target_generation: target_commitment.target_generation,
                                target_port_contract_digest: target_commitment.port_contract_digest,
                                target_composition_digest: target_commitment.composition_digest,
                                native_input_tokens: usage.input_tokens,
                                native_output_tokens: usage.output_tokens,
                            };
                            state.batch.push(Fact::AttemptRecorded(attempt.clone()));
                            state.committed_model_attempts.push(attempt);
                            state
                                .committed_model_lineages
                                .push(committed_dispatch.lineage);
                        }
                        // The inference dispatch owns retry numbering. Only
                        // attach an attempt identity when it reports the
                        // actual committed attempt; an unknown/cancelled
                        // dispatch must not be fabricated as attempt `.0`.
                        let attempt_id = execution.committed_attempt.map(|attempt_index| {
                            format!("model-attempt.{node_execution_id}.{attempt_index}")
                        });
                        state.observe(
                            ports,
                            apxm_runtime_protocol::ObservationKind::ModelAttempt,
                            apxm_runtime_protocol::Commitment::Provisional,
                            Some(&node_execution_id),
                            Some(&occurrence_id),
                            attempt_id.as_deref(),
                            region_occurrence_id.as_deref(),
                            None,
                            None,
                            None,
                            Some(node_started_at),
                        )?;
                        match &outcome {
                            ModelOutcome::CommittedSuccess { .. } => {}
                            ModelOutcome::Cancelled => {
                                state.observe(
                                    ports,
                                    apxm_runtime_protocol::ObservationKind::InvocationCancelled,
                                    apxm_runtime_protocol::Commitment::Provisional,
                                    Some(&node_execution_id),
                                    Some(&occurrence_id),
                                    attempt_id.as_deref(),
                                    region_occurrence_id.as_deref(),
                                    None,
                                    None,
                                    None,
                                    Some(node_started_at),
                                )?;
                                state.append_invocation_cancelled(Some(&node_execution_id));
                            }
                            ModelOutcome::ModelOutcomeUnknown { .. } => {
                                state.observe(
                                    ports,
                                    apxm_runtime_protocol::ObservationKind::OutcomeUnknown,
                                    apxm_runtime_protocol::Commitment::Provisional,
                                    Some(&node_execution_id),
                                    Some(&occurrence_id),
                                    attempt_id.as_deref(),
                                    region_occurrence_id.as_deref(),
                                    None,
                                    None,
                                    None,
                                    Some(node_started_at),
                                )?;
                                state.append_effect_outcome_unknown(
                                    &node_execution_id,
                                    call.effect_id(),
                                );
                            }
                            ModelOutcome::TypedFailure { error } => {
                                state.append_invocation_failure(
                                    &node_execution_id,
                                    model_failure_envelope(&node_execution_id, error),
                                );
                            }
                        }
                        state.last_result = result.clone();
                        if state.last_operation_succeeded {
                            state.committed_output_node_execution_id =
                                Some(node_execution_id.clone());
                            state.committed_output_occurrence_id = Some(occurrence_id.clone());
                            state
                                .committed_output_region_occurrence_id
                                .clone_from(&region_occurrence_id);
                        }
                        state.node_outcomes.push(NodeOutcome::Model {
                            node_id: op.node_id.clone(),
                            outcome,
                            result,
                            replaced: false,
                        });
                        state.last_model_node_execution_id = Some(node_execution_id.clone());
                    }
                    SemanticOpKind::CapabilityInvoke => {
                        let capability_ref =
                            operand_str(op, "capability_ref").ok_or_else(|| {
                                ExecutionError::MissingOperand {
                                    node_id: op.node_id.clone(),
                                    operand: "capability_ref",
                                }
                            })?;
                        {
                            let arguments_type_ref = op
                                .operands
                                .iter()
                                .find(|operand| operand.slot == "arguments")
                                .ok_or_else(|| ExecutionError::MissingOperand {
                                    node_id: op.node_id.clone(),
                                    operand: "arguments",
                                })?
                                .type_ref
                                .clone();
                            let admission =
                                capability_invocations.get(&op.node_id).ok_or_else(|| {
                                    ExecutionError::MissingCapabilityInvocationAdmission {
                                        node_id: op.node_id.clone(),
                                    }
                                })?;
                            // This is not the grant check, and it is not
                            // `x == x`. `CapabilityGrantSet::resolve` answers
                            // "is this reference granted"; it is handed a bare
                            // reference and never sees a node id, so it cannot
                            // say which node the reference was granted *for*.
                            // The node-to-admission pairing is established
                            // entirely outside this crate: `capability_invocations`
                            // is a public field on `ExecutionRequest`, keyed by
                            // node id, that a composition root fills in by
                            // walking the AIR — and that `Continuation`
                            // serializes, so after a park it is rehydrated from
                            // persisted bytes and paired with an AIR supplied
                            // separately. Nothing between those two sources
                            // re-checks that the admission filed under this node
                            // still names the capability this node authored.
                            // This does, before any argument is materialized.
                            if admission.capability_ref != capability_ref {
                                return Err(
                                    ExecutionError::CapabilityInvocationAdmissionMismatch {
                                        node_id: op.node_id.clone(),
                                        authored: capability_ref,
                                        admitted: admission.capability_ref.clone(),
                                    },
                                );
                            }
                            // Every attempt records its decision before the
                            // effect is attempted, so a refusal is in the
                            // committed evidence rather than only in a log
                            // line. There is no undecided attempt to skip:
                            // the admission type has no "no decision" state.
                            let resolved = &admission.permission;
                            // Permission for a host-fulfilled reference is the
                            // host's decision (ADR-0025). APXM records the
                            // authored request and carries it to the host; it
                            // does not broker it, so it raises neither of its
                            // own approval observations here.
                            let host_fulfilled = is_host_capability_ref(&capability_ref);
                            if !host_fulfilled && !resolved.decision.is_allow() {
                                state.observe(
                                    ports,
                                    apxm_runtime_protocol::ObservationKind::ApprovalRequested,
                                    apxm_runtime_protocol::Commitment::Provisional,
                                    Some(&node_execution_id),
                                    Some(&occurrence_id),
                                    None,
                                    region_occurrence_id.as_deref(),
                                    None,
                                    None,
                                    None,
                                    Some(node_started_at),
                                )?;
                            }
                            if !host_fulfilled {
                                state.observe(
                                    ports,
                                    apxm_runtime_protocol::ObservationKind::ApprovalResolved,
                                    apxm_runtime_protocol::Commitment::Provisional,
                                    Some(&node_execution_id),
                                    Some(&occurrence_id),
                                    None,
                                    region_occurrence_id.as_deref(),
                                    None,
                                    None,
                                    None,
                                    Some(node_started_at),
                                )?;
                            }
                            state.seq += 1;
                            let mut decided = fact(
                                &state.program_invocation_id,
                                state.seq,
                                FactKind::CapabilityAttemptRecorded,
                                None,
                                None,
                                Some(node_execution_id.clone()),
                                None,
                            );
                            let recorded = runtime_fact_mut(&mut decided);
                            recorded.capability_ref = Some(capability_ref.clone());
                            recorded.permission_decision = Some(resolved.clone());
                            state.batch.push(decided);

                            // Anything the layer stack did not resolve to an
                            // outright allow refuses here: this driver has no
                            // approval broker, so `Ask` has nothing to ask.
                            let mut attempt_id = None;
                            let mut effect_id = None;
                            let outcome = if host_fulfilled {
                                let arguments_value_id =
                                    operand_str(op, "arguments").ok_or_else(|| {
                                        ExecutionError::MissingOperand {
                                            node_id: op.node_id.clone(),
                                            operand: "arguments",
                                        }
                                    })?;
                                let authored_arguments = materialize_ssa_value(
                                    air,
                                    &state,
                                    &op.node_id,
                                    &arguments_value_id,
                                    &mut BTreeSet::new(),
                                )?;
                                let request = CapabilityRequest::prepare(
                                    capability_ref.clone(),
                                    arguments_type_ref,
                                    authored_arguments,
                                    &state.program_invocation_id,
                                    &node_execution_id,
                                    admission.authority.clone(),
                                )
                                .map_err(ExecutionError::CapabilityRequest)?;
                                let capability_request_id = host_capability_request_id(
                                    &state.program_invocation_id,
                                    &node_execution_id,
                                );
                                let event_ref = EventRef::new(capability_request_id.clone())
                                    .map_err(|source| ExecutionError::InvalidEventRef {
                                        node_id: op.node_id.clone(),
                                        source,
                                    })?;
                                state.pending_host_capability =
                                    Some(apxm_runtime_protocol::HostCapabilityObservation {
                                        capability_request_id: capability_request_id.clone(),
                                        capability_ref: capability_ref.clone(),
                                        input: Some(
                                            request.arguments().canonical_json().to_owned(),
                                        ),
                                        authored_permission: Some(authored_permission(
                                            &resolved.decision,
                                        )),
                                        outcome: None,
                                        receipt_ref: None,
                                    });
                                state.observe(
                                    ports,
                                    apxm_runtime_protocol::ObservationKind::CapabilityRequested,
                                    apxm_runtime_protocol::Commitment::Provisional,
                                    Some(&node_execution_id),
                                    Some(&occurrence_id),
                                    None,
                                    region_occurrence_id.as_deref(),
                                    None,
                                    None,
                                    None,
                                    Some(node_started_at),
                                )?;
                                // Nothing in APXM runs. The request is published
                                // and the node awaits the host's answer through
                                // the same durable-event machinery `await.event`
                                // parks on.
                                let event_outcome = tokio::select! {
                                    outcome = ports.events.await_event(EventAwait {
                                        node_id: op.node_id.clone(),
                                        event_ref: event_ref.clone(),
                                    }) => outcome,
                                    () = ports.cancellation.cancelled() => EventOutcome::Cancelled,
                                };
                                if options.suspend_on_park
                                    && matches!(&event_outcome, EventOutcome::Parked)
                                {
                                    state.park_active_loops();
                                    state.seq += 1;
                                    state.batch.push(fact(
                                        &state.program_invocation_id,
                                        state.seq,
                                        FactKind::EventAwaitRegistered,
                                        None,
                                        Some(InvocationState::WaitingEvent),
                                        Some(op.node_id.clone()),
                                        None,
                                    ));
                                    state.seq += 1;
                                    state.batch.push(fact(
                                        &state.program_invocation_id,
                                        state.seq,
                                        FactKind::InvocationParked,
                                        None,
                                        Some(InvocationState::WaitingEvent),
                                        Some(op.node_id.clone()),
                                        None,
                                    ));
                                    return Ok(DriveEnd::Parked {
                                        state,
                                        continuation_id: op.node_id.clone(),
                                        event_ref: Some(event_ref),
                                        next_schedule_position: schedule_position + 1,
                                        parked_node_execution_id: Some(node_execution_id),
                                        parked_loop_path: loop_path.clone(),
                                        resume_value_id: op
                                            .result
                                            .as_ref()
                                            .map(|result| result.value_id.clone()),
                                    });
                                }
                                let settlement = host_settlement_from_event(
                                    &capability_request_id,
                                    &capability_ref,
                                    &event_outcome,
                                );
                                state.settle_host_capability(
                                    ports,
                                    &capability_ref,
                                    &settlement,
                                    &node_execution_id,
                                    &occurrence_id,
                                    region_occurrence_id.as_deref(),
                                    node_started_at,
                                )?;
                                host_capability_outcome(&settlement)
                            } else if resolved.decision.is_allow() {
                                let arguments_value_id =
                                    operand_str(op, "arguments").ok_or_else(|| {
                                        ExecutionError::MissingOperand {
                                            node_id: op.node_id.clone(),
                                            operand: "arguments",
                                        }
                                    })?;
                                let authored_arguments = materialize_ssa_value(
                                    air,
                                    &state,
                                    &op.node_id,
                                    &arguments_value_id,
                                    &mut BTreeSet::new(),
                                )?;
                                let request = CapabilityRequest::prepare(
                                    capability_ref,
                                    arguments_type_ref,
                                    authored_arguments,
                                    &state.program_invocation_id,
                                    &node_execution_id,
                                    admission.authority.clone(),
                                )
                                .map_err(ExecutionError::CapabilityRequest)?;
                                // The effect identity is minted by the
                                // canonical request constructor. It is an
                                // exact capability-attempt coordinate; do
                                // not synthesize a retry index here.
                                attempt_id = Some(request.effect().effect_id.clone());
                                effect_id.clone_from(&attempt_id);
                                tokio::select! {
                                    outcome = ports.capability.invoke_authorized(request) => outcome,
                                    () = ports.cancellation.cancelled() => {
                                        CapabilityOutcome::OutcomeUnknown {
                                            message: "capability effect outcome is unknown after cancellation".to_owned(),
                                        }
                                    }
                                }
                            } else {
                                CapabilityOutcome::Failed {
                                    message: format!(
                                        "capability '{capability_ref}' is {} by the {} layer",
                                        resolved.decision, resolved.layer
                                    ),
                                }
                            };
                            state.last_operation_succeeded =
                                matches!(&outcome, CapabilityOutcome::Completed { .. });
                            state.last_result = match &outcome {
                                CapabilityOutcome::Completed { result } => {
                                    Value::String(result.clone())
                                }
                                CapabilityOutcome::Failed { message }
                                | CapabilityOutcome::OutcomeUnknown { message } => {
                                    Value::String(message.clone())
                                }
                            };
                            if matches!(&outcome, CapabilityOutcome::Completed { .. }) {
                                state.committed_output_node_execution_id =
                                    Some(node_execution_id.clone());
                                state.committed_output_occurrence_id = Some(occurrence_id.clone());
                                state
                                    .committed_output_region_occurrence_id
                                    .clone_from(&region_occurrence_id);
                            }
                            // The request constructor supplied the exact
                            // effect identity when dispatch was authorized;
                            // denied work intentionally has no attempt id.
                            if attempt_id.is_some() {
                                state.observe(
                                    ports,
                                    apxm_runtime_protocol::ObservationKind::CapabilityAttempt,
                                    apxm_runtime_protocol::Commitment::Provisional,
                                    Some(&node_execution_id),
                                    Some(&occurrence_id),
                                    attempt_id.as_deref(),
                                    region_occurrence_id.as_deref(),
                                    None,
                                    None,
                                    None,
                                    Some(node_started_at),
                                )?;
                            }
                            match &outcome {
                                CapabilityOutcome::Completed { .. } => {}
                                CapabilityOutcome::OutcomeUnknown { .. } => {
                                    state.observe(
                                        ports,
                                        apxm_runtime_protocol::ObservationKind::OutcomeUnknown,
                                        apxm_runtime_protocol::Commitment::Provisional,
                                        Some(&node_execution_id),
                                        Some(&occurrence_id),
                                        attempt_id.as_deref(),
                                        region_occurrence_id.as_deref(),
                                        None,
                                        None,
                                        None,
                                        Some(node_started_at),
                                    )?;
                                    if let Some(effect_id) = effect_id.as_deref() {
                                        state.append_effect_outcome_unknown(
                                            &node_execution_id,
                                            effect_id,
                                        );
                                    }
                                }
                                CapabilityOutcome::Failed { message } => {
                                    state.append_invocation_failure(
                                        &node_execution_id,
                                        unavailable_failure_envelope(
                                            "capability",
                                            &node_execution_id,
                                            message,
                                        ),
                                    );
                                }
                            }
                            state.node_outcomes.push(NodeOutcome::Capability {
                                node_id: op.node_id.clone(),
                                outcome,
                                replaced: false,
                            });
                        }
                    }
                    SemanticOpKind::ProgramNew => {
                        let program_ref = operand_str(op, "program_ref").ok_or(
                            ExecutionError::MissingOperand {
                                node_id: op.node_id.clone(),
                                operand: "program_ref",
                            },
                        )?;
                        let outcome = ports
                            .composition
                            .program_new(CompositionRequest {
                                node_id: op.node_id.clone(),
                                receiver: CompositionReceiver::Program { program_ref },
                            })
                            .await;
                        state.last_operation_succeeded =
                            matches!(&outcome, CompositionOutcome::Created { .. });
                        state.observe(
                            ports,
                            apxm_runtime_protocol::ObservationKind::ProgramAttempt,
                            apxm_runtime_protocol::Commitment::Provisional,
                            Some(&node_execution_id),
                            Some(&occurrence_id),
                            // Composition has no retry field in its neutral
                            // outcome; the scheduler's node execution id is
                            // the exact attempt coordinate it does provide.
                            Some(&node_execution_id),
                            region_occurrence_id.as_deref(),
                            None,
                            None,
                            None,
                            Some(node_started_at),
                        )?;
                        state.last_result = Value::String(match &outcome {
                            CompositionOutcome::Created { child_instance_ref }
                            | CompositionOutcome::Invoked { child_instance_ref } => {
                                child_instance_ref.clone()
                            }
                            CompositionOutcome::Failed { message } => message.clone(),
                        });
                        if let CompositionOutcome::Failed { message } = &outcome {
                            state.append_invocation_failure(
                                &node_execution_id,
                                unavailable_failure_envelope(
                                    "program",
                                    &node_execution_id,
                                    message,
                                ),
                            );
                        }
                        if state.last_operation_succeeded {
                            state.seq += 1;
                            let mut attached = fact(
                                &state.program_invocation_id,
                                state.seq,
                                FactKind::ChildAttached,
                                None,
                                None,
                                Some(node_execution_id.clone()),
                                None,
                            );
                            runtime_fact_mut(&mut attached).air_node_id = Some(op.node_id.clone());
                            state.batch.push(attached);
                        }
                        state.node_outcomes.push(NodeOutcome::ProgramNew {
                            node_id: op.node_id.clone(),
                            outcome,
                        });
                        state.last_program_new_node_execution_id = Some(node_execution_id.clone());
                    }
                    SemanticOpKind::ProgramInvoke => {
                        let receiver = composition_receiver(op)?;
                        let outcome = ports
                            .composition
                            .program_invoke(CompositionRequest {
                                node_id: op.node_id.clone(),
                                receiver,
                            })
                            .await;
                        state.last_operation_succeeded =
                            matches!(&outcome, CompositionOutcome::Invoked { .. });
                        state.observe(
                            ports,
                            apxm_runtime_protocol::ObservationKind::ProgramAttempt,
                            apxm_runtime_protocol::Commitment::Provisional,
                            Some(&node_execution_id),
                            Some(&occurrence_id),
                            Some(&node_execution_id),
                            region_occurrence_id.as_deref(),
                            None,
                            None,
                            None,
                            Some(node_started_at),
                        )?;
                        state.last_result = Value::String(match &outcome {
                            CompositionOutcome::Created { child_instance_ref }
                            | CompositionOutcome::Invoked { child_instance_ref } => {
                                child_instance_ref.clone()
                            }
                            CompositionOutcome::Failed { message } => message.clone(),
                        });
                        if let CompositionOutcome::Failed { message } = &outcome {
                            state.append_invocation_failure(
                                &node_execution_id,
                                unavailable_failure_envelope(
                                    "program",
                                    &node_execution_id,
                                    message,
                                ),
                            );
                        }
                        if state.last_operation_succeeded {
                            state.seq += 1;
                            let mut attached = fact(
                                &state.program_invocation_id,
                                state.seq,
                                FactKind::ChildAttached,
                                None,
                                None,
                                Some(node_execution_id.clone()),
                                None,
                            );
                            let attached_fact = runtime_fact_mut(&mut attached);
                            attached_fact.air_node_id = Some(op.node_id.clone());
                            attached_fact
                                .parent_node_execution_id
                                .clone_from(&state.last_program_new_node_execution_id);
                            state.batch.push(attached);
                        }
                        state.node_outcomes.push(NodeOutcome::ProgramInvoke {
                            node_id: op.node_id.clone(),
                            outcome,
                        });
                    }
                    SemanticOpKind::AwaitEvent => {
                        let event_ref = operand_str(op, "event_ref")
                            .ok_or_else(|| ExecutionError::MissingOperand {
                                node_id: op.node_id.clone(),
                                operand: "event_ref",
                            })
                            .and_then(|value| {
                                EventRef::new(value).map_err(|source| {
                                    ExecutionError::InvalidEventRef {
                                        node_id: op.node_id.clone(),
                                        source,
                                    }
                                })
                            })?;
                        state.pending_event_ref = Some(event_ref.as_str().to_owned());
                        state.observe(
                            ports,
                            apxm_runtime_protocol::ObservationKind::EventWaiting,
                            apxm_runtime_protocol::Commitment::Provisional,
                            Some(&node_execution_id),
                            Some(&occurrence_id),
                            None,
                            region_occurrence_id.as_deref(),
                            None,
                            None,
                            None,
                            Some(node_started_at),
                        )?;
                        let outcome = tokio::select! {
                            outcome = ports.events.await_event(EventAwait {
                                node_id: op.node_id.clone(),
                                event_ref: event_ref.clone(),
                            }) => outcome,
                                    () = ports.cancellation.cancelled() => EventOutcome::Cancelled,
                        };
                        if let EventOutcome::Fulfilled {
                            event_ref: fulfilled_event_ref,
                            ..
                        } = &outcome
                            && fulfilled_event_ref != &event_ref
                        {
                            return Err(ExecutionError::EventRefMismatch {
                                expected: event_ref,
                                delivered: fulfilled_event_ref.clone(),
                            });
                        }
                        if let EventOutcome::Mismatched {
                            delivered_event_ref,
                        } = &outcome
                        {
                            return Err(ExecutionError::EventRefMismatch {
                                expected: event_ref,
                                delivered: delivered_event_ref.clone(),
                            });
                        }
                        state.last_operation_succeeded =
                            matches!(&outcome, EventOutcome::Fulfilled { .. });
                        match &outcome {
                            EventOutcome::Fulfilled {
                                event_ref: fulfilled_event_ref,
                                ..
                            } => {
                                state.pending_event_ref =
                                    Some(fulfilled_event_ref.as_str().to_owned());
                                state.observe(
                                    ports,
                                    apxm_runtime_protocol::ObservationKind::EventResumed,
                                    apxm_runtime_protocol::Commitment::Provisional,
                                    Some(&node_execution_id),
                                    Some(&occurrence_id),
                                    None,
                                    region_occurrence_id.as_deref(),
                                    None,
                                    None,
                                    None,
                                    Some(node_started_at),
                                )?;
                            }
                            EventOutcome::Cancelled => {
                                state.observe(
                                    ports,
                                    apxm_runtime_protocol::ObservationKind::InvocationCancelled,
                                    apxm_runtime_protocol::Commitment::Provisional,
                                    Some(&node_execution_id),
                                    Some(&occurrence_id),
                                    None,
                                    region_occurrence_id.as_deref(),
                                    None,
                                    None,
                                    None,
                                    Some(node_started_at),
                                )?;
                                state.append_invocation_cancelled(Some(&node_execution_id));
                            }
                            EventOutcome::Parked
                            | EventOutcome::Expired
                            | EventOutcome::Mismatched { .. } => {}
                        }
                        if options.suspend_on_park && matches!(&outcome, EventOutcome::Parked) {
                            state.park_active_loops();
                            state.seq += 1;
                            state.batch.push(fact(
                                &state.program_invocation_id,
                                state.seq,
                                FactKind::EventAwaitRegistered,
                                None,
                                Some(InvocationState::WaitingEvent),
                                Some(op.node_id.clone()),
                                None,
                            ));
                            state.seq += 1;
                            state.batch.push(fact(
                                &state.program_invocation_id,
                                state.seq,
                                FactKind::InvocationParked,
                                None,
                                Some(InvocationState::WaitingEvent),
                                Some(op.node_id.clone()),
                                None,
                            ));
                            return Ok(DriveEnd::Parked {
                                state,
                                continuation_id: op.node_id.clone(),
                                event_ref: Some(event_ref),
                                next_schedule_position: schedule_position + 1,
                                parked_node_execution_id: Some(node_execution_id),
                                parked_loop_path: loop_path.clone(),
                                resume_value_id: op
                                    .result
                                    .as_ref()
                                    .map(|result| result.value_id.clone()),
                            });
                        }
                        state.node_outcomes.push(NodeOutcome::AwaitEvent {
                            node_id: op.node_id.clone(),
                            outcome,
                        });
                    }
                }
                if let Some(result) = &op.result {
                    state
                        .values
                        .insert(result.value_id.clone(), state.last_result.clone());
                    state.last_result_value_id = Some(result.value_id.clone());
                    state.observe(
                        ports,
                        apxm_runtime_protocol::ObservationKind::OperandPublished,
                        apxm_runtime_protocol::Commitment::Provisional,
                        Some(&node_execution_id),
                        Some(&occurrence_id),
                        None,
                        region_occurrence_id.as_deref(),
                        None,
                        None,
                        None,
                        Some(node_started_at),
                    )?;
                    if state.last_operation_succeeded {
                        state.committed_output_node_execution_id = Some(node_execution_id.clone());
                        state.committed_output_occurrence_id = Some(occurrence_id.clone());
                        state
                            .committed_output_region_occurrence_id
                            .clone_from(&region_occurrence_id);
                    }
                }
                state.record_node_outcome(
                    loop_path,
                    &node_execution_id,
                    state.last_operation_succeeded,
                );
            }
            ScheduleStep::LoopBackEdge { static_loop_id } => {
                let region = air
                    .structural_ir
                    .iter()
                    .find(|region| region.region_id == *static_loop_id)
                    .expect("schedule loop references its AIR structural node");
                for (argument, carried) in loop_value_pairs(region, "carried")? {
                    let value = state
                        .values
                        .get(&carried.value_id)
                        .cloned()
                        .ok_or_else(|| ExecutionError::MissingControlValue {
                            region_id: static_loop_id.clone(),
                            value_id: carried.value_id.clone(),
                        })?;
                    state.values.insert(argument.value_id.clone(), value);
                }
                // Capture the dynamic loop occurrence before completion may
                // retire a failed frame. A static loop id is never sufficient
                // to identify a back-edge inside repeated/nested iterations.
                let (region_occurrence_id, iteration_occurrence_id) = state
                    .active_loops
                    .iter()
                    .find(|frame| frame.static_loop_id == *static_loop_id)
                    .map(|frame| {
                        (
                            frame.dynamic_occurrence_id.clone(),
                            format!(
                                "{}.iteration.{}",
                                frame.dynamic_occurrence_id, frame.iteration_index
                            ),
                        )
                    })
                    .map_or((None, None), |(region, iteration)| {
                        (Some(region), Some(iteration))
                    });
                state.complete_loop_iteration(static_loop_id);
                state.observe(
                    ports,
                    apxm_runtime_protocol::ObservationKind::JoinTransition,
                    apxm_runtime_protocol::Commitment::Provisional,
                    None,
                    iteration_occurrence_id.as_deref(),
                    None,
                    region_occurrence_id.as_deref(),
                    None,
                    None,
                    None,
                    None,
                )?;
                let loop_entry = schedule[..schedule_position]
                    .iter()
                    .rposition(|step| {
                        matches!(
                            step,
                            ScheduleStep::EnterLoop {
                                static_loop_id: candidate,
                                ..
                            } if candidate == static_loop_id
                        )
                    })
                    .ok_or_else(|| {
                        ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
                            message: format!(
                                "loop {static_loop_id} back edge has no schedule entry"
                            ),
                        })
                    })?;
                let repeats = matches!(
                    &schedule[loop_entry],
                    ScheduleStep::EnterLoop {
                        predicate: Some(_),
                        ..
                    }
                );
                if repeats {
                    schedule_position = loop_entry;
                    continue;
                }
            }
            ScheduleStep::ProgramYield {
                region_id,
                resume_value_id,
            } => {
                enforce_runtime_limits(&state, resource_ceilings)?;
                if options.suspend_on_park && options.yield_at_loop {
                    return Ok(DriveEnd::Parked {
                        state,
                        continuation_id: region_id.clone(),
                        event_ref: None,
                        next_schedule_position: schedule_position + 1,
                        parked_node_execution_id: None,
                        parked_loop_path: Vec::new(),
                        resume_value_id: resume_value_id.clone(),
                    });
                }
                return Ok(DriveEnd::RanToEnd(state));
            }
            ScheduleStep::ProgramReturn { .. } => {
                // A structural return owns the program's final value.  The
                // value may be a pure AIR assembly (for example an authored
                // `{ text: "ready" }` return) and therefore has no semantic
                // node execution to publish it.  Resolve the exact `output`
                // operand here, at the scheduler boundary, so the commit
                // path can stage the value and mint its SessionOutputRef.
                let ScheduleStep::ProgramReturn {
                    region_id: return_region_id,
                } = step
                else {
                    unreachable!("matched ProgramReturn above")
                };
                let return_value_id = air
                    .structural_ir
                    .iter()
                    .find(|region| region.region_id == *return_region_id)
                    .and_then(|region| {
                        region
                            .operands
                            .iter()
                            .find(|operand| operand.slot == "output")
                            .map(|operand| operand.value_id.clone())
                    });
                if let Some(value_id) = return_value_id {
                    state.last_result = materialize_ssa_value(
                        air,
                        &state,
                        return_region_id,
                        &value_id,
                        &mut BTreeSet::new(),
                    )?;
                    state.last_result_value_id = Some(value_id);
                    state.last_operation_succeeded = true;
                }
                state.fail_active_loops();
                enforce_runtime_limits(&state, resource_ceilings)?;
                return Ok(DriveEnd::RanToEnd(state));
            }
            ScheduleStep::ProgramExit { region_id } => {
                state.fail_active_loops();
                return Err(ExecutionError::ProgramThrew {
                    region_id: region_id.clone(),
                });
            }
        }
        schedule_position += 1;
    }
    enforce_runtime_limits(&state, resource_ceilings)?;
    Ok(DriveEnd::RanToEnd(state))
}

/// Execute one artifact-bound handler and apply only its explicit assignments.
fn evidence_hook_scope(scope: apxm_program::frontend_graph::HookScope) -> EvidenceHookScope {
    match scope {
        apxm_program::frontend_graph::HookScope::Agent => EvidenceHookScope::Agent,
        apxm_program::frontend_graph::HookScope::Loop => EvidenceHookScope::Loop,
        apxm_program::frontend_graph::HookScope::Node => EvidenceHookScope::Node,
        apxm_program::frontend_graph::HookScope::Model => EvidenceHookScope::Model,
        apxm_program::frontend_graph::HookScope::Capability => EvidenceHookScope::Capability,
    }
}

fn context_transition_fact(program_invocation_id: &str, seq: u64) -> Fact {
    let mut fact = fact(
        program_invocation_id,
        seq,
        FactKind::ContextTransitioned,
        None,
        None,
        None,
        None,
    );
    let runtime = runtime_fact_mut(&mut fact);
    runtime.context_transition_id = Some(format!("context-transition.{seq}"));
    runtime.context_before_ref = Some(context_ref(format!("context.{}.before", seq)));
    runtime.context_after_ref = Some(context_ref(format!("context.{}.after", seq)));
    fact
}

async fn apply_static_hook(
    state: &mut DriveState,
    ports: &ExecutionPorts,
    air: &AirModule,
    binding: &HookBinding,
) -> Result<(Value, Value), ExecutionError> {
    let before = state.context.clone();
    let target_snapshot = state
        .hook_target_snapshots
        .get(&binding.body_region_id)
        .cloned();
    let target_result = target_snapshot.as_ref().map_or_else(
        || state.last_result.clone(),
        |snapshot| snapshot.result.clone(),
    );
    let captured_context = match binding.assigned_context_value_id.as_deref() {
        Some(value_id) => Some(materialize_ssa_value(
            air,
            state,
            &binding.body_region_id,
            value_id,
            &mut BTreeSet::new(),
        )?),
        None => None,
    };
    let outcome = ports
        .hook_handlers
        .execute(StaticHookInvocation {
            binding,
            context: &state.context,
            result: &target_result,
            captured_context,
        })
        .await
        .map_err(ExecutionError::StaticHook)?;

    // The one authority bit a Hook binding carries about itself is its return
    // mode, and it used to be read nowhere: a Hook declared `observe` whose
    // handler replaced the result replaced it anyway. An observing Hook now
    // mutates nothing, whatever the injected port hands back.
    if binding.return_mode == HookReturnMode::Observe
        && !matches!(
            outcome,
            StaticHookResult::Keep {
                assigned_context: None
            }
        )
    {
        return Err(ExecutionError::StaticHook(StaticHookExecutionError {
            hook_id: binding.hook_id.clone(),
            message: "an observing Hook returned a replacement for Context or the target result"
                .to_string(),
        }));
    }

    match outcome {
        StaticHookResult::Keep { assigned_context } => {
            if let Some(context) = assigned_context {
                state.context = context;
            }
        }
        StaticHookResult::Replace {
            assigned_context,
            result,
        } => {
            if let Some(context) = assigned_context {
                state.context = context;
            }
            state.last_result = result.clone();
            if let Some(value_id) = target_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.result_value_id.as_ref())
                .or(state.last_result_value_id.as_ref())
            {
                state.values.insert(value_id.clone(), result.clone());
            }
            // A replacement has to reach the node outcome the evidence records,
            // or a Capability-scope Hook would replace the value the program
            // goes on to use while the committed outcome still reported the
            // original — two different answers to one question.
            match state
                .node_outcomes
                .iter_mut()
                .rev()
                .find(|outcome| node_outcome_id(outcome) == Some(binding.target_selector.as_str()))
            {
                Some(NodeOutcome::Model {
                    result: model_result,
                    replaced,
                    ..
                }) => {
                    *model_result = result;
                    *replaced = true;
                }
                Some(NodeOutcome::Capability {
                    outcome, replaced, ..
                }) => {
                    *outcome = CapabilityOutcome::Completed {
                        result: render_replacement(&result),
                    };
                    *replaced = true;
                }
                _ => {}
            }
        }
    }
    if binding.phase == apxm_program::frontend_graph::HookPhase::After
        && let Some(snapshot) = target_snapshot
    {
        state.last_result = match state
            .node_outcomes
            .iter()
            .rev()
            .find(|outcome| node_outcome_id(outcome) == Some(binding.target_selector.as_str()))
        {
            Some(NodeOutcome::Model { result, .. }) => result.clone(),
            Some(NodeOutcome::Capability { outcome, .. }) => match outcome {
                CapabilityOutcome::Completed { result }
                | CapabilityOutcome::Failed { message: result }
                | CapabilityOutcome::OutcomeUnknown { message: result } => {
                    Value::String(result.clone())
                }
            },
            _ => snapshot.result.clone(),
        };
        state.last_result_value_id = snapshot.result_value_id;
        state.last_operation_succeeded = snapshot.succeeded;
    }
    Ok((before, state.context.clone()))
}

fn node_outcome_id(outcome: &NodeOutcome) -> Option<&str> {
    match outcome {
        NodeOutcome::Model { node_id, .. }
        | NodeOutcome::Capability { node_id, .. }
        | NodeOutcome::ExternalAgent { node_id, .. }
        | NodeOutcome::ProgramNew { node_id, .. }
        | NodeOutcome::ProgramInvoke { node_id, .. }
        | NodeOutcome::AwaitEvent { node_id, .. } => Some(node_id.as_str()),
    }
}

/// Render a Hook's replacement value as the string a Capability outcome
/// carries, without inventing a JSON wrapper around an already-textual result.
fn render_replacement(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Perform the one atomic commit for a completed run and build its report. The
/// invocation-committed fact is appended just before the commit, exactly as the
/// single-shot path has always done.
async fn commit_and_report(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    program_invocation_ref: &ProgramInvocationRef,
    commit_id: &str,
    mut write_set: AtomicWriteSet,
    mut state: DriveState,
) -> Result<RunReport, ExecutionError> {
    let expected = ports
        .execution_commit
        .current_version(program_instance_ref)
        .await;
    if ports.cancellation.is_cancelled() && !state.has_terminal_non_success() {
        state.observe(
            ports,
            apxm_runtime_protocol::ObservationKind::InvocationCancelled,
            apxm_runtime_protocol::Commitment::Provisional,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )?;
        state.append_invocation_cancelled(None);
    }
    let unknown_outcome = state.has_unknown_outcome();
    let terminal_non_success = state.has_terminal_non_success() || unknown_outcome;
    if !terminal_non_success {
        state.seq += 1;
        state.batch.push(fact(
            &state.program_invocation_id,
            state.seq,
            FactKind::InvocationCommitted,
            None,
            Some(InvocationState::CommittedReturn),
            None,
            Some(expected + 1),
        ));
    }

    let prepared_output = if terminal_non_success {
        None
    } else {
        prepare_final_output(
            ports,
            &state,
            program_instance_ref,
            program_invocation_ref,
            commit_id,
        )
        .await?
    };
    let mut tuple = commit_tuple(&state, None, None);
    if let Some(prepared_output) = prepared_output {
        tuple.output_refs.push(
            serde_json::to_value(prepared_output)
                .map_err(|error| ExecutionError::OutputPreparation(error.to_string()))?,
        );
    }
    // Only the exact ref returned by the pre-commit staging port may cross the
    // commit boundary. The driver never invents a readable ref after commit.
    let output_ref = tuple
        .output_refs
        .iter()
        .find_map(|value| {
            (value.get("ref_type").and_then(Value::as_str) == Some("SessionOutputRef"))
                .then(|| value.get("ref").and_then(Value::as_str))
                .flatten()
        })
        .map(str::to_owned);
    let output_node_execution_id = state.committed_output_node_execution_id.clone();
    let output_occurrence_id = state.committed_output_occurrence_id.clone();
    let output_region_occurrence_id = state.committed_output_region_occurrence_id.clone();
    // Terminal coordinates are captured when the scheduler observes the
    // terminal node/effect. They must not be reconstructed from an output
    // path after the fact (and failures have no output path at all).
    let terminal_node_execution_id = if terminal_non_success {
        state.terminal_node_execution_id.clone()
    } else {
        output_node_execution_id
            .clone()
            .or_else(|| state.current_node_execution_id.clone())
    };
    let terminal_occurrence_id = if terminal_non_success {
        state.terminal_occurrence_id.clone()
    } else {
        output_occurrence_id
            .clone()
            .or_else(|| state.current_occurrence_id.clone())
    };
    let terminal_region_occurrence_id = if terminal_non_success {
        state.terminal_region_occurrence_id.clone()
    } else {
        output_region_occurrence_id
            .clone()
            .or_else(|| state.current_region_occurrence_id.clone())
    };
    if !terminal_non_success && let Some(output_ref) = output_ref.as_deref() {
        let observation = state.stage_observation(
            ports,
            apxm_runtime_protocol::ObservationKind::ContentPublished,
            apxm_runtime_protocol::Commitment::Provisional,
            output_node_execution_id.as_deref(),
            output_occurrence_id.as_deref(),
            None,
            output_region_occurrence_id.as_deref(),
            Some(output_ref),
            Some(output_ref),
            None,
            None,
        )?;
        DriveState::deliver_precommit(ports, &observation)?;
    }
    let mut committed_observations = Vec::new();
    let evidence_index =
        state
            .batch
            .len()
            .checked_sub(1)
            .ok_or_else(|| ExecutionError::InvalidCommitRequest {
                message: "completed execution has no evidence fact for terminal markers".into(),
            })?;
    let precommit_evidence_ref = PrecommitEvidenceRef::new(
        commit_id,
        program_invocation_ref.as_str(),
        evidence_index as u64,
    )
    .map_err(|error| ExecutionError::InvalidCommitRequest {
        message: error.to_owned(),
    })?;
    precommit_evidence_ref
        .validate()
        .map_err(|error| ExecutionError::InvalidCommitRequest {
            message: error.to_owned(),
        })?;
    if !terminal_non_success && let Some(output_ref) = output_ref.as_deref() {
        committed_observations.push(state.stage_observation(
            ports,
            apxm_runtime_protocol::ObservationKind::ContentCommitted,
            apxm_runtime_protocol::Commitment::Committed,
            output_node_execution_id.as_deref(),
            output_occurrence_id.as_deref(),
            None,
            output_region_occurrence_id.as_deref(),
            None,
            Some(output_ref),
            None,
            None,
        )?);
    }
    let has_failure = state
        .batch
        .iter()
        .any(|fact| fact.is_kind(FactKind::InvocationFailed))
        || state.node_outcomes.iter().any(|outcome| {
            node_outcome_terminal_status(outcome) == Some(RunTerminalStatus::Failed)
        });
    if terminal_non_success && has_failure {
        // Failure is a terminal committed observation, not merely a
        // provisional log. Its evidence ref is minted before this observation
        // enters the tuple digest, and its coordinates come from the node
        // where the scheduler recorded the failure.
        committed_observations.push(state.stage_observation(
            ports,
            apxm_runtime_protocol::ObservationKind::InvocationFailed,
            apxm_runtime_protocol::Commitment::Committed,
            terminal_node_execution_id.as_deref(),
            terminal_occurrence_id.as_deref(),
            None,
            terminal_region_occurrence_id.as_deref(),
            None,
            None,
            Some(precommit_evidence_ref.as_str()),
            None,
        )?);
    }
    if terminal_non_success && !has_failure {
        let terminal_kind = if unknown_outcome {
            apxm_runtime_protocol::ObservationKind::OutcomeUnknown
        } else {
            apxm_runtime_protocol::ObservationKind::InvocationCancelled
        };
        // Cancellation and unknown effect outcomes are also owner facts. If
        // the scheduler has a terminal node coordinate, retain that exact
        // coordinate and the same precommit evidence identity in the durable
        // observation. An invocation cancelled before any node exists keeps
        // its earlier invocation-level observation without inventing a node.
        committed_observations.push(state.stage_observation(
            ports,
            terminal_kind,
            apxm_runtime_protocol::Commitment::Committed,
            terminal_node_execution_id.as_deref(),
            terminal_occurrence_id.as_deref(),
            None,
            terminal_region_occurrence_id.as_deref(),
            None,
            None,
            Some(precommit_evidence_ref.as_str()),
            None,
        )?);
    }
    if !terminal_non_success {
        // Both terminal markers carry the kernel-minted canonical evidence
        // reference for the final evidence record in this commit. It is
        // validated before entering the atomic observation digest.
        committed_observations.push(state.stage_observation(
            ports,
            apxm_runtime_protocol::ObservationKind::TerminalCommitted,
            apxm_runtime_protocol::Commitment::Committed,
            terminal_node_execution_id.as_deref(),
            terminal_occurrence_id.as_deref(),
            None,
            terminal_region_occurrence_id.as_deref(),
            None,
            output_ref.as_deref(),
            Some(precommit_evidence_ref.as_str()),
            None,
        )?);
        committed_observations.push(state.stage_observation(
            ports,
            apxm_runtime_protocol::ObservationKind::EvidenceCommitted,
            apxm_runtime_protocol::Commitment::Committed,
            terminal_node_execution_id.as_deref(),
            terminal_occurrence_id.as_deref(),
            None,
            terminal_region_occurrence_id.as_deref(),
            None,
            None,
            Some(precommit_evidence_ref.as_str()),
            None,
        )?);
    }
    tuple.observations = state
        .durable_observations
        .iter()
        .map(|observation| {
            serde_json::to_value(observation)
                .expect("typed execution observation serializes deterministically")
        })
        .collect();
    bind_tuple_digests(&mut write_set, &tuple);
    let request = ExecutionCommitRequest {
        commit_id: commit_id.to_string(),
        program_instance_ref: program_instance_ref.clone(),
        program_invocation_ref: program_invocation_ref.clone(),
        idempotency_key: format!("idem.{commit_id}"),
        expected_program_state_version: expected,
        write_set,
        tuple,
        evidence_batch: state.batch.clone(),
    };
    request
        .validate()
        .map_err(|error| ExecutionError::InvalidCommitRequest {
            message: error.to_string(),
        })?;
    let commit = ports.execution_commit.commit(request).await;

    match &commit {
        ExecutionCommitResult::Committed { .. } => {
            for observation in &committed_observations {
                DriveState::deliver_post_commit(ports, observation);
            }
        }
        ExecutionCommitResult::OutcomeUnknown { .. }
        | ExecutionCommitResult::CompareConflict { .. } => {
            // A commit result is not an execution observation. Any effect or
            // invocation uncertainty must already have been staged in the
            // atomic tuple; do not publish a live-only record here.
        }
    }

    let operational_usage = publish_committed_native_model_usage(
        ports,
        commit_id,
        &commit,
        &state.committed_model_attempts,
        &state.committed_model_lineages,
    )
    .await;

    let terminal_status = if unknown_outcome
        || state.node_outcomes.iter().any(|outcome| {
            node_outcome_terminal_status(outcome) == Some(RunTerminalStatus::OutcomeUnknown)
        }) {
        RunTerminalStatus::OutcomeUnknown
    } else if state.batch.iter().any(|fact| {
        fact.is_kind(FactKind::InvocationCancelled)
            || matches!(
                fact.runtime().and_then(|runtime| runtime.invocation_state),
                Some(InvocationState::Cancelled)
            )
    }) || state
        .node_outcomes
        .iter()
        .any(|outcome| node_outcome_terminal_status(outcome) == Some(RunTerminalStatus::Cancelled))
    {
        RunTerminalStatus::Cancelled
    } else if state.batch.iter().any(|fact| {
        fact.is_kind(FactKind::InvocationFailed)
            || matches!(
                fact.runtime().and_then(|runtime| runtime.invocation_state),
                Some(InvocationState::Failed)
            )
    }) || state
        .node_outcomes
        .iter()
        .any(|outcome| node_outcome_terminal_status(outcome) == Some(RunTerminalStatus::Failed))
    {
        RunTerminalStatus::Failed
    } else {
        RunTerminalStatus::CommittedReturn
    };
    Ok(RunReport {
        node_outcomes: state.node_outcomes,
        native_usage: state.native_usage,
        external_agent_evidence: state.external_agent_evidence,
        final_context: state.context,
        commit,
        terminal_status,
        operational_usage,
    })
}

async fn publish_committed_native_model_usage(
    ports: &ExecutionPorts,
    commit_id: &str,
    commit: &ExecutionCommitResult,
    attempts: &[ModelAttemptRecordedFact],
    lineages: &[InferenceUsageLineage],
) -> CommittedNativeModelUsageOutcome {
    let ExecutionCommitResult::Committed {
        evidence_position_ref,
        ..
    } = commit
    else {
        return CommittedNativeModelUsageOutcome::NotApplicable;
    };
    if attempts.is_empty() {
        return CommittedNativeModelUsageOutcome::NotApplicable;
    }
    let Some(port) = &ports.operational_usage else {
        return CommittedNativeModelUsageOutcome::NotConfigured;
    };
    if attempts.len() != lineages.len() {
        return CommittedNativeModelUsageOutcome::Failed(CommittedNativeModelUsageError::Rejected);
    }
    let evidence_position_ref = EvidencePositionRef {
        ref_type: EvidencePositionRefType::EvidencePositionRef,
        r#ref: evidence_position_ref.clone(),
    };
    for (attempt, sealed_lineage) in attempts.iter().zip(lineages) {
        let mut lineage = sealed_lineage.clone();
        if lineage
            .bind_evidence(attempt.fact_id.clone(), commit_id.to_string())
            .is_err()
        {
            return CommittedNativeModelUsageOutcome::Failed(
                CommittedNativeModelUsageError::Rejected,
            );
        }
        let Ok(usage) = CommittedNativeModelUsage::from_lineage(
            commit_id.to_string(),
            evidence_position_ref.clone(),
            attempt.clone(),
            &lineage,
        ) else {
            return CommittedNativeModelUsageOutcome::Failed(
                CommittedNativeModelUsageError::Rejected,
            );
        };
        if let Err(error) = port.publish(usage).await {
            return CommittedNativeModelUsageOutcome::Failed(error);
        }
    }
    CommittedNativeModelUsageOutcome::Published
}

fn validate_commit_inputs(
    program_instance_ref: &ProgramInstanceRef,
    program_invocation_ref: &ProgramInvocationRef,
    commit_id: &str,
    write_set: &AtomicWriteSet,
) -> Result<(), ExecutionError> {
    for (field, value) in [
        (
            "next_program_state_digest",
            &write_set.next_program_state_digest,
        ),
        ("continuation_digest", &write_set.continuation_digest),
        (
            "checkpoint_effect_outcomes_digest",
            &write_set.checkpoint_effect_outcomes_digest,
        ),
        (
            "runtime_evidence_batch_digest",
            &write_set.runtime_evidence_batch_digest,
        ),
        ("usage_facts_digest", &write_set.usage_facts_digest),
        (
            "session_output_refs_digest",
            &write_set.session_output_refs_digest,
        ),
    ] {
        if !is_digest(value) {
            return Err(ExecutionError::InvalidCommitRequest {
                message: format!("commit field {field} is not a sha256 digest"),
            });
        }
    }
    // The request write-set supplied at admission contains placeholders for
    // the scheduler-owned evidence/output digests.  Those two members are
    // rebound from the actual tuple immediately before commit; validating
    // them against an empty pre-drive tuple here would reject every otherwise
    // valid execution before it can produce its authoritative observations.
    let mut write_set_for_validation = write_set.clone();
    write_set_for_validation.runtime_evidence_batch_digest =
        runtime_evidence_and_observation_digest(&[], &[]);
    write_set_for_validation.session_output_refs_digest = output_refs_digest(&[]);
    let request = ExecutionCommitRequest {
        commit_id: commit_id.to_string(),
        program_instance_ref: program_instance_ref.clone(),
        program_invocation_ref: program_invocation_ref.clone(),
        idempotency_key: format!("idem.{commit_id}"),
        expected_program_state_version: 0,
        write_set: write_set_for_validation,
        tuple: ExecutionCommitTuple::empty(Vec::new()),
        evidence_batch: Vec::new(),
    };
    request
        .validate()
        .map_err(|error| ExecutionError::InvalidCommitRequest {
            message: error.to_string(),
        })
}

/// Hold every supplied Hook binding against the AIR it will be scheduled
/// against.
///
/// `build_schedule` keys bindings by `body_region_id` and emits a Hook step
/// only where a region matches, so a binding naming a region this AIR does not
/// have is silently inert: the Hook's Observe/ReplaceResult contract never
/// applies and no evidence says a Hook was even supposed to run. Two bindings
/// on one region are the same failure with a different cause — the map keeps
/// whichever comes last and drops the other without a word. The compile path
/// catches both against a FrontendGraph; nothing did at this boundary, where
/// AIR and bindings arrive from separate sources and a rehydrated
/// `Continuation` arrives from persisted bytes.
fn validate_hook_bindings(
    air: &AirModule,
    hook_bindings: &[HookBinding],
) -> Result<(), ExecutionError> {
    let regions = air
        .structural_ir
        .iter()
        .map(|region| region.region_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut claimed = BTreeSet::new();
    let mut claimed_hook_ids = BTreeSet::new();
    for binding in hook_bindings {
        if !regions.contains(binding.body_region_id.as_str()) {
            return Err(ExecutionError::InvalidAir {
                message: format!(
                    "hook {}: body_region_id {} names no region in this AIR, so the Hook \
                     would never run",
                    binding.hook_id, binding.body_region_id
                ),
            });
        }
        if !claimed.insert(binding.body_region_id.as_str()) {
            return Err(ExecutionError::InvalidAir {
                message: format!(
                    "hook {}: body_region_id {} is claimed by another Hook binding, so only \
                     one of them would run",
                    binding.hook_id, binding.body_region_id
                ),
            });
        }
        if !claimed_hook_ids.insert(binding.hook_id.as_str()) {
            return Err(ExecutionError::InvalidAir {
                message: format!(
                    "hook {}: hook_id is claimed more than once in the execution request",
                    binding.hook_id
                ),
            });
        }
    }

    let carried: Vec<&HookBinding> = air
        .structural_ir
        .iter()
        .filter_map(|region| region.hook.as_ref())
        .collect();
    if carried.len() != hook_bindings.len() {
        return Err(ExecutionError::InvalidAir {
            message: format!(
                "AIR carries {} Hook bindings but the execution request supplies {}",
                carried.len(),
                hook_bindings.len()
            ),
        });
    }
    for (index, (carried, supplied)) in carried.iter().zip(hook_bindings).enumerate() {
        if *carried != supplied {
            return Err(ExecutionError::InvalidAir {
                message: format!(
                    "Hook binding at AIR order {index} ({}) differs from the binding carried by its AIR body region",
                    supplied.hook_id
                ),
            });
        }
    }
    Ok(())
}

fn validate_execution_request(
    air: &AirModule,
    initial_values: &BTreeMap<String, Value>,
    hook_bindings: &[HookBinding],
) -> Result<(), ExecutionError> {
    let structural_counts = [
        (
            "semantic_operations",
            air.semantic_operations.len(),
            MAX_SEMANTIC_OPERATIONS,
        ),
        (
            "structural_regions",
            air.structural_ir.len(),
            MAX_STRUCTURAL_REGIONS,
        ),
        (
            "value_assemblies",
            air.value_assemblies.len(),
            MAX_VALUE_ASSEMBLIES,
        ),
        ("hook_bindings", hook_bindings.len(), MAX_HOOK_BINDINGS),
        ("initial_values", initial_values.len(), MAX_INITIAL_VALUES),
    ];
    if let Some((name, observed, limit)) = structural_counts
        .into_iter()
        .find(|(_, observed, limit)| *observed > *limit)
    {
        return Err(ExecutionError::InvalidAir {
            message: format!("{name} has {observed} entries, limit is {limit}"),
        });
    }
    validate_hook_bindings(air, hook_bindings)?;
    let verdict = air.verify();
    if !verdict.is_accepted() {
        let message = verdict
            .into_diagnostics()
            .into_iter()
            .map(|diagnostic| format!("{}:{}", diagnostic.location, diagnostic.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ExecutionError::InvalidAir { message });
    }

    let assembled = air
        .value_assemblies
        .iter()
        .map(|assembly| assembly.value_id.as_str())
        .collect::<BTreeSet<_>>();
    let defined = air
        .semantic_operations
        .iter()
        .filter_map(|operation| {
            operation
                .result
                .as_ref()
                .map(|result| result.value_id.as_str())
        })
        .chain(air.structural_ir.iter().flat_map(|region| {
            region
                .block_arguments
                .iter()
                .map(|argument| argument.value_id.as_str())
        }))
        .collect::<BTreeSet<_>>();

    for operation in &air.semantic_operations {
        if operation.op != SemanticOpKind::CapabilityInvoke {
            continue;
        }
        let Some(arguments) = operation
            .operands
            .iter()
            .find(|operand| operand.slot == "arguments")
        else {
            continue;
        };
        if initial_values.contains_key(arguments.value_id.as_str()) {
            return Err(ExecutionError::InvalidAir {
                message: format!(
                    "node {}: initial_values may not supply a capability argument ({})",
                    operation.node_id, arguments.value_id
                ),
            });
        }
        if let Some(supplied_value_id) = authored_value_reaches_initial_value(
            air,
            &arguments.value_id,
            initial_values,
            &mut BTreeSet::new(),
        ) {
            return Err(ExecutionError::InvalidAir {
                message: format!(
                    "node {}: initial_values may not supply a capability argument dependency ({})",
                    operation.node_id, supplied_value_id
                ),
            });
        }
        if assembled.contains(arguments.value_id.as_str())
            || defined.contains(arguments.value_id.as_str())
        {
            continue;
        }
        return Err(ExecutionError::InvalidAir {
            message: format!(
                "node {}: capability argument is not an authored assembly or defined SSA value ({})",
                operation.node_id, arguments.value_id
            ),
        });
    }

    for value_id in initial_values.keys() {
        if assembled.contains(value_id.as_str()) {
            return Err(ExecutionError::InvalidAir {
                message: format!("initial_values cannot override authored assembly {value_id}"),
            });
        }
        if air.semantic_operations.iter().any(|operation| {
            operation
                .result
                .as_ref()
                .is_some_and(|result| result.value_id == *value_id)
        }) {
            return Err(ExecutionError::InvalidAir {
                message: format!(
                    "initial_values cannot supply a semantic result before its invocation ({value_id})"
                ),
            });
        }
    }
    Ok(())
}

fn authored_value_reaches_initial_value(
    air: &AirModule,
    value_id: &str,
    initial_values: &BTreeMap<String, Value>,
    visiting: &mut BTreeSet<String>,
) -> Option<String> {
    if initial_values.contains_key(value_id) {
        return Some(value_id.to_string());
    }
    if !visiting.insert(value_id.to_string()) {
        return None;
    }
    let assembly = air
        .value_assemblies
        .iter()
        .find(|assembly| assembly.value_id == value_id)?;
    let mut references = Vec::new();
    collect_runtime_expression_references(&assembly.expression, &mut references);
    references.into_iter().find_map(|dependency| {
        authored_value_reaches_initial_value(air, &dependency, initial_values, visiting)
    })
}

fn validate_resume_capability_arguments(
    air: &AirModule,
    resume_value_id: &str,
) -> Result<(), ExecutionError> {
    for operation in &air.semantic_operations {
        if operation.op != SemanticOpKind::CapabilityInvoke {
            continue;
        }
        let Some(arguments) = operation
            .operands
            .iter()
            .find(|operand| operand.slot == "arguments")
        else {
            continue;
        };
        let mut visiting = BTreeSet::new();
        if arguments.value_id == resume_value_id
            || authored_value_reaches_resume(
                air,
                &arguments.value_id,
                resume_value_id,
                &mut visiting,
            )
        {
            return Err(ExecutionError::InvalidAir {
                message: format!(
                    "node {}: delivered resume input cannot become a capability argument ({})",
                    operation.node_id, resume_value_id
                ),
            });
        }
    }
    Ok(())
}

fn authored_value_reaches_resume(
    air: &AirModule,
    value_id: &str,
    resume_value_id: &str,
    visiting: &mut BTreeSet<String>,
) -> bool {
    if value_id == resume_value_id || !visiting.insert(value_id.to_string()) {
        return value_id == resume_value_id;
    }
    let Some(assembly) = air
        .value_assemblies
        .iter()
        .find(|assembly| assembly.value_id == value_id)
    else {
        return false;
    };
    let mut references = Vec::new();
    collect_runtime_expression_references(&assembly.expression, &mut references);
    references.into_iter().any(|dependency| {
        authored_value_reaches_resume(air, &dependency, resume_value_id, visiting)
    })
}

fn collect_runtime_expression_references(
    expression: &ValueExpression,
    references: &mut Vec<String>,
) {
    match expression {
        ValueExpression::Ssa { value_id } => references.push(value_id.clone()),
        ValueExpression::Projection { root, .. } => {
            collect_runtime_expression_references(root, references);
        }
        ValueExpression::Object { fields } => fields
            .iter()
            .for_each(|field| collect_runtime_expression_references(&field.value, references)),
        ValueExpression::Array { items } => items
            .iter()
            .for_each(|item| collect_runtime_expression_references(item, references)),
        ValueExpression::Context { .. }
        | ValueExpression::String { .. }
        | ValueExpression::Integer { .. }
        | ValueExpression::Boolean { .. }
        | ValueExpression::Null => {}
    }
}

/// Assemble the one authoritative execution tuple for a completion or yield.
fn commit_tuple(
    state: &DriveState,
    continuation: Option<Value>,
    event_wait: Option<Value>,
) -> ExecutionCommitTuple {
    ExecutionCommitTuple {
        context: state.context.clone(),
        continuation,
        event_wait,
        effect_outcomes: state.node_outcomes.iter().map(node_outcome_value).collect(),
        evidence: state.batch.clone(),
        usage: serde_json::json!({
            "input_tokens": state.native_usage.input_tokens,
            "output_tokens": state.native_usage.output_tokens,
        }),
        output_refs: Vec::new(),
        observations: state
            .durable_observations
            .iter()
            .map(|observation| {
                serde_json::to_value(observation)
                    .expect("typed execution observation serializes deterministically")
            })
            .collect(),
    }
}

/// Stage the final redacted output before the atomic commit. The commit-local
/// adapter owns the bytes and returns the only readable reference the driver
/// may place in the tuple.
async fn prepare_final_output(
    ports: &ExecutionPorts,
    state: &DriveState,
    program_instance_ref: &ProgramInstanceRef,
    program_invocation_ref: &ProgramInvocationRef,
    commit_id: &str,
) -> Result<Option<PreparedSessionOutputRef>, ExecutionError> {
    // Most outputs are produced by a semantic node, but a structural return
    // may return an assembled value directly and consequently has no dynamic
    // node/occurrence coordinates.  The return value was resolved by the
    // scheduler and is still authoritative; only omit output when the run had
    // neither a producing node nor an explicit return value.
    if state.committed_output_node_execution_id.is_none() && state.last_result_value_id.is_none() {
        return Ok(None);
    }
    let node_execution_id = state.committed_output_node_execution_id.as_ref();
    let content = serde_json::to_vec(&state.last_result).map_err(|error| {
        ExecutionError::OutputPreparation(format!("encode final output: {error}"))
    })?;
    let prepared = ports
        .execution_commit
        .prepare_output(SessionOutputPreparation {
            contract: SESSION_OUTPUT_REF_CONTRACT.to_owned(),
            commit_id: commit_id.to_owned(),
            program_instance_ref: program_instance_ref.as_str().to_owned(),
            program_invocation_ref: program_invocation_ref.as_str().to_owned(),
            content,
            media_type: "application/json".to_owned(),
            visibility: SessionOutputVisibility::Provisional,
            node_execution_id: node_execution_id.cloned(),
            occurrence_id: state.committed_output_occurrence_id.clone(),
            access_scope_ref: program_invocation_ref.as_str().to_owned(),
            disclosure_ref: None,
        })
        .await
        .map_err(ExecutionError::OutputPreparation)?;
    prepared
        .validate()
        .map_err(|error| ExecutionError::OutputPreparation(error.to_owned()))?;
    if prepared.program_instance_id != program_instance_ref.as_str()
        || prepared.program_invocation_id != program_invocation_ref.as_str()
        || prepared.node_execution_id.as_deref() != node_execution_id.map(String::as_str)
        || prepared.occurrence_id != state.committed_output_occurrence_id
    {
        return Err(ExecutionError::OutputPreparation(
            "prepared output scope does not match its producing node".into(),
        ));
    }
    Ok(Some(prepared))
}

fn output_refs_digest(output_refs: &[Value]) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(canonical_json_bytes(&Value::Array(output_refs.to_vec(),)))
    )
}

fn bind_tuple_digests(write_set: &mut AtomicWriteSet, tuple: &ExecutionCommitTuple) {
    write_set.session_output_refs_digest = output_refs_digest(&tuple.output_refs);
    write_set.runtime_evidence_batch_digest =
        runtime_evidence_and_observation_digest(&tuple.evidence, &tuple.observations);
}

fn node_outcome_value(outcome: &NodeOutcome) -> Value {
    match outcome {
        NodeOutcome::Model { node_id, .. }
        | NodeOutcome::Capability { node_id, .. }
        | NodeOutcome::ExternalAgent { node_id, .. }
        | NodeOutcome::ProgramNew { node_id, .. }
        | NodeOutcome::ProgramInvoke { node_id, .. }
        | NodeOutcome::AwaitEvent { node_id, .. } => Value::String(node_id.clone()),
    }
}

/// Commit a parked continuation and its event registration in the same tuple.
async fn commit_suspension(
    ports: &ExecutionPorts,
    continuation: &Continuation,
    mut state: DriveState,
) -> Result<(ExecutionCommitResult, CommittedNativeModelUsageOutcome), ExecutionError> {
    let expected = ports
        .execution_commit
        .current_version(&continuation.program_instance_ref)
        .await;
    let terminal_non_success = state.has_terminal_non_success();
    if continuation.event_ref.is_none() && !terminal_non_success {
        state.seq += 1;
        state.batch.push(fact(
            &state.program_invocation_id,
            state.seq,
            FactKind::InvocationCommitted,
            None,
            Some(InvocationState::CommittedYield),
            None,
            Some(expected + 1),
        ));
    }
    let mut committed_continuation = continuation.clone();
    // Persist the observation high-water mark alongside the scheduler's
    // sequence lower bound. The continuation schema has one monotonic event
    // cursor; carrying the maximum prevents a resumed invocation from
    // reusing an observation position when the live stream had more boundary
    // records than evidence facts.
    committed_continuation.event_sequence = state.seq.max(state.observation_seq);
    committed_continuation
        .evidence_batch
        .clone_from(&state.batch);
    let payload = serde_json::to_value(&committed_continuation)
        .expect("continuation contains only serializable canonical runtime values");
    // The write-set digest is part of the continuation metadata. The canonical
    // digest helper masks that one self-referential field while hashing every
    // other continuation field, allowing the digest to be rebound here before
    // the payload and digest cross the single commit boundary.
    committed_continuation.write_set.continuation_digest = continuation_digest(Some(&payload));
    let event_wait = continuation.event_ref.as_ref().map(|event_ref| {
        serde_json::json!({
            "continuation_id": continuation.continuation_id,
            "event_ref": event_ref,
        })
    });
    let attempts = state.committed_model_attempts.clone();
    let lineages = state.committed_model_lineages.clone();
    let mut tuple = commit_tuple(&state, Some(payload), event_wait);
    bind_tuple_digests(&mut committed_continuation.write_set, &tuple);
    let payload = serde_json::to_value(&committed_continuation)
        .expect("continuation contains only serializable canonical runtime values");
    committed_continuation.write_set.continuation_digest = continuation_digest(Some(&payload));
    let payload = serde_json::to_value(&committed_continuation)
        .expect("continuation contains only serializable canonical runtime values");
    tuple.continuation = Some(payload);
    // One invocation may park more than once — a program that invokes two
    // host-fulfilled Capabilities parks at each. The commit scope is keyed by
    // this id, so a park that reused it would collide with the previous park
    // rather than commit. The parked node and the continuation's event
    // sequence are exactly the coordinates that separate two parks of one
    // invocation, and both are replayed identically, so this stays idempotent.
    let yield_commit_id = format!(
        "{}.yield.{}.{}",
        continuation.commit_id, continuation.continuation_id, committed_continuation.event_sequence
    );
    let request = ExecutionCommitRequest {
        commit_id: yield_commit_id.clone(),
        program_instance_ref: continuation.program_instance_ref.clone(),
        program_invocation_ref: continuation.program_invocation_ref.clone(),
        idempotency_key: format!("idem.{yield_commit_id}"),
        expected_program_state_version: expected,
        write_set: committed_continuation.write_set.clone(),
        tuple,
        evidence_batch: state.batch,
    };
    request
        .validate()
        .map_err(|error| ExecutionError::InvalidCommitRequest {
            message: error.to_string(),
        })?;
    let commit = ports.execution_commit.commit(request).await;
    let operational_usage = publish_committed_native_model_usage(
        ports,
        &yield_commit_id,
        &commit,
        &attempts,
        &lineages,
    )
    .await;
    Ok((commit, operational_usage))
}

/// Execute a canonical AIR program single-shot and commit its effects
/// atomically. A parked `await.event` is recorded and the walk continues; this
/// path never suspends.
///
/// # Errors
///
/// Returns [`ExecutionError`] if an operation is missing a required operand or a
/// model effect receives a mismatched admitted binding.
pub async fn execute(
    ports: &ExecutionPorts,
    request: ExecutionRequest,
    initial_context: Value,
) -> Result<RunReport, ExecutionError> {
    execute_with_resource_ceilings(ports, request, initial_context, None).await
}

/// Execute with the exact resource ceilings carried by a verified admission.
pub async fn execute_with_resource_ceilings(
    ports: &ExecutionPorts,
    request: ExecutionRequest,
    initial_context: Value,
    resource_ceilings: Option<&ResourceCeilings>,
) -> Result<RunReport, ExecutionError> {
    validate_execution_request(
        &request.air,
        &request.initial_values,
        &request.hook_bindings,
    )?;
    validate_commit_inputs(
        &request.program_instance_ref,
        &request.program_invocation_ref,
        &request.commit_id,
        &request.write_set,
    )?;
    let state = DriveState::new(
        initial_context,
        request.initial_values.clone(),
        &request.air,
        request.program_invocation_ref.as_str(),
    );
    let mut state = state;
    state.observe(
        ports,
        apxm_runtime_protocol::ObservationKind::InvocationStarted,
        apxm_runtime_protocol::Commitment::Provisional,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )?;
    let end = drive_from(
        DriveInputs {
            ports,
            air: &request.air,
            hook_bindings: &request.hook_bindings,
            model_admission: &request.model_admission,
            capability_invocations: &request.capability_invocations,
            resource_ceilings,
        },
        0,
        state,
        DriveOptions {
            suspend_on_park: false,
            yield_at_loop: false,
        },
    )
    .await?;
    let DriveEnd::RanToEnd(state) = end else {
        unreachable!("single-shot execute never suspends (suspend_on_park = false)");
    };
    commit_and_report(
        ports,
        &request.program_instance_ref,
        &request.program_invocation_ref,
        &request.commit_id,
        request.write_set,
        state,
    )
    .await
}

/// Execute a canonical AIR program with durable park/resume. It behaves exactly
/// like [`execute`] until it reaches a structural yield or parked `await.event`,
/// at which point it persists a [`Continuation`] through `continuation` and
/// returns [`RunOutcome::Suspended`] without completing the invocation. When it
/// runs to the end it commits atomically and returns [`RunOutcome::Completed`].
///
/// # Errors
///
/// Returns [`ExecutionError`] on a missing operand, a mismatched model binding,
/// or a continuation-store failure.
pub async fn execute_resumable(
    ports: &ExecutionPorts,
    request: ExecutionRequest,
    initial_context: Value,
) -> Result<RunOutcome, ExecutionError> {
    execute_resumable_with_resource_ceilings(ports, request, initial_context, None).await
}

/// Execute resumably with the exact resource ceilings carried by admission.
pub async fn execute_resumable_with_resource_ceilings(
    ports: &ExecutionPorts,
    request: ExecutionRequest,
    initial_context: Value,
    resource_ceilings: Option<&ResourceCeilings>,
) -> Result<RunOutcome, ExecutionError> {
    validate_execution_request(
        &request.air,
        &request.initial_values,
        &request.hook_bindings,
    )?;
    validate_commit_inputs(
        &request.program_instance_ref,
        &request.program_invocation_ref,
        &request.commit_id,
        &request.write_set,
    )?;
    let state = DriveState::new(
        initial_context,
        request.initial_values.clone(),
        &request.air,
        request.program_invocation_ref.as_str(),
    );
    let mut state = state;
    state.observe(
        ports,
        apxm_runtime_protocol::ObservationKind::InvocationStarted,
        apxm_runtime_protocol::Commitment::Provisional,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )?;
    let end = drive_from(
        DriveInputs {
            ports,
            air: &request.air,
            hook_bindings: &request.hook_bindings,
            model_admission: &request.model_admission,
            capability_invocations: &request.capability_invocations,
            resource_ceilings,
        },
        0,
        state,
        DriveOptions {
            suspend_on_park: true,
            yield_at_loop: true,
        },
    )
    .await?;
    finish(ports, request_parts(request), end).await
}

/// Resume a committed structural continuation for one Program Instance.
///
/// Event waits use [`wake_from_event_application`] so a raw delivered value
/// cannot advance a parked Invocation without a proven terminal application.
///
/// # Errors
///
/// Returns [`ExecutionError::Continuation`] when the atomic commit has no
/// continuation payload for `program_instance_ref`.
pub async fn resume(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    delivered: Value,
) -> Result<RunOutcome, ExecutionError> {
    resume_with_resource_ceilings(ports, program_instance_ref, delivered, None).await
}

/// Resume with the exact resource ceilings carried by admission.
pub async fn resume_with_resource_ceilings(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    delivered: Value,
    resource_ceilings: Option<&ResourceCeilings>,
) -> Result<RunOutcome, ExecutionError> {
    resume_from_continuation(
        ports,
        program_instance_ref,
        None,
        delivered,
        resource_ceilings,
    )
    .await
}

/// Wake a parked Event wait after a fulfilled Event application.
///
/// # Errors
///
/// Returns [`ExecutionError::UnprovenEventWake`] when the application is not
/// fulfilled, or [`ExecutionError::EventRefMismatch`] when the EventRef does
/// not match the parked wait.
pub async fn wake_from_event_application(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    event_ref: EventRef,
    application_result: EventApplicationResult,
    delivered: Value,
) -> Result<RunOutcome, ExecutionError> {
    if application_result != EventApplicationResult::Fulfilled {
        return Err(ExecutionError::UnprovenEventWake {
            program_instance_ref: program_instance_ref.clone(),
        });
    }
    resume_event(ports, program_instance_ref, event_ref, delivered).await
}

/// Internal runner entrypoint. Callers must already hold a fulfilled application.
pub(crate) async fn resume_event(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    event_ref: EventRef,
    delivered: Value,
) -> Result<RunOutcome, ExecutionError> {
    resume_from_continuation(
        ports,
        program_instance_ref,
        Some(event_ref),
        delivered,
        None,
    )
    .await
}

/// Wake an event continuation with the exact resource ceilings carried by admission.
pub(crate) async fn resume_event_with_resource_ceilings(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    event_ref: EventRef,
    delivered: Value,
    resource_ceilings: Option<&ResourceCeilings>,
) -> Result<RunOutcome, ExecutionError> {
    resume_from_continuation(
        ports,
        program_instance_ref,
        Some(event_ref),
        delivered,
        resource_ceilings,
    )
    .await
}

async fn resume_from_continuation(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    delivered_event_ref: Option<EventRef>,
    delivered: Value,
    resource_ceilings: Option<&ResourceCeilings>,
) -> Result<RunOutcome, ExecutionError> {
    let payload = ports
        .execution_commit
        .load_continuation_with_integrity(program_instance_ref)
        .await
        .ok_or_else(|| {
            ExecutionError::Continuation(ContinuationError::NotCommitted {
                program_instance_ref: program_instance_ref.clone(),
            })
        })?;
    verify_continuation_integrity(&payload)?;
    let parked: Continuation = serde_json::from_value(payload.payload).map_err(|error| {
        ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
            message: error.to_string(),
        })
    })?;

    let Continuation {
        air,
        hook_bindings,
        model_admission,
        capability_invocations,
        next_schedule_position,
        loop_frames,
        parked_node_execution_id,
        parked_loop_path,
        branch_decisions,
        resume_value_id,
        context,
        values,
        last_result,
        last_result_value_id,
        hook_target_snapshots,
        native_usage,
        external_agent_evidence,
        evidence_batch: _,
        event_sequence,
        program_invocation_ref,
        program_instance_ref: committed_program_instance_ref,
        commit_id,
        write_set,
        continuation_id,
        event_ref,
    } = parked;

    validate_commit_inputs(
        &committed_program_instance_ref,
        &program_invocation_ref,
        &commit_id,
        &write_set,
    )?;
    // The rehydrated bindings and the rehydrated AIR are separate persisted
    // fields, so resume checks the pairing the same way the entry paths do.
    validate_hook_bindings(&air, &hook_bindings)?;

    if committed_program_instance_ref != *program_instance_ref {
        return Err(ExecutionError::Continuation(
            ContinuationError::InstanceScopeMismatch {
                requested: program_instance_ref.clone(),
                committed: committed_program_instance_ref,
            },
        ));
    }

    match (event_ref.as_ref(), delivered_event_ref.as_ref()) {
        (Some(expected), Some(delivered_event_ref)) if expected == delivered_event_ref => {}
        (Some(expected), Some(delivered_event_ref)) => {
            return Err(ExecutionError::EventRefMismatch {
                expected: expected.clone(),
                delivered: delivered_event_ref.clone(),
            });
        }
        (Some(_), None) => {
            return Err(ExecutionError::EventDeliveryRequiresRef {
                program_instance_ref: program_instance_ref.clone(),
            });
        }
        (None, Some(delivered_event_ref)) => {
            return Err(ExecutionError::EventRefMismatch {
                expected: EventRef::new("structural.continuation")
                    .expect("fixed non-empty structural continuation identity"),
                delivered: delivered_event_ref.clone(),
            });
        }
        (None, None) => {}
    }

    let mut state = DriveState {
        node_outcomes: Vec::new(),
        native_usage,
        committed_model_attempts: Vec::new(),
        committed_model_lineages: Vec::new(),
        external_agent_evidence,
        context,
        last_result,
        values,
        last_result_value_id,
        hook_target_snapshots,
        branch_decisions,
        last_operation_succeeded: true,
        batch: Vec::new(),
        durable_observations: Vec::new(),
        pending_event_ref: None,
        pending_host_capability: None,
        // A resumed invocation may already have durable observations from an
        // earlier commit. The committed event sequence is a safe lower bound;
        // the next live position remains strictly increasing without changing
        // fact/node identities.
        observation_seq: event_sequence,
        seq: event_sequence,
        program_invocation_id: program_invocation_ref.as_str().to_string(),
        active_loops: loop_frames,
        last_model_node_execution_id: None,
        last_program_new_node_execution_id: None,
        committed_output_node_execution_id: None,
        committed_output_occurrence_id: None,
        committed_output_region_occurrence_id: None,
        current_node_execution_id: None,
        current_occurrence_id: None,
        current_region_occurrence_id: None,
        terminal_node_execution_id: None,
        terminal_occurrence_id: None,
        terminal_region_occurrence_id: None,
    };

    let resume_value_id = resume_value_id.ok_or_else(|| {
        ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
            message: "continuation is missing its exact resume SSA destination".into(),
        })
    })?;
    state.last_result = delivered.clone();
    state.last_result_value_id = Some(resume_value_id.clone());
    state.last_operation_succeeded = true;

    // A park at a host-fulfilled `capability.invoke` resumes as that node's
    // settlement, not as an `await.event` delivery: the parked node is a
    // capability node, and the payload is the host's settlement document
    // (ADR-0025). The AIR the continuation was rehydrated with is what says
    // which of the two this is.
    let host_capability_ref = air
        .semantic_operations
        .iter()
        .find(|operation| operation.node_id == continuation_id)
        .filter(|operation| operation.op == SemanticOpKind::CapabilityInvoke)
        .and_then(|operation| operand_str(operation, "capability_ref"))
        .filter(|capability_ref| is_host_capability_ref(capability_ref));

    let resumed_occurrence = format!("occurrence.{continuation_id}");
    let resumed_region_occurrence = parked_loop_path.last().and_then(|loop_id| {
        state
            .active_loops
            .iter()
            .find(|frame| frame.static_loop_id == *loop_id)
            .map(|frame| frame.dynamic_occurrence_id.clone())
    });

    let mut bound_value = delivered.clone();
    if let Some(capability_ref) = host_capability_ref.clone() {
        let capability_request_id = event_ref.as_ref().map_or_else(
            || host_capability_request_id(&state.program_invocation_id, &continuation_id),
            |reference| reference.as_str().to_owned(),
        );
        let settlement = host_settlement_from_event(
            &capability_request_id,
            &capability_ref,
            &EventOutcome::Fulfilled {
                event_ref: EventRef::new(capability_request_id.clone()).map_err(|_| {
                    ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
                        message: "parked host capability request has no identity".into(),
                    })
                })?,
                payload: match &delivered {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                },
            },
        );
        let parked_node_execution_id = parked_node_execution_id.clone().ok_or_else(|| {
            ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
                message: "host capability continuation is missing parked NodeExecution identity"
                    .into(),
            })
        })?;
        state.settle_host_capability(
            ports,
            &capability_ref,
            &settlement,
            &parked_node_execution_id,
            &resumed_occurrence,
            resumed_region_occurrence.as_deref(),
            Instant::now(),
        )?;
        let outcome = host_capability_outcome(&settlement);
        state.last_operation_succeeded =
            matches!(&outcome, CapabilityOutcome::Completed { .. });
        state.last_result = match &outcome {
            CapabilityOutcome::Completed { result } => Value::String(result.clone()),
            CapabilityOutcome::Failed { message }
            | CapabilityOutcome::OutcomeUnknown { message } => Value::String(message.clone()),
        };
        bound_value = state.last_result.clone();
        match &outcome {
            CapabilityOutcome::Completed { .. } => {
                state.committed_output_node_execution_id = Some(parked_node_execution_id.clone());
                state.committed_output_occurrence_id = Some(resumed_occurrence.clone());
                state
                    .committed_output_region_occurrence_id
                    .clone_from(&resumed_region_occurrence);
            }
            CapabilityOutcome::OutcomeUnknown { .. } => {
                state.append_effect_outcome_unknown(
                    &parked_node_execution_id,
                    &settlement.capability_request_id,
                );
            }
            CapabilityOutcome::Failed { message } => {
                state.append_invocation_failure(
                    &parked_node_execution_id,
                    unavailable_failure_envelope(
                        "capability",
                        &parked_node_execution_id,
                        message,
                    ),
                );
            }
        }
        state.node_outcomes.push(NodeOutcome::Capability {
            node_id: continuation_id.clone(),
            outcome,
            replaced: false,
        });
        for static_loop_id in &parked_loop_path {
            let frame = state
                .active_loops
                .iter_mut()
                .find(|frame| frame.static_loop_id == *static_loop_id)
                .ok_or_else(|| {
                    ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
                        message: format!("missing parked loop frame {static_loop_id}"),
                    })
                })?;
            frame.parked = false;
            frame
                .causal_node_execution_ids
                .push(parked_node_execution_id.clone());
        }
    } else if event_ref.is_some() {
        // A continuation without an event ref is a structural yield (for
        // example a loop-yield), not an event delivery. Do not mislabel its
        // resume as EventResumed; that observation requires a real event
        // identity.
        state.pending_event_ref = event_ref
            .as_ref()
            .map(|reference| reference.as_str().to_owned());
        state.observe(
            ports,
            apxm_runtime_protocol::ObservationKind::EventResumed,
            apxm_runtime_protocol::Commitment::Provisional,
            parked_node_execution_id.as_deref(),
            Some(&resumed_occurrence),
            None,
            resumed_region_occurrence.as_deref(),
            None,
            None,
            None,
            None,
        )?;
        let event_ref =
            event_ref
                .clone()
                .ok_or_else(|| ExecutionError::EventDeliveryRequiresRef {
                    program_instance_ref: program_instance_ref.clone(),
                })?;
        let payload = match &delivered {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        state.node_outcomes.push(NodeOutcome::AwaitEvent {
            node_id: continuation_id,
            outcome: EventOutcome::Fulfilled { event_ref, payload },
        });
        let parked_node_execution_id = parked_node_execution_id.ok_or_else(|| {
            ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
                message: "event continuation is missing parked NodeExecution identity".into(),
            })
        })?;
        for static_loop_id in &parked_loop_path {
            let frame = state
                .active_loops
                .iter_mut()
                .find(|frame| frame.static_loop_id == *static_loop_id)
                .ok_or_else(|| {
                    ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
                        message: format!("missing parked loop frame {static_loop_id}"),
                    })
                })?;
            frame.parked = false;
            frame
                .causal_node_execution_ids
                .push(parked_node_execution_id.clone());
        }
    }
    if host_capability_ref.is_none() {
        // A delivered event payload is external data the program never asked
        // for, so it may not become a Capability argument. A host capability
        // settlement is not that: it is the authored result of the node the
        // program itself invoked, and flows onward like any other result.
        validate_resume_capability_arguments(&air, &resume_value_id)?;
    }
    state.values.insert(resume_value_id, bound_value);

    let end = drive_from(
        DriveInputs {
            ports,
            air: &air,
            hook_bindings: &hook_bindings,
            model_admission: &model_admission,
            capability_invocations: &capability_invocations,
            resource_ceilings,
        },
        next_schedule_position,
        state,
        DriveOptions {
            suspend_on_park: true,
            yield_at_loop: true,
        },
    )
    .await?;
    let parts = CommitParts {
        air,
        hook_bindings,
        model_admission,
        capability_invocations,
        program_invocation_ref,
        program_instance_ref: committed_program_instance_ref,
        commit_id,
        write_set,
    };
    finish(ports, parts, end).await
}

/// The bare authored decision one permission carries, for the request the host
/// receives. The reason stays in AIR; the host is told what was asked for, not
/// why the author asked for it.
fn authored_permission(
    decision: &apxm_ais::permissions::PermissionDecision,
) -> apxm_runtime_protocol::AuthoredPermission {
    use apxm_ais::permissions::PermissionDecision;
    match decision {
        PermissionDecision::Allow { .. } => apxm_runtime_protocol::AuthoredPermission::Allow,
        PermissionDecision::Ask { .. } => apxm_runtime_protocol::AuthoredPermission::Ask,
        PermissionDecision::Deny { .. } => apxm_runtime_protocol::AuthoredPermission::Deny,
    }
}

/// How one host capability request settled, from the event the host applied.
///
/// A fulfilled event carries the host's own settlement document. Anything else
/// is APXM's own answer about a request the host never settled: parked without
/// durable suspension is a runtime that cannot wait, and expiry and
/// cancellation each settle the node rather than leaving it open.
fn host_settlement_from_event(
    capability_request_id: &str,
    capability_ref: &str,
    outcome: &EventOutcome,
) -> HostCapabilitySettlement {
    match outcome {
        EventOutcome::Fulfilled { payload, .. } => {
            match serde_json::from_str::<HostCapabilitySettlement>(payload) {
                Ok(settlement)
                    if settlement.is_well_formed()
                        && settlement.capability_request_id == capability_request_id =>
                {
                    settlement
                }
                _ => HostCapabilitySettlement::new(
                    capability_request_id,
                    HostCapabilityOutcomeKind::Failed,
                    None,
                    None,
                    Some(format!(
                        "the host settled '{capability_ref}' with a document outside \
                         apxm.host-capability.v1"
                    )),
                ),
            }
        }
        EventOutcome::Cancelled => HostCapabilitySettlement::new(
            capability_request_id,
            HostCapabilityOutcomeKind::Cancelled,
            None,
            None,
            Some(format!(
                "the invocation was cancelled while '{capability_ref}' was outstanding"
            )),
        ),
        EventOutcome::Expired => HostCapabilitySettlement::new(
            capability_request_id,
            HostCapabilityOutcomeKind::Unknown,
            None,
            None,
            Some(format!(
                "the request for '{capability_ref}' expired before the host settled it"
            )),
        ),
        EventOutcome::Parked | EventOutcome::Mismatched { .. } => HostCapabilitySettlement::new(
            capability_request_id,
            HostCapabilityOutcomeKind::Unknown,
            None,
            None,
            Some(format!(
                "'{capability_ref}' parked in an execution that cannot suspend, so no \
                 host answer can reach it"
            )),
        ),
    }
}

/// The program-visible outcome of one settlement.
///
/// `ok` hands the host's output to the program. `denied` and `failed` are typed
/// failures the program observes. `unknown` and `cancelled` leave the effect
/// uncertain, which is the honest state when a write may or may not have landed.
fn host_capability_outcome(settlement: &HostCapabilitySettlement) -> CapabilityOutcome {
    let message = || {
        settlement.message.clone().unwrap_or_else(|| {
            format!(
                "the host settled the request as {}",
                settlement.outcome.wire()
            )
        })
    };
    match settlement.outcome {
        HostCapabilityOutcomeKind::Ok => CapabilityOutcome::Completed {
            result: settlement.output.clone().unwrap_or_default(),
        },
        HostCapabilityOutcomeKind::Denied | HostCapabilityOutcomeKind::Failed => {
            CapabilityOutcome::Failed { message: message() }
        }
        HostCapabilityOutcomeKind::Unknown | HostCapabilityOutcomeKind::Cancelled => {
            CapabilityOutcome::OutcomeUnknown { message: message() }
        }
    }
}

fn verify_continuation_integrity(committed: &CommittedContinuation) -> Result<(), ExecutionError> {
    let expected = continuation_digest(Some(&committed.payload));
    if committed.digest != expected {
        return Err(ExecutionError::Continuation(
            ContinuationError::InvalidCommittedState {
                message: format!(
                    "continuation integrity mismatch: expected {expected}, got {}",
                    committed.digest
                ),
            },
        ));
    }
    Ok(())
}

/// The commit-scope parts carried from a request or a resumed continuation,
/// reused to build the next continuation on a re-park.
struct CommitParts {
    air: AirModule,
    hook_bindings: Vec<HookBinding>,
    model_admission: ModelBindingAdmission,
    capability_invocations: BTreeMap<String, CapabilityInvocationAdmission>,
    program_invocation_ref: ProgramInvocationRef,
    program_instance_ref: ProgramInstanceRef,
    commit_id: String,
    write_set: AtomicWriteSet,
}

fn request_parts(request: ExecutionRequest) -> CommitParts {
    CommitParts {
        air: request.air,
        hook_bindings: request.hook_bindings,
        model_admission: request.model_admission,
        capability_invocations: request.capability_invocations,
        program_invocation_ref: request.program_invocation_ref,
        program_instance_ref: request.program_instance_ref,
        commit_id: request.commit_id,
        write_set: request.write_set,
    }
}

/// Complete a resumable drive: commit when it ran to the end, or persist a fresh
/// continuation and report suspension when it parked.
async fn finish(
    ports: &ExecutionPorts,
    parts: CommitParts,
    end: DriveEnd,
) -> Result<RunOutcome, ExecutionError> {
    match end {
        DriveEnd::RanToEnd(state) => {
            let report = commit_and_report(
                ports,
                &parts.program_instance_ref,
                &parts.program_invocation_ref,
                &parts.commit_id,
                parts.write_set,
                state,
            )
            .await;
            Ok(RunOutcome::Completed(report?))
        }
        DriveEnd::Parked {
            state,
            continuation_id,
            event_ref,
            next_schedule_position,
            parked_node_execution_id,
            parked_loop_path,
            resume_value_id,
        } => {
            let cont = Continuation {
                air: parts.air,
                hook_bindings: parts.hook_bindings,
                model_admission: parts.model_admission,
                capability_invocations: parts.capability_invocations,
                next_schedule_position,
                loop_frames: state.active_loops.clone(),
                parked_node_execution_id,
                parked_loop_path,
                branch_decisions: state.branch_decisions.clone(),
                resume_value_id,
                context: state.context.clone(),
                values: state.values.clone(),
                last_result: state.last_result.clone(),
                last_result_value_id: state.last_result_value_id.clone(),
                hook_target_snapshots: state.hook_target_snapshots.clone(),
                native_usage: state.native_usage,
                external_agent_evidence: state.external_agent_evidence.clone(),
                evidence_batch: state.batch.clone(),
                event_sequence: state.seq,
                program_invocation_ref: parts.program_invocation_ref,
                program_instance_ref: parts.program_instance_ref,
                commit_id: parts.commit_id,
                write_set: parts.write_set,
                continuation_id: continuation_id.clone(),
                event_ref,
            };
            let (commit, operational_usage) = commit_suspension(ports, &cont, state).await?;
            match commit {
                ExecutionCommitResult::Committed { .. } => Ok(RunOutcome::Suspended {
                    continuation_id,
                    event_ref: cont.event_ref,
                    operational_usage,
                }),
                result => Err(ExecutionError::Commit(result)),
            }
        }
    }
}

#[cfg(test)]
mod loop_evidence_tests {
    use super::*;
    use serde_json::json;

    fn predicate_air(expression: ValueExpression) -> AirModule {
        AirModule {
            schema_version: apxm_program::air::AirVersion::V2,
            value_assemblies: vec![apxm_program::air::ValueAssembly {
                value_id: "value.predicate".into(),
                expression,
            }],
            semantic_operations: Vec::new(),
            structural_ir: Vec::new(),
            context_flow: Vec::new(),
            capability_permission_requests: Default::default(),
            source_map: apxm_program::source_map::SourceMap {
                schema_version: apxm_program::source_map::SourceMapVersion::V1,
                source_language: apxm_program::source_map::SourceLanguage::Python,
                node_spans: Vec::new(),
                region_annotations: Vec::new(),
                region_spans: Vec::new(),
                edge_spans: Vec::new(),
            },
        }
    }

    #[test]
    fn continuation_integrity_rejects_tampering_before_typed_decode() {
        let original = serde_json::json!({"context": {"counter": 1}});
        let committed = CommittedContinuation {
            payload: serde_json::json!({"context": {"counter": 2}}),
            digest: continuation_digest(Some(&original)),
        };
        assert!(matches!(
            verify_continuation_integrity(&committed),
            Err(ExecutionError::Continuation(
                ContinuationError::InvalidCommittedState { message }
            )) if message.contains("integrity mismatch")
        ));
    }

    fn assembled_predicate() -> ControlPredicate {
        ControlPredicate {
            root_value_id: "value.predicate".into(),
            property_path: vec!["kind".into()],
            comparator: PredicateComparator::Equals,
            literal: Some(PredicateLiteral::String("final".into())),
        }
    }

    fn loop_air() -> AirModule {
        serde_json::from_value(json!({
            "schema_version": "apxm.air",
            "semantic_operations": [],
            "structural_ir": [
                {
                    "region_id": "region.loop.main",
                    "kind": "ais.loop",
                    "execution_order": 0
                }
            ],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": [
                    {"region_id": "region.loop.main", "annotation": "structural_loop"}
                ]
            }
        }))
        .expect("generic loop AIR")
    }

    #[test]
    fn completed_back_edge_adds_typed_fact_to_atomic_commit_tuple() {
        let mut state = DriveState::new(Value::Null, BTreeMap::new(), &loop_air(), "invocation.1");
        state.enter_loop("region.loop.main");
        state.seq += 1;
        let occurrence = state
            .active_loops
            .iter()
            .find(|frame| frame.static_loop_id == "region.loop.main")
            .unwrap()
            .dynamic_occurrence_id
            .clone();
        state
            .batch
            .push(Fact::NodeExecutionRecorded(NodeExecutionRecordedFact {
                fact_id: format!("fact.{}", state.seq),
                event_sequence: state.seq,
                program_invocation_id: "invocation.1".into(),
                node_execution_id: "node-execution.1".into(),
                air_node_id: "node.1".into(),
                parent_node_execution_id: None,
                execution_scope: NodeExecutionScope::Loop {
                    region_occurrence_id: occurrence.clone(),
                    static_region_id: "region.loop.main".into(),
                    loop_memberships: vec![LoopMembership {
                        static_loop_id: "region.loop.main".into(),
                        loop_occurrence_id: occurrence,
                    }],
                },
            }));
        state.record_node_outcome(&["region.loop.main".into()], "node-execution.1", true);

        assert!(state.complete_loop_iteration("region.loop.main"));
        let tuple = commit_tuple(&state, None, None);
        let completed = tuple
            .evidence
            .iter()
            .find_map(Fact::loop_iteration_completed)
            .expect("typed completion shares the atomic tuple");
        assert_eq!(completed.static_loop_id, "region.loop.main");
        assert_eq!(
            completed.loop_occurrence_id,
            "loop-occurrence.region.loop.main.invocation.1"
        );
        assert_eq!(completed.iteration_index, 0);
        assert_eq!(completed.program_invocation_id, "invocation.1");
        assert_eq!(
            completed.causal_node_execution_ids,
            ["node-execution.1".to_string()]
        );
    }

    #[test]
    fn earlier_failed_body_operation_is_sticky_after_later_success() {
        let mut state = DriveState::new(Value::Null, BTreeMap::new(), &loop_air(), "invocation.1");
        state.enter_loop("region.loop.main");
        let loop_path = ["region.loop.main".to_string()];
        state.record_node_outcome(&loop_path, "node-execution.failed", false);
        state.record_node_outcome(&loop_path, "node-execution.succeeded", true);

        assert!(!state.complete_loop_iteration("region.loop.main"));
        assert!(
            state
                .batch
                .iter()
                .all(|fact| fact.loop_iteration_completed().is_none())
        );
    }

    #[test]
    fn resume_input_dependency_is_rejected_before_capability_dispatch() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air",
            "semantic_operations": [{
                "node_id": "node.capability",
                "op": "capability.invoke",
                "parent_region_id": "region.root",
                "execution_order": 1,
                "operands": [
                    {"slot": "capability_ref", "value_id": "cap.finish", "type_ref": "CapabilityRef"},
                    {"slot": "arguments", "value_id": "value.cap.arguments", "type_ref": "CapabilityArguments"}
                ],
                "result": {"value_id": "value.capability.output", "type_ref": "CapabilityOutput"}
            }],
            "value_assemblies": [{
                "value_id": "value.cap.arguments",
                "expression": {"kind": "projection", "root": {"kind": "ssa", "value_id": "value.resume.input"}, "property_path": ["arguments"]}
            }],
            "structural_ir": [],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("resume dependency AIR");

        let error = validate_resume_capability_arguments(&air, "value.resume.input")
            .expect_err("resume-derived capability argument must fail closed");
        assert!(matches!(error, ExecutionError::InvalidAir { .. }));
    }

    #[test]
    fn predicate_materializes_value_assemblies_and_resume_values_deterministically() {
        let air = predicate_air(ValueExpression::Object {
            fields: vec![apxm_program::frontend_graph::ValueField {
                name: "kind".into(),
                value: ValueExpression::String {
                    value: "final".into(),
                },
            }],
        });
        let state = DriveState::new(Value::Null, BTreeMap::new(), &air, "invocation.1");
        let predicate = assembled_predicate();
        assert!(evaluate_predicate(&air, &state, "region.branch", &predicate).unwrap());
        assert!(evaluate_predicate(&air, &state, "region.branch", &predicate).unwrap());

        let resume_air = predicate_air(ValueExpression::Projection {
            root: Box::new(ValueExpression::Ssa {
                value_id: "value.resume".into(),
            }),
            property_path: vec!["payload".into()],
        });
        let mut resumed =
            DriveState::new(Value::Null, BTreeMap::new(), &resume_air, "invocation.1");
        resumed
            .values
            .insert("value.resume".into(), json!({"payload": {"kind": "final"}}));
        assert!(
            evaluate_predicate(
                &resume_air,
                &resumed,
                "region.branch",
                &assembled_predicate()
            )
            .unwrap()
        );
    }

    #[test]
    fn predicate_assembly_failures_are_closed() {
        let missing = predicate_air(ValueExpression::Ssa {
            value_id: "value.missing".into(),
        });
        let state = DriveState::new(Value::Null, BTreeMap::new(), &missing, "invocation.1");
        assert!(matches!(
            evaluate_predicate(&missing, &state, "region.branch", &assembled_predicate()),
            Err(ExecutionError::MissingControlValue { .. })
        ));

        let cycle = predicate_air(ValueExpression::Ssa {
            value_id: "value.predicate".into(),
        });
        assert!(matches!(
            evaluate_predicate(&cycle, &state, "region.branch", &assembled_predicate()),
            Err(ExecutionError::InvalidControlPredicate { .. })
        ));

        let mut nested = ValueExpression::Boolean { value: true };
        for _ in 0..65 {
            nested = ValueExpression::Projection {
                root: Box::new(nested),
                property_path: vec!["kind".into()],
            };
        }
        let too_deep = predicate_air(nested);
        assert!(matches!(
            evaluate_predicate(&too_deep, &state, "region.branch", &assembled_predicate()),
            Err(ExecutionError::InvalidValueExpression { .. })
        ));

        let wrong_type = predicate_air(ValueExpression::Object {
            fields: vec![apxm_program::frontend_graph::ValueField {
                name: "kind".into(),
                value: ValueExpression::Object { fields: Vec::new() },
            }],
        });
        assert!(matches!(
            evaluate_predicate(&wrong_type, &state, "region.branch", &assembled_predicate()),
            Err(ExecutionError::InvalidControlPredicate { .. })
        ));
    }

    #[test]
    fn admitted_memory_and_effect_ceilings_fail_closed() {
        let state = DriveState::new(
            json!({"payload": "x".repeat(128)}),
            BTreeMap::new(),
            &predicate_air(ValueExpression::Null),
            "invocation.1",
        );
        let ceilings = ResourceCeilings {
            max_wall_ms: 1,
            max_memory_bytes: 1,
            max_effect_bytes: 1,
        };
        assert!(matches!(
            enforce_runtime_limits(&state, Some(&ceilings)),
            Err(ExecutionError::ResourceLimitExceeded {
                resource: "memory_bytes",
                ..
            })
        ));
        let effect_only = ResourceCeilings {
            max_wall_ms: 1,
            max_memory_bytes: u64::MAX,
            max_effect_bytes: 1,
        };
        assert!(matches!(
            enforce_runtime_limits(&state, Some(&effect_only)),
            Err(ExecutionError::ResourceLimitExceeded {
                resource: "effect_bytes",
                ..
            })
        ));
    }

    #[test]
    fn structural_input_count_is_bounded_before_air_verification() {
        let air = predicate_air(ValueExpression::Null);
        let initial_values = (0..=MAX_INITIAL_VALUES)
            .map(|index| (format!("value.{index}"), Value::Null))
            .collect();
        assert!(matches!(
            validate_execution_request(&air, &initial_values, &[]),
            Err(ExecutionError::InvalidAir { message }) if message.contains("initial_values")
        ));
    }
}
