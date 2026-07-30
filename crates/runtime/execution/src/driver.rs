//! The canonical runtime driver: execute a five-operation AIR node graph through
//! injected kernel ports and commit atomically.
//!
//! The driver derives an execution schedule from structural AIR when present,
//! interleaving compiled Hook callsites and explicit Context commits around the
//! authored semantic operation order. It dispatches each semantic operation to
//! its exact injected port — `model.call` to the model inference port,
//! `capability.invoke` to the External Agent port or the Capability port,
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

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use apxm_inference::{
    BindingError, ModelBindingAdmission, ModelCallPreparation, ModelCallRequest,
    ModelCallRequestError, ModelCallRequestMetadataPort, ModelInferencePort, ModelOutcome,
    ModelTargetRef, RetryPolicy, TypedError, Usage, execute_with_attempt as run_model,
};
use apxm_kernel::{
    AcpPromptRequest, AtomicWriteSet, ExecutionCommitPort, ExecutionCommitRequest,
    ExecutionCommitResult, ExecutionCommitTuple, ExternalAgentCapabilityPort, PortBundle, PortSlot,
    ProgramInstanceRef, ProgramInvocationRef, assemble_evidence,
};
use apxm_program::air::{AirModule, SemanticOp, SemanticOpKind};
use apxm_program::capability::{CapabilityInvocationAuthority, CapabilityRequestError};
use apxm_program::common::TypedRef;
use apxm_program::external_agent::ExternalAgentEvidence;
use apxm_program::frontend_graph::HookBinding;
use apxm_program::runtime_evidence::{
    Fact, FactKind, HookPhase as EvidenceHookPhase, HookScope as EvidenceHookScope, InstanceState,
    InvocationState, LoopIterationCompletedFact, LoopMembership, ModelAttemptRecordedFact,
    NodeExecutionRecordedFact, NodeExecutionScope, RuntimeFact,
};

use crate::operational_usage::{
    CommittedNativeModelUsage, CommittedNativeModelUsageOutcome, CommittedNativeModelUsagePort,
    CommittedNativeModelUsageVersion, EvidencePositionRef, EvidencePositionRefType,
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
    external_agent: Arc<dyn ExternalAgentCapabilityPort>,
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
    /// Construct the driver ports from one validated kernel bundle.
    ///
    /// Ordinary capability, model, external-agent, and commit effects are
    /// copied only from their exact admitted slots. The remaining driver-local
    /// ports do not select an implementation for those effects.
    pub fn from_admitted_bundle(
        bundle: &PortBundle,
        model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
        events: Arc<dyn EventPort>,
        composition: Arc<dyn CompositionPort>,
        hook_handlers: Arc<dyn StaticHookHandlerPort>,
    ) -> Result<Self, ExecutionPortsError> {
        let model_inference =
            bundle
                .model_inference()
                .cloned()
                .ok_or(ExecutionPortsError::MissingAdmittedPort(
                    PortSlot::ModelInference,
                ))?;
        let capability =
            bundle
                .capability()
                .cloned()
                .ok_or(ExecutionPortsError::MissingAdmittedPort(
                    PortSlot::Capability,
                ))?;
        let external_agent = bundle.external_agent_capability().cloned().ok_or(
            ExecutionPortsError::MissingAdmittedPort(PortSlot::ExternalAgentCapability),
        )?;

        Ok(Self {
            model_inference,
            model_call_request_metadata,
            capability,
            external_agent,
            events,
            composition,
            execution_commit: bundle.execution_commit().clone(),
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
    ) -> StaticHookResult;
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
    ) -> StaticHookResult {
        StaticHookResult::Keep {
            assigned_context: None,
        }
    }
}

/// One canonical execution request: the AIR to run, the materialized
/// model-binding admission, per-node Capability invocation admissions,
/// immutable instance and invocation identities, and prepared write set.
pub struct ExecutionRequest {
    pub air: AirModule,
    pub hook_bindings: Vec<HookBinding>,
    pub model_admission: ModelBindingAdmission,
    pub capability_invocations: BTreeMap<String, CapabilityInvocationAdmission>,
    pub program_instance_ref: ProgramInstanceRef,
    pub program_invocation_ref: ProgramInvocationRef,
    pub commit_id: String,
    pub write_set: AtomicWriteSet,
}

/// Application data and admitted Auth/Server references for one exact
/// `capability.invoke` AIR node.
///
/// Arguments remain application data. The authority references are carried in
/// a separate typed member and cannot be inferred from or replaced by argument
/// fields. `capability_ref` is repeated so the driver can reject an admission
/// prepared for a different authored Capability before dispatch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityInvocationAdmission {
    pub capability_ref: String,
    pub arguments: Value,
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
    MissingOperand {
        node_id: String,
        operand: &'static str,
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
    ModelRequest(ModelCallRequestError),
    ModelRequestMetadata(TypedError),
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
    Commit(ExecutionCommitResult),
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingOperand { node_id, operand } => {
                write!(f, "node {node_id} is missing operand {operand}")
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
            Self::ModelRequest(error) => write!(f, "model request error: {error}"),
            Self::ModelRequestMetadata(error) => {
                write!(f, "model request metadata error: {}", error.message)
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

fn model_result_value(outcome: &ModelOutcome) -> Value {
    match outcome {
        ModelOutcome::CommittedSuccess { .. } => Value::String("model.result".to_string()),
        _ => Value::Null,
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
    external_agent_evidence: Vec<ExternalAgentEvidence>,
    context: Value,
    last_result: Value,
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
    context: &Value,
) -> String {
    let context_bytes = serde_json::to_vec(context)
        .expect("canonical runtime Context serializes deterministically");
    let envelope = CanonicalModelRequestEnvelope {
        schema_version: "apxm.model-request-identity.v1",
        program_invocation_id,
        node_execution_id,
        operation,
        context_digest: format!("sha256:{:x}", Sha256::digest(context_bytes)),
    };
    let request_bytes = serde_json::to_vec(&envelope)
        .expect("canonical model request envelope serializes deterministically");
    format!("sha256:{:x}", Sha256::digest(request_bytes))
}

impl DriveState {
    /// A fresh run: emit the instance-created and invocation-admitted lifecycle
    /// facts (sequences 1 and 2), exactly as the single-shot path always has.
    fn new(initial_context: Value, _air: &AirModule, program_invocation_id: &str) -> Self {
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
            external_agent_evidence: Vec::new(),
            context: initial_context,
            last_result: Value::Null,
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

    fn complete_loop_iteration(&mut self, static_loop_id: &str) -> bool {
        let Some(position) = self
            .active_loops
            .iter()
            .position(|frame| frame.static_loop_id == static_loop_id)
        else {
            return false;
        };
        let active = self.active_loops.remove(position);
        if active.failed || active.causal_node_execution_ids.is_empty() {
            return false;
        }
        let loop_occurrence_id = active.dynamic_occurrence_id;
        let iteration_index = active.iteration_index;
        self.seq += 1;
        let completed = Fact::LoopIterationCompleted(LoopIterationCompletedFact::new(
            format!("loop-iteration.{loop_occurrence_id}.{iteration_index}"),
            self.seq,
            static_loop_id.to_string(),
            loop_occurrence_id,
            iteration_index,
            self.program_invocation_id.clone(),
            active.causal_node_execution_ids,
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

    for (schedule_position, step) in schedule.iter().enumerate().skip(start_schedule_position) {
        match step {
            ScheduleStep::HookBefore { binding } => {
                let (before, after) = apply_static_hook(&mut state, ports, binding).await;
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
                    let (before, after) = apply_static_hook(&mut state, ports, binding).await;
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
            ScheduleStep::ContextEdge { from_node, to_node } => {
                state.seq += 1;
                let mut fact = context_transition_fact(&state.program_invocation_id, state.seq);
                let runtime = runtime_fact_mut(&mut fact);
                runtime.air_node_id = Some(to_node.clone());
                runtime.parent_node_execution_id = Some(from_node.clone());
                state.batch.push(fact);
            }
            ScheduleStep::EnterLoop { static_loop_id } => {
                state.enter_loop(static_loop_id);
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
                        let effect_id =
                            model_effect_identity(&state.program_invocation_id, &node_execution_id);
                        let request_digest = model_request_digest(
                            &state.program_invocation_id,
                            &node_execution_id,
                            op,
                            &state.context,
                        );
                        let preparation = ModelCallPreparation::authorize(
                            effect_id,
                            node_execution_id.clone(),
                            request_digest,
                            &ModelTargetRef(target),
                            model_admission,
                        )
                        .map_err(ExecutionError::Binding)?;
                        let metadata = ports
                            .model_call_request_metadata
                            .materialize(&preparation)
                            .map_err(ExecutionError::ModelRequestMetadata)?;
                        let call = ModelCallRequest::prepare(preparation, metadata)
                            .map_err(ExecutionError::ModelRequest)?;
                        let execution =
                            run_model(&*ports.model_inference, &call, RetryPolicy::default());
                        let outcome = execution.outcome;
                        state.last_operation_succeeded =
                            matches!(&outcome, ModelOutcome::CommittedSuccess { .. });
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
                                native_input_tokens: usage.input_tokens,
                                native_output_tokens: usage.output_tokens,
                            };
                            state.batch.push(Fact::AttemptRecorded(attempt.clone()));
                            state.committed_model_attempts.push(attempt);
                        }
                        let result = model_result_value(&outcome);
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
                        if let Some(profile) = capability_ref.strip_prefix("external-agent:") {
                            let session_ref = operand_str(op, "arguments").ok_or_else(|| {
                                ExecutionError::MissingOperand {
                                    node_id: op.node_id.clone(),
                                    operand: "arguments",
                                }
                            })?;
                            let outcome = ports
                                .external_agent
                                .prompt(AcpPromptRequest {
                                    effect_ref: op.node_id.clone(),
                                    session_ref,
                                    profile_ref: profile.to_string(),
                                    prompt: String::new(),
                                })
                                .await;
                            let evidence = assemble_evidence(op.node_id.clone(), &outcome);
                            state.last_operation_succeeded = matches!(
                                &outcome.state,
                                apxm_kernel::PromptEffectState::Completed { .. }
                            );
                            state.external_agent_evidence.push(evidence.clone());
                            state.node_outcomes.push(NodeOutcome::ExternalAgent {
                                node_id: op.node_id.clone(),
                                evidence,
                            });
                        } else {
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
                            let outcome = ports
                                .capability
                                .invoke(
                                    CapabilityRequest::prepare(
                                        capability_ref,
                                        arguments_type_ref,
                                        admission.arguments.clone(),
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
                            });
                        }
                        state.node_outcomes.push(NodeOutcome::AwaitEvent {
                            node_id: op.node_id.clone(),
                            outcome,
                        });
                    }
                }
                state.record_node_outcome(
                    loop_path,
                    &node_execution_id,
                    state.last_operation_succeeded,
                );
            }
            ScheduleStep::LoopBackEdge { static_loop_id } => {
                state.complete_loop_iteration(static_loop_id);
            }
            ScheduleStep::ProgramYield { region_id } => {
                state.fail_active_loops();
                if options.suspend_on_park && options.yield_at_loop {
                    return Ok(DriveEnd::Parked {
                        state,
                        continuation_id: region_id.clone(),
                        event_ref: None,
                        next_schedule_position: schedule_position + 1,
                        parked_node_execution_id: None,
                        parked_loop_path: Vec::new(),
                    });
                }
                return Ok(DriveEnd::RanToEnd(state));
            }
            ScheduleStep::ProgramReturn { .. } | ScheduleStep::ProgramExit { .. } => {
                state.fail_active_loops();
                return Ok(DriveEnd::RanToEnd(state));
            }
        }
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
) -> (Value, Value) {
    let before = state.context.clone();
    let outcome = ports
        .hook_handlers
        .execute(binding, &state.context, &state.last_result)
        .await;
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
    (before, state.context.clone())
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
) -> RunReport {
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

    let commit = ports
        .execution_commit
        .commit(ExecutionCommitRequest {
            commit_id: commit_id.to_string(),
            program_instance_ref: program_instance_ref.clone(),
            program_invocation_ref: program_invocation_ref.clone(),
            idempotency_key: format!("idem.{commit_id}"),
            expected_program_state_version: expected,
            write_set,
            tuple: commit_tuple(&state, None, None),
            evidence_batch: state.batch.clone(),
        })
        .await;

    let operational_usage = publish_committed_native_model_usage(
        ports,
        commit_id,
        &commit,
        &state.committed_model_attempts,
    )
    .await;

    RunReport {
        node_outcomes: state.node_outcomes,
        native_usage: state.native_usage,
        external_agent_evidence: state.external_agent_evidence,
        final_context: state.context,
        commit,
        operational_usage,
    }
}

async fn publish_committed_native_model_usage(
    ports: &ExecutionPorts,
    commit_id: &str,
    commit: &ExecutionCommitResult,
    attempts: &[ModelAttemptRecordedFact],
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

    let mut failure = None;
    for attempt in attempts {
        let usage = CommittedNativeModelUsage {
            schema_version: CommittedNativeModelUsageVersion::V1,
            source_contract_digest: CommittedNativeModelUsage::SOURCE_CONTRACT_DIGEST.to_string(),
            usage_measurement_id: CommittedNativeModelUsage::measurement_id(
                commit_id,
                &attempt.fact_id,
            ),
            commit_id: commit_id.to_string(),
            evidence_position_ref: EvidencePositionRef {
                ref_type: EvidencePositionRefType::EvidencePositionRef,
                r#ref: evidence_position_ref.clone(),
            },
            attempt: attempt.clone(),
        };
        if let Err(error) = port.publish(usage).await {
            failure.get_or_insert(error);
        }
    }
    match failure {
        Some(error) => CommittedNativeModelUsageOutcome::Failed(error),
        None => CommittedNativeModelUsageOutcome::Published,
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
) -> (ExecutionCommitResult, CommittedNativeModelUsageOutcome) {
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
    let commit = ports
        .execution_commit
        .commit(ExecutionCommitRequest {
            commit_id: format!("{}.yield", continuation.commit_id),
            program_instance_ref: continuation.program_instance_ref.clone(),
            program_invocation_ref: continuation.program_invocation_ref.clone(),
            idempotency_key: format!("idem.{}.yield", continuation.commit_id),
            expected_program_state_version: expected,
            write_set: continuation.write_set.clone(),
            tuple: commit_tuple(&state, Some(payload), event_wait),
            evidence_batch: state.batch,
        })
        .await;
    let operational_usage = publish_committed_native_model_usage(
        ports,
        &format!("{}.yield", continuation.commit_id),
        &commit,
        &attempts,
    )
    .await;
    (commit, operational_usage)
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
    let state = DriveState::new(
        initial_context,
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
    Ok(commit_and_report(
        ports,
        &request.program_instance_ref,
        &request.program_invocation_ref,
        &request.commit_id,
        request.write_set,
        state,
    )
    .await)
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
    let state = DriveState::new(
        initial_context,
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
        context,
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
        external_agent_evidence,
        context,
        last_result: Value::Null,
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
    } else if event_ref.is_none() {
        state.context = delivered;
    }

    let yield_at_loop = event_ref.is_some();
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
            yield_at_loop,
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
            Ok(RunOutcome::Completed(report))
        }
        DriveEnd::Parked {
            state,
            continuation_id,
            event_ref,
            next_schedule_position,
            parked_node_execution_id,
            parked_loop_path,
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
                context: state.context.clone(),
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
            let (commit, operational_usage) = commit_suspension(ports, &cont, state).await;
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
            "schema_version": "apxm.air.v1",
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
        let mut state = DriveState::new(Value::Null, &loop_air(), "invocation.1");
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
        let mut state = DriveState::new(Value::Null, &loop_air(), "invocation.1");
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
}
