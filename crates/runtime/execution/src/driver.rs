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

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use apxm_inference::{
    BindingError, ModelBindingAdmission, ModelCallRequest, ModelInferencePort, ModelOutcome,
    ModelTargetRef, RetryPolicy, Usage, execute as run_model,
};
use apxm_kernel::{
    AcpPromptRequest, AtomicWriteSet, ExecutionCommitPort, ExecutionCommitRequest,
    ExecutionCommitResult, ExecutionCommitTuple, ExternalAgentCapabilityPort, assemble_evidence,
};
use apxm_program::air::{AirModule, SemanticOp, SemanticOpKind};
use apxm_program::common::TypedRef;
use apxm_program::external_agent::ExternalAgentEvidence;
use apxm_program::frontend_graph::HookBinding;
use apxm_program::runtime_evidence::{
    Fact, FactKind, HookPhase as EvidenceHookPhase, HookScope as EvidenceHookScope, InstanceState,
    InvocationState,
};

use crate::ports::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, EventAwait, EventOutcome, EventPort, EventRef, EventRefError,
};
use crate::resume::{Continuation, ContinuationError, RunOutcome};
use crate::structural::{ScheduleStep, build_schedule};

/// The exact set of injected ports the driver drives. Every port is a single
/// admitted implementation; the driver holds no registry and does no discovery.
pub struct ExecutionPorts {
    pub model_inference: Arc<dyn ModelInferencePort + Send + Sync>,
    pub capability: Arc<dyn CapabilityPort>,
    pub external_agent: Arc<dyn ExternalAgentCapabilityPort>,
    pub events: Arc<dyn EventPort>,
    pub composition: Arc<dyn CompositionPort>,
    pub execution_commit: Arc<dyn ExecutionCommitPort>,
    pub hook_handlers: Arc<dyn StaticHookHandlerPort>,
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
/// model-binding admission, and the commit scope and prepared write set.
pub struct ExecutionRequest {
    pub air: AirModule,
    pub hook_bindings: Vec<HookBinding>,
    pub model_admission: ModelBindingAdmission,
    pub version_scope: String,
    pub commit_id: String,
    pub write_set: AtomicWriteSet,
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
}

/// Why a canonical run could not be driven.
#[derive(Debug)]
pub enum ExecutionError {
    MissingOperand {
        node_id: String,
        operand: &'static str,
    },
    Binding(BindingError),
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
        invocation_ref: String,
    },
    Commit(ExecutionCommitResult),
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingOperand { node_id, operand } => {
                write!(f, "node {node_id} is missing operand {operand}")
            }
            Self::Binding(error) => write!(f, "model binding error: {error}"),
            Self::Continuation(error) => write!(f, "continuation error: {error}"),
            Self::InvalidEventRef { node_id, source } => {
                write!(f, "node {node_id} has an invalid event_ref: {source}")
            }
            Self::EventRefMismatch { expected, delivered } => {
                write!(f, "event reference mismatch: expected {expected}, delivered {delivered}")
            }
            Self::EventDeliveryRequiresRef { invocation_ref } => {
                write!(f, "continuation for {invocation_ref} requires an EventRef delivery")
            }
            Self::Commit(result) => write!(f, "atomic execution commit failed: {}", result.label()),
        }
    }
}

impl std::error::Error for ExecutionError {}

fn operand_str(op: &SemanticOp, key: &str) -> Option<String> {
    op.operands
        .as_ref()?
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn model_result_value(outcome: &ModelOutcome) -> Value {
    match outcome {
        ModelOutcome::CommittedSuccess { .. } => Value::String("model.result".to_string()),
        _ => Value::Null,
    }
}

fn fact(
    seq: u64,
    kind: FactKind,
    instance_state: Option<InstanceState>,
    invocation_state: Option<InvocationState>,
    node_execution_id: Option<String>,
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
        node_execution_id,
        attempt_id: None,
        region_occurrence_id: None,
        static_region_id: None,
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
    }
}

/// The mutable run accumulators threaded through op dispatch. Shared by the
/// single-shot and resumable paths so both build identical evidence.
struct DriveState {
    node_outcomes: Vec<NodeOutcome>,
    native_usage: Usage,
    external_agent_evidence: Vec<ExternalAgentEvidence>,
    context: Value,
    last_result: Value,
    last_operation_succeeded: bool,
    batch: Vec<Fact>,
    seq: u64,
    static_region_id: Option<String>,
    region_occurrence_id: Option<String>,
    last_model_node_execution_id: Option<String>,
    last_program_new_node_execution_id: Option<String>,
}

impl DriveState {
    /// A fresh run: emit the instance-created and invocation-admitted lifecycle
    /// facts (sequences 1 and 2), exactly as the single-shot path always has.
    fn new(initial_context: Value, air: &AirModule) -> Self {
        let mut batch = Vec::new();
        let mut seq = 0u64;
        seq += 1;
        batch.push(fact(seq, FactKind::InstanceCreated, Some(InstanceState::Ready), None, None, None));
        seq += 1;
        batch.push(fact(seq, FactKind::InvocationAdmitted, None, Some(InvocationState::Running), None, None));
        let mut state = Self {
            node_outcomes: Vec::new(),
            native_usage: Usage::default(),
            external_agent_evidence: Vec::new(),
            context: initial_context,
            last_result: Value::Null,
            last_operation_succeeded: true,
            batch,
            seq,
            static_region_id: conversational_region_id(air),
            region_occurrence_id: None,
            last_model_node_execution_id: None,
            last_program_new_node_execution_id: None,
        };
        state.begin_region_occurrence();
        state
    }

    fn begin_region_occurrence(&mut self) {
        let Some(static_region_id) = self.static_region_id.clone() else {
            return;
        };
        self.seq += 1;
        let occurrence = format!("region-occurrence.{static_region_id}.{}", self.seq);
        self.batch.push(join_fact(
            self.seq,
            FactKind::RegionOccurrenceStarted,
            Some(occurrence.clone()),
            Some(static_region_id),
            None,
            None,
            None,
        ));
        self.region_occurrence_id = Some(occurrence);
        self.last_model_node_execution_id = None;
        self.last_program_new_node_execution_id = None;
    }
}

fn conversational_region_id(air: &AirModule) -> Option<String> {
    air.source_map
        .region_annotations
        .iter()
        .find(|region| matches!(region.annotation, apxm_program::source_map::RegionAnnotationKind::ConversationalLoop))
        .map(|region| region.region_id.clone())
}

fn join_fact(
    seq: u64,
    kind: FactKind,
    region_occurrence_id: Option<String>,
    static_region_id: Option<String>,
    node_execution_id: Option<String>,
    air_node_id: Option<String>,
    parent_node_execution_id: Option<String>,
) -> Fact {
    let mut fact = fact(seq, kind, None, None, None, None);
    fact.region_occurrence_id = region_occurrence_id;
    fact.static_region_id = static_region_id;
    fact.node_execution_id = node_execution_id;
    fact.air_node_id = air_node_id;
    fact.parent_node_execution_id = parent_node_execution_id;
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
        next_op_index: usize,
    },
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
    start_index: usize,
    mut state: DriveState,
    suspend_on_park: bool,
    yield_at_loop: bool,
) -> Result<DriveEnd, ExecutionError> {
    let schedule = build_schedule(air, hook_bindings);
    let start_pos = schedule
        .iter()
        .position(|step| matches!(step, ScheduleStep::Semantic { index } if *index == start_index))
        .unwrap_or(0);

    for step in schedule.iter().skip(start_pos) {
        match step {
            ScheduleStep::HookBefore { binding } => {
                let (before, after) = apply_static_hook(&mut state, ports, binding).await;
                state.seq += 1;
                let mut fact = fact(state.seq, FactKind::HookExecuted, None, None, None, None);
                fact.hook_execution_id = Some(format!("hook-execution.{}.{}", binding.hook_id, state.seq));
                fact.hook_id = Some(binding.hook_id.clone());
                fact.hook_scope = Some(evidence_hook_scope(binding.scope));
                fact.hook_phase = Some(EvidenceHookPhase::Before);
                fact.context_before_ref = Some(context_ref(format!("context.{}.before", state.seq)));
                fact.context_after_ref = Some(context_ref(format!("context.{}.after", state.seq)));
                state.batch.push(fact);
                if before != after {
                    state.seq += 1;
                    state.batch.push(context_transition_fact(state.seq));
                }
            }
            ScheduleStep::HookAfter { binding } => {
                if state.last_operation_succeeded {
                    let (before, after) = apply_static_hook(&mut state, ports, binding).await;
                    state.seq += 1;
                    let mut fact = fact(state.seq, FactKind::HookExecuted, None, None, None, None);
                    fact.hook_execution_id = Some(format!("hook-execution.{}.{}", binding.hook_id, state.seq));
                    fact.hook_id = Some(binding.hook_id.clone());
                    fact.hook_scope = Some(evidence_hook_scope(binding.scope));
                    fact.hook_phase = Some(EvidenceHookPhase::After);
                    fact.context_before_ref = Some(context_ref(format!("context.{}.before", state.seq)));
                    fact.context_after_ref = Some(context_ref(format!("context.{}.after", state.seq)));
                    state.batch.push(fact);
                    if before != after {
                        state.seq += 1;
                        state.batch.push(context_transition_fact(state.seq));
                    }
                }
            }
            ScheduleStep::ContextEdge { from_node, to_node } => {
                state.seq += 1;
                let mut fact = context_transition_fact(state.seq);
                fact.air_node_id = Some(to_node.clone());
                fact.parent_node_execution_id = Some(from_node.clone());
                state.batch.push(fact);
            }
            ScheduleStep::Semantic { index } => {
                let op = &air.semantic_operations[*index];
                state.seq += 1;
                let node_execution_id = state.static_region_id.as_ref().map(|_| {
                    format!("node-execution.{}.{}", op.node_id, state.seq)
                });
                if let Some(node_execution_id) = node_execution_id.clone() {
                    state.batch.push(join_fact(
                        state.seq,
                        FactKind::NodeExecutionRecorded,
                        state.region_occurrence_id.clone(),
                        state.static_region_id.clone(),
                        Some(node_execution_id),
                        Some(op.node_id.clone()),
                        match op.op {
                            SemanticOpKind::CapabilityInvoke => {
                                state.last_model_node_execution_id.clone()
                            }
                            SemanticOpKind::ProgramInvoke => {
                                state.last_program_new_node_execution_id.clone()
                            }
                            _ => None,
                        },
                    ));
                } else {
                    state.batch.push(fact(
                        state.seq,
                        FactKind::AttemptRecorded,
                        None,
                        None,
                        Some(op.node_id.clone()),
                        None,
                    ));
                }

                match op.op {
                    SemanticOpKind::ModelCall => {
                        let target = operand_str(op, "model_target_ref").ok_or_else(|| {
                            ExecutionError::MissingOperand {
                                node_id: op.node_id.clone(),
                                operand: "model_target_ref",
                            }
                        })?;
                        let call = ModelCallRequest::authorize(
                            op.node_id.clone(),
                            op.node_id.clone(),
                            &ModelTargetRef(target),
                            model_admission,
                        )
                        .map_err(ExecutionError::Binding)?;
                        let outcome =
                            run_model(&*ports.model_inference, &call, RetryPolicy::default());
                        state.last_operation_succeeded =
                            matches!(&outcome, ModelOutcome::CommittedSuccess { .. });
                        if let ModelOutcome::CommittedSuccess { usage } = &outcome {
                            state.native_usage.input_tokens += usage.input_tokens;
                            state.native_usage.output_tokens += usage.output_tokens;
                        }
                        let result = model_result_value(&outcome);
                        state.last_result = result.clone();
                        state.node_outcomes.push(NodeOutcome::Model {
                            node_id: op.node_id.clone(),
                            outcome,
                            result,
                            replaced: false,
                        });
                        state.last_model_node_execution_id = node_execution_id;
                    }
                    SemanticOpKind::CapabilityInvoke => {
                        let capability_ref = operand_str(op, "capability_ref").ok_or_else(|| {
                            ExecutionError::MissingOperand {
                                node_id: op.node_id.clone(),
                                operand: "capability_ref",
                            }
                        })?;
                        if let Some(profile) = capability_ref.strip_prefix("external-agent:") {
                            let outcome = ports
                                .external_agent
                                .prompt(AcpPromptRequest {
                                    effect_ref: op.node_id.clone(),
                                    session_ref: operand_str(op, "external_agent_session")
                                        .unwrap_or_default(),
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
                            let outcome = ports
                                .capability
                                .invoke(CapabilityRequest {
                                    node_id: op.node_id.clone(),
                                    capability_ref,
                                })
                                .await;
                            state.last_operation_succeeded =
                                matches!(&outcome, CapabilityOutcome::Completed { .. });
                            state.last_result = match &outcome {
                                CapabilityOutcome::Completed { result } => Value::String(result.clone()),
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
                        let outcome = ports
                            .composition
                            .program_new(CompositionRequest {
                                node_id: op.node_id.clone(),
                                program_ref: operand_str(op, "program_ref")
                                    .unwrap_or_else(|| op.node_id.clone()),
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
                        state.node_outcomes.push(NodeOutcome::ProgramNew {
                            node_id: op.node_id.clone(),
                            outcome,
                        });
                        state.last_program_new_node_execution_id = node_execution_id;
                    }
                    SemanticOpKind::ProgramInvoke => {
                        let outcome = ports
                            .composition
                            .program_invoke(CompositionRequest {
                                node_id: op.node_id.clone(),
                                program_ref: operand_str(op, "program_ref")
                                    .unwrap_or_else(|| op.node_id.clone()),
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
                                EventRef::new(value).map_err(|source| ExecutionError::InvalidEventRef {
                                    node_id: op.node_id.clone(),
                                    source,
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
                        if suspend_on_park && matches!(&outcome, EventOutcome::Parked) {
                            state.seq += 1;
                            state.batch.push(fact(
                                state.seq,
                                FactKind::EventAwaitRegistered,
                                None,
                                Some(InvocationState::WaitingEvent),
                                Some(op.node_id.clone()),
                                None,
                            ));
                            state.seq += 1;
                            state.batch.push(fact(
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
                                next_op_index: *index + 1,
                            });
                        }
                        state.node_outcomes.push(NodeOutcome::AwaitEvent {
                            node_id: op.node_id.clone(),
                            outcome,
                        });
                    }
                }
            }
            ScheduleStep::LoopYield {
                continuation_id,
                resume_semantic_index,
            } => {
                if suspend_on_park && yield_at_loop {
                    return Ok(DriveEnd::Parked {
                        state,
                        continuation_id: continuation_id.clone(),
                        event_ref: None,
                        next_op_index: *resume_semantic_index,
                    });
                }
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

fn context_transition_fact(seq: u64) -> Fact {
    let mut fact = fact(seq, FactKind::ContextTransitioned, None, None, None, None);
    fact.context_transition_id = Some(format!("context-transition.{seq}"));
    fact.context_before_ref = Some(context_ref(format!("context.{}.before", seq)));
    fact.context_after_ref = Some(context_ref(format!("context.{}.after", seq)));
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
    version_scope: &str,
    commit_id: &str,
    write_set: AtomicWriteSet,
    mut state: DriveState,
) -> RunReport {
    let expected = ports.execution_commit.current_version(version_scope).await;
    state.seq += 1;
    state.batch.push(fact(
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
            invocation_ref: version_scope.to_string(),
            idempotency_key: format!("idem.{commit_id}"),
            expected_program_state_version: expected,
            write_set,
            tuple: commit_tuple(&state, None, None),
            evidence_batch: state.batch.clone(),
        })
        .await;

    RunReport {
        node_outcomes: state.node_outcomes,
        native_usage: state.native_usage,
        external_agent_evidence: state.external_agent_evidence,
        final_context: state.context,
        commit,
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
        effect_outcomes: state
            .node_outcomes
            .iter()
            .map(node_outcome_value)
            .collect(),
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
) -> ExecutionCommitResult {
    let expected = ports
        .execution_commit
        .current_version(&continuation.version_scope)
        .await;
    state.seq += 1;
    state.batch.push(fact(
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
    ports
        .execution_commit
        .commit(ExecutionCommitRequest {
            commit_id: format!("{}.yield", continuation.commit_id),
            invocation_ref: continuation.version_scope.clone(),
            idempotency_key: format!("idem.{}.yield", continuation.commit_id),
            expected_program_state_version: expected,
            write_set: continuation.write_set.clone(),
            tuple: commit_tuple(&state, Some(payload), event_wait),
            evidence_batch: state.batch,
        })
        .await
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
    let state = DriveState::new(initial_context, &request.air);
    let end = drive_from(
        ports,
        &request.air,
        &request.hook_bindings,
        &request.model_admission,
        0,
        state,
        false,
        false,
    )
    .await?;
    let DriveEnd::RanToEnd(state) = end else {
        unreachable!("single-shot execute never suspends (suspend_on_park = false)");
    };
    Ok(commit_and_report(
        ports,
        &request.version_scope,
        &request.commit_id,
        request.write_set,
        state,
    )
    .await)
}

/// Execute a canonical AIR program with durable park/resume. It behaves exactly
/// like [`execute`] until it reaches a parked `await.event`, at which point it
/// persists a [`Continuation`] through `continuation` and returns
/// [`RunOutcome::Suspended`] without committing. When it runs to the end it
/// commits atomically and returns [`RunOutcome::Completed`].
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
    let state = DriveState::new(initial_context, &request.air);
    let end = drive_from(
        ports,
        &request.air,
        &request.hook_bindings,
        &request.model_admission,
        0,
        state,
        true,
        true,
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
/// continuation payload for `version_scope`.
pub async fn resume(
    ports: &ExecutionPorts,
    version_scope: &str,
    delivered: Value,
) -> Result<RunOutcome, ExecutionError> {
    resume_from_continuation(ports, version_scope, None, delivered).await
}

/// Resume a committed `await.event` continuation with one exact EventRef.
///
/// # Errors
///
/// Returns [`ExecutionError::EventRefMismatch`] when an OS delivery targets a
/// different durable event than the one atomically registered at park time.
pub async fn resume_event(
    ports: &ExecutionPorts,
    version_scope: &str,
    event_ref: EventRef,
    delivered: Value,
) -> Result<RunOutcome, ExecutionError> {
    resume_from_continuation(ports, version_scope, Some(event_ref), delivered).await
}

async fn resume_from_continuation(
    ports: &ExecutionPorts,
    version_scope: &str,
    delivered_event_ref: Option<EventRef>,
    delivered: Value,
) -> Result<RunOutcome, ExecutionError> {
    let payload = ports
        .execution_commit
        .load_continuation(version_scope)
        .await
        .ok_or_else(|| {
            ExecutionError::Continuation(ContinuationError::NotCommitted {
                invocation_ref: version_scope.to_string(),
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
        next_op_index,
        context,
        native_usage,
        external_agent_evidence,
        evidence_batch,
        event_sequence,
        version_scope,
        commit_id,
        write_set,
        continuation_id: _,
        event_ref,
    } = parked;

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
                invocation_ref: version_scope.to_string(),
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
        external_agent_evidence,
        context,
        last_result: Value::Null,
        last_operation_succeeded: true,
        batch: evidence_batch,
        seq: event_sequence,
        static_region_id: conversational_region_id(&air),
        region_occurrence_id: None,
        last_model_node_execution_id: None,
        last_program_new_node_execution_id: None,
    };
    if next_op_index == 0 {
        state.begin_region_occurrence();
    }

    // Record the delivered input as the parked `await.event`'s fulfillment so
    // the resumed run's evidence includes the turn the wake delivered.
    if next_op_index > 0
        && let Some(op) = air.semantic_operations.get(next_op_index - 1)
    {
        let event_ref = event_ref.clone().ok_or_else(|| {
            ExecutionError::EventDeliveryRequiresRef {
                invocation_ref: version_scope.clone(),
            }
        })?;
        let payload = match &delivered {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        state.node_outcomes.push(NodeOutcome::AwaitEvent {
            node_id: op.node_id.clone(),
            outcome: EventOutcome::Fulfilled { event_ref, payload },
        });
    } else if event_ref.is_none() {
        state.context = delivered;
    }

    let yield_at_loop = event_ref.is_some();
    let end = drive_from(
        ports,
        &air,
        &hook_bindings,
        &model_admission,
        next_op_index,
        state,
        true,
        yield_at_loop,
    )
    .await?;
    let parts = CommitParts {
        air,
        hook_bindings,
        model_admission,
        version_scope,
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
    version_scope: String,
    commit_id: String,
    write_set: AtomicWriteSet,
}

fn request_parts(request: ExecutionRequest) -> CommitParts {
    CommitParts {
        air: request.air,
        hook_bindings: request.hook_bindings,
        model_admission: request.model_admission,
        version_scope: request.version_scope,
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
                &parts.version_scope,
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
            next_op_index,
        } => {
            let cont = Continuation {
                air: parts.air,
                hook_bindings: parts.hook_bindings,
                model_admission: parts.model_admission,
                next_op_index,
                context: state.context.clone(),
                native_usage: state.native_usage.clone(),
                external_agent_evidence: state.external_agent_evidence.clone(),
                evidence_batch: state.batch.clone(),
                event_sequence: state.seq,
                version_scope: parts.version_scope,
                commit_id: parts.commit_id,
                write_set: parts.write_set,
                continuation_id: continuation_id.clone(),
                event_ref,
            };
            match commit_suspension(ports, &cont, state).await {
                ExecutionCommitResult::Committed { .. } => Ok(RunOutcome::Suspended {
                    continuation_id,
                    event_ref: cont.event_ref,
                }),
                result => Err(ExecutionError::Commit(result)),
            }
        }
    }
}
