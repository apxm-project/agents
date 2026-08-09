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
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use apxm_inference::{
    BindingError, CommittedInferenceDispatch, InferenceTargetCommitment, InferenceUsageLineage,
    ModelBindingAdmission, ModelCallPreparation, ModelCallRequest, ModelCallRequestError,
    ModelCallRequestMetadataPort, ModelInferencePort, ModelOutcome, ModelTargetRef, RetryPolicy,
    TargetCommitmentError, TypedError, Usage, dispatch_committed_inference,
};
use apxm_kernel::{
    AtomicWriteSet, ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    ExecutionCommitTuple, PortSlot, ProgramInstanceRef, ProgramInvocationRef,
};
use apxm_program::air::{
    AirModule, ControlPredicate, PredicateComparator, PredicateLiteral, SemanticOp, SemanticOpKind,
};
use apxm_program::capability::{CapabilityInvocationAuthority, CapabilityRequestError};
use apxm_program::common::TypedRef;
use apxm_program::external_agent::ExternalAgentEvidence;
use apxm_program::frontend_graph::HookBinding;
use apxm_program::frontend_graph::ValueExpression;
use apxm_program::runtime_evidence::{
    Fact, FactKind, HookPhase as EvidenceHookPhase, HookScope as EvidenceHookScope, InstanceState,
    InvocationState, LoopIterationCompletedFact, LoopMembership, ModelAttemptRecordedFact,
    NodeExecutionRecordedFact, NodeExecutionScope, RuntimeFact,
};

use crate::ExecutionPortBundle;
use crate::operational_usage::{
    CommittedNativeModelUsage, CommittedNativeModelUsageError, CommittedNativeModelUsageOutcome,
    CommittedNativeModelUsagePort, EvidencePositionRef, EvidencePositionRefType,
};
use crate::ports::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionReceiver, CompositionRequest, EventAwait, EventOutcome, EventPort, EventRef,
    EventRefError,
};
use crate::resume::{Continuation, ContinuationError, DurableLoopFrame, RunOutcome};
use crate::structural::{ScheduleStep, build_schedule};

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
        })
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

/// Execute the exact digest-pinned handler named by a compiled Hook binding.
///
/// The execution port receives the artifact binding directly. It cannot discover
/// a Hook by event name or register callbacks dynamically.
#[async_trait]
pub trait StaticHookHandlerPort: Send + Sync {
    async fn execute(
        &self,
        binding: &HookBinding,
        context: &Value,
        result: &Value,
    ) -> Result<StaticHookResult, StaticHookExecutionError>;
}

/// A no-op handler implementation for programs with no static Hook bindings.
pub struct NoopStaticHookHandler;

#[async_trait]
impl StaticHookHandlerPort for NoopStaticHookHandler {
    async fn execute(
        &self,
        _binding: &HookBinding,
        _context: &Value,
        _result: &Value,
    ) -> Result<StaticHookResult, StaticHookExecutionError> {
        Err(StaticHookExecutionError {
            hook_id: _binding.hook_id.clone(),
            message: "no static Hook executor is installed".to_string(),
        })
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
    /// The outcome of publishing the exact committed native model attempts.
    /// External-agent/ACP usage never enters this Agents-owned contract.
    pub operational_usage: CommittedNativeModelUsageOutcome,
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
    InvalidCommitRequest {
        message: String,
    },
    Commit(ExecutionCommitResult),
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
            Self::InvalidCommitRequest { message } => {
                write!(f, "invalid atomic commit request: {message}")
            }
            Self::Commit(result) => write!(f, "atomic execution commit failed: {}", result.label()),
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
            typed_error: None,
        },
    )
}

fn runtime_fact_mut(fact: &mut Fact) -> &mut RuntimeFact {
    fact.runtime_mut()
        .expect("runtime helper constructed a non-loop fact")
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
    branch_decisions: BTreeMap<String, usize>,
    last_operation_succeeded: bool,
    batch: Vec<Fact>,
    seq: u64,
    program_invocation_id: String,
    active_loops: Vec<DurableLoopFrame>,
    last_model_node_execution_id: Option<String>,
    last_program_new_node_execution_id: Option<String>,
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
    hasher.update(b"apxm.model-effect.v1\0");
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
        schema_version: "apxm.model-request-identity.v1",
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
            branch_decisions: BTreeMap::new(),
            last_operation_succeeded: true,
            batch,
            seq,
            program_invocation_id: program_invocation_id.to_string(),
            active_loops: Vec::new(),
            last_model_node_execution_id: None,
            last_program_new_node_execution_id: None,
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
        self.batch.push(join_fact(
            &self.program_invocation_id,
            self.seq,
            FactKind::RegionOccurrenceStarted,
            Some(occurrence.clone()),
            Some(static_loop_id.to_string()),
            None,
            None,
            None,
        ));
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

fn join_fact(
    program_invocation_id: &str,
    seq: u64,
    kind: FactKind,
    region_occurrence_id: Option<String>,
    static_region_id: Option<String>,
    node_execution_id: Option<String>,
    air_node_id: Option<String>,
    parent_node_execution_id: Option<String>,
) -> Fact {
    let mut fact = fact(program_invocation_id, seq, kind, None, None, None, None);
    let runtime = runtime_fact_mut(&mut fact);
    runtime.region_occurrence_id = region_occurrence_id;
    runtime.static_region_id = static_region_id;
    runtime.node_execution_id = node_execution_id;
    runtime.air_node_id = air_node_id;
    runtime.parent_node_execution_id = parent_node_execution_id;
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
    if depth > 64 {
        return Err(ExecutionError::InvalidValueExpression {
            value_id: owner.to_string(),
            message: "expression depth exceeds 64".to_string(),
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
                value == &expected
            } else {
                value != &expected
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

/// Walk the structural execution schedule from `start_index`, dispatching semantic
/// operations to their exact injected ports and threading Context through Hooks.
/// When `suspend_on_park` is set, a parked `await.event` or loop yield stops the
/// walk and returns [`DriveEnd::Parked`]; otherwise a parked outcome is recorded
/// and the walk continues (the single-shot contract).
async fn drive_from(
    ports: &ExecutionPorts,
    air: &AirModule,
    hook_bindings: &[HookBinding],
    model_admission: &ModelBindingAdmission,
    capability_invocations: &BTreeMap<String, CapabilityInvocationAdmission>,
    start_schedule_position: usize,
    mut state: DriveState,
    options: DriveOptions,
) -> Result<DriveEnd, ExecutionError> {
    let schedule = build_schedule(air, hook_bindings);
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
        let step = &schedule[schedule_position];
        match step {
            ScheduleStep::HookBefore { binding } => {
                let (before, after) = apply_static_hook(&mut state, ports, binding).await?;
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
                if state.last_operation_succeeded {
                    let (before, after) = apply_static_hook(&mut state, ports, binding).await?;
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
                    state.exit_loop(static_loop_id);
                    schedule_position =
                        matching_loop_back_edge(&schedule, schedule_position, static_loop_id)? + 1;
                    continue;
                }
                state.enter_loop(static_loop_id);
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
                        let committed_dispatch =
                            dispatch_committed_inference(CommittedInferenceDispatch {
                                target_commitment: &target_commitment,
                                authored_target: &authored_target,
                                request: &call,
                                backend: &*ports.model_inference,
                                duration_ms: dispatch_started.elapsed().as_millis() as u64,
                                policy: RetryPolicy::default(),
                            })
                            .map_err(|error| match error {
                                apxm_inference::InferenceDispatchError::TargetCommitment(error) => {
                                    ExecutionError::TargetCommitment(error)
                                }
                                apxm_inference::InferenceDispatchError::Lineage(error) => {
                                    ExecutionError::Lineage(error)
                                }
                                apxm_inference::InferenceDispatchError::Driver(error) => {
                                    unreachable!(
                                        "production dispatch does not use a driver wrapper: {error}"
                                    )
                                }
                                apxm_inference::InferenceDispatchError::Lease(error) => {
                                    unreachable!(
                                        "production dispatch does not use a lease: {error}"
                                    )
                                }
                            })?;
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
                        state.last_result = result.clone();
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
                            if admission.capability_ref != capability_ref {
                                return Err(
                                    ExecutionError::CapabilityInvocationAdmissionMismatch {
                                        node_id: op.node_id.clone(),
                                        authored: capability_ref,
                                        admitted: admission.capability_ref.clone(),
                                    },
                                );
                            }
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
                            let outcome = ports
                                .capability
                                .invoke(
                                    CapabilityRequest::prepare(
                                        capability_ref,
                                        arguments_type_ref,
                                        authored_arguments,
                                        &state.program_invocation_id,
                                        &node_execution_id,
                                        admission.authority.clone(),
                                    )
                                    .map_err(ExecutionError::CapabilityRequest)?,
                                )
                                .await;
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
                            state.node_outcomes.push(NodeOutcome::Capability {
                                node_id: op.node_id.clone(),
                                outcome,
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
                        state.last_result = Value::String(match &outcome {
                            CompositionOutcome::Created { child_instance_ref }
                            | CompositionOutcome::Invoked { child_instance_ref } => {
                                child_instance_ref.clone()
                            }
                            CompositionOutcome::Failed { message } => message.clone(),
                        });
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
                        state.last_result = Value::String(match &outcome {
                            CompositionOutcome::Created { child_instance_ref }
                            | CompositionOutcome::Invoked { child_instance_ref } => {
                                child_instance_ref.clone()
                            }
                            CompositionOutcome::Failed { message } => message.clone(),
                        });
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
                            attached_fact.parent_node_execution_id =
                                state.last_program_new_node_execution_id.clone();
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
                        let outcome = ports
                            .events
                            .await_event(EventAwait {
                                node_id: op.node_id.clone(),
                                event_ref: event_ref.clone(),
                            })
                            .await;
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
                                Some(InvocationState::CommittedYield),
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
                state.complete_loop_iteration(static_loop_id);
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
                state.fail_active_loops();
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
    binding: &HookBinding,
) -> Result<(Value, Value), ExecutionError> {
    let before = state.context.clone();
    let outcome = ports
        .hook_handlers
        .execute(binding, &state.context, &state.last_result)
        .await
        .map_err(ExecutionError::StaticHook)?;
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
            if let Some(value_id) = &state.last_result_value_id {
                state.values.insert(value_id.clone(), result.clone());
            }
            if let Some(NodeOutcome::Model {
                result: model_result,
                replaced,
                ..
            }) = state.node_outcomes.last_mut()
            {
                *model_result = result;
                *replaced = true;
            }
        }
    }
    Ok((before, state.context.clone()))
}

/// Perform the one atomic commit for a completed run and build its report. The
/// invocation-committed fact is appended just before the commit, exactly as the
/// single-shot path has always done.
async fn commit_and_report(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    program_invocation_ref: &ProgramInvocationRef,
    commit_id: &str,
    write_set: AtomicWriteSet,
    mut state: DriveState,
) -> Result<RunReport, ExecutionError> {
    let expected = ports
        .execution_commit
        .current_version(program_instance_ref)
        .await;
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

    let request = ExecutionCommitRequest {
        commit_id: commit_id.to_string(),
        program_instance_ref: program_instance_ref.clone(),
        program_invocation_ref: program_invocation_ref.clone(),
        idempotency_key: format!("idem.{commit_id}"),
        expected_program_state_version: expected,
        write_set,
        tuple: commit_tuple(&state, None, None),
        evidence_batch: state.batch.clone(),
    };
    request
        .validate()
        .map_err(|error| ExecutionError::InvalidCommitRequest {
            message: error.to_string(),
        })?;
    let commit = ports.execution_commit.commit(request).await;

    let operational_usage = publish_committed_native_model_usage(
        ports,
        commit_id,
        &commit,
        &state.committed_model_attempts,
        &state.committed_model_lineages,
    )
    .await;

    Ok(RunReport {
        node_outcomes: state.node_outcomes,
        native_usage: state.native_usage,
        external_agent_evidence: state.external_agent_evidence,
        final_context: state.context,
        commit,
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
        let usage = match CommittedNativeModelUsage::from_lineage(
            commit_id.to_string(),
            evidence_position_ref.clone(),
            attempt.clone(),
            &lineage,
        ) {
            Ok(usage) => usage,
            Err(_) => {
                return CommittedNativeModelUsageOutcome::Failed(
                    CommittedNativeModelUsageError::Rejected,
                );
            }
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
    let request = ExecutionCommitRequest {
        commit_id: commit_id.to_string(),
        program_instance_ref: program_instance_ref.clone(),
        program_invocation_ref: program_invocation_ref.clone(),
        idempotency_key: format!("idem.{commit_id}"),
        expected_program_state_version: 0,
        write_set: write_set.clone(),
        tuple: ExecutionCommitTuple::empty(Vec::new()),
        evidence_batch: Vec::new(),
    };
    request
        .validate()
        .map_err(|error| ExecutionError::InvalidCommitRequest {
            message: error.to_string(),
        })
}

fn validate_execution_request(
    air: &AirModule,
    initial_values: &BTreeMap<String, Value>,
) -> Result<(), ExecutionError> {
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
            collect_runtime_expression_references(root, references)
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
    }
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
    let mut committed_continuation = continuation.clone();
    committed_continuation.event_sequence = state.seq;
    committed_continuation.evidence_batch = state.batch.clone();
    let payload = serde_json::to_value(&committed_continuation)
        .expect("continuation contains only serializable canonical runtime values");
    let event_wait = continuation.event_ref.as_ref().map(|event_ref| {
        serde_json::json!({
            "continuation_id": continuation.continuation_id,
            "event_ref": event_ref,
        })
    });
    let attempts = state.committed_model_attempts.clone();
    let lineages = state.committed_model_lineages.clone();
    let request = ExecutionCommitRequest {
        commit_id: format!("{}.yield", continuation.commit_id),
        program_instance_ref: continuation.program_instance_ref.clone(),
        program_invocation_ref: continuation.program_invocation_ref.clone(),
        idempotency_key: format!("idem.{}.yield", continuation.commit_id),
        expected_program_state_version: expected,
        write_set: continuation.write_set.clone(),
        tuple: commit_tuple(&state, Some(payload), event_wait),
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
        &format!("{}.yield", continuation.commit_id),
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
    validate_execution_request(&request.air, &request.initial_values)?;
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
    let end = drive_from(
        ports,
        &request.air,
        &request.hook_bindings,
        &request.model_admission,
        &request.capability_invocations,
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
    validate_execution_request(&request.air, &request.initial_values)?;
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
    let end = drive_from(
        ports,
        &request.air,
        &request.hook_bindings,
        &request.model_admission,
        &request.capability_invocations,
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
/// Event continuations use [`resume_event`] so their durable identity is
/// validated before the runtime advances the parked continuation.
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
    resume_from_continuation(ports, program_instance_ref, None, delivered).await
}

/// Resume a committed `await.event` continuation with one exact EventRef.
///
/// # Errors
///
/// Returns [`ExecutionError::EventRefMismatch`] when an OS delivery targets a
/// different durable event than the one atomically registered at park time.
pub async fn resume_event(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    event_ref: EventRef,
    delivered: Value,
) -> Result<RunOutcome, ExecutionError> {
    resume_from_continuation(ports, program_instance_ref, Some(event_ref), delivered).await
}

async fn resume_from_continuation(
    ports: &ExecutionPorts,
    program_instance_ref: &ProgramInstanceRef,
    delivered_event_ref: Option<EventRef>,
    delivered: Value,
) -> Result<RunOutcome, ExecutionError> {
    let payload = ports
        .execution_commit
        .load_continuation(program_instance_ref)
        .await
        .ok_or_else(|| {
            ExecutionError::Continuation(ContinuationError::NotCommitted {
                program_instance_ref: program_instance_ref.clone(),
            })
        })?;
    let parked: Continuation = serde_json::from_value(payload).map_err(|error| {
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
        branch_decisions,
        last_operation_succeeded: true,
        batch: Vec::new(),
        seq: event_sequence,
        program_invocation_id: program_invocation_ref.as_str().to_string(),
        active_loops: loop_frames,
        last_model_node_execution_id: None,
        last_program_new_node_execution_id: None,
    };

    if event_ref.is_some() {
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
    let resume_value_id = resume_value_id.ok_or_else(|| {
        ExecutionError::Continuation(ContinuationError::InvalidCommittedState {
            message: "continuation is missing its exact resume SSA destination".into(),
        })
    })?;
    validate_resume_capability_arguments(&air, &resume_value_id)?;
    state.values.insert(resume_value_id, delivered);

    let end = drive_from(
        ports,
        &air,
        &hook_bindings,
        &model_admission,
        &capability_invocations,
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

    fn loop_air() -> AirModule {
        serde_json::from_value(json!({
            "schema_version": "apxm.air.v2",
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
                "schema_version": "apxm.source-map.v1",
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
            "schema_version": "apxm.air.v2",
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
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("resume dependency AIR");

        let error = validate_resume_capability_arguments(&air, "value.resume.input")
            .expect_err("resume-derived capability argument must fail closed");
        assert!(matches!(error, ExecutionError::InvalidAir { .. }));
    }
}
