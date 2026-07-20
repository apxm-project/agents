//! The canonical runtime driver: execute a five-operation AIR node graph through
//! injected kernel ports and commit atomically.
//!
//! The driver walks the semantic operations in order, dispatching each to its
//! exact injected port — `model.call` to the model inference port,
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

use serde_json::Value;

use apxm_inference::{
    BindingError, ModelBindingAdmission, ModelCallRequest, ModelInferencePort, ModelOutcome,
    ModelTargetRef, RetryPolicy, Usage, execute as run_model,
};
use apxm_kernel::{
    AcpPromptRequest, AtomicWriteSet, ExecutionCommitPort, ExecutionCommitRequest,
    ExecutionCommitResult, ExternalAgentCapabilityPort, Hook, apply_hooks, assemble_evidence,
};
use apxm_program::air::{AirModule, SemanticOp, SemanticOpKind};
use apxm_program::external_agent::ExternalAgentEvidence;
use apxm_program::runtime_evidence::{Fact, FactKind, InstanceState, InvocationState};

use crate::ports::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, EventAwait, EventOutcome, EventPort,
};
use crate::resume::{Continuation, ContinuationError, ContinuationPort, RunOutcome};

/// The exact set of injected ports the driver drives. Every port is a single
/// admitted implementation; the driver holds no registry and does no discovery.
pub struct ExecutionPorts {
    pub model_inference: Arc<dyn ModelInferencePort + Send + Sync>,
    pub capability: Arc<dyn CapabilityPort>,
    pub external_agent: Arc<dyn ExternalAgentCapabilityPort>,
    pub events: Arc<dyn EventPort>,
    pub composition: Arc<dyn CompositionPort>,
    pub execution_commit: Arc<dyn ExecutionCommitPort>,
}

/// One canonical execution request: the AIR to run, the materialized
/// model-binding admission, and the commit scope and prepared write set.
pub struct ExecutionRequest {
    pub air: AirModule,
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
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingOperand { node_id, operand } => {
                write!(f, "node {node_id} is missing operand {operand}")
            }
            Self::Binding(error) => write!(f, "model binding error: {error}"),
            Self::Continuation(error) => write!(f, "continuation error: {error}"),
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
    batch: Vec<Fact>,
    seq: u64,
}

impl DriveState {
    /// A fresh run: emit the instance-created and invocation-admitted lifecycle
    /// facts (sequences 1 and 2), exactly as the single-shot path always has.
    fn new(initial_context: Value) -> Self {
        let mut batch = Vec::new();
        let mut seq = 0u64;
        seq += 1;
        batch.push(fact(seq, FactKind::InstanceCreated, Some(InstanceState::Ready), None, None, None));
        seq += 1;
        batch.push(fact(seq, FactKind::InvocationAdmitted, None, Some(InvocationState::Running), None, None));
        Self {
            node_outcomes: Vec::new(),
            native_usage: Usage::default(),
            external_agent_evidence: Vec::new(),
            context: initial_context,
            batch,
            seq,
        }
    }
}

/// The result of driving the op loop from a start index: either it reached the
/// end, or it parked at an `await.event` (only when `suspend_on_park`).
enum DriveEnd {
    RanToEnd(DriveState),
    Parked {
        state: DriveState,
        wait_key: String,
        next_op_index: usize,
    },
}

/// Walk `air.semantic_operations[start_index..]`, dispatching each op to its
/// exact injected port and threading Context through the model Hooks. When
/// `suspend_on_park` is set, a parked `await.event` stops the walk and returns
/// [`DriveEnd::Parked`]; otherwise a parked outcome is recorded and the walk
/// continues (the single-shot contract).
async fn drive_from(
    ports: &ExecutionPorts,
    air: &AirModule,
    model_admission: &ModelBindingAdmission,
    start_index: usize,
    mut state: DriveState,
    model_hooks: &[Hook<Value, Value>],
    suspend_on_park: bool,
) -> Result<DriveEnd, ExecutionError> {
    for index in start_index..air.semantic_operations.len() {
        let op = &air.semantic_operations[index];
        state.seq += 1;
        state.batch.push(fact(
            state.seq,
            FactKind::AttemptRecorded,
            None,
            None,
            Some(op.node_id.clone()),
            None,
        ));

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
                let outcome = run_model(&*ports.model_inference, &call, RetryPolicy::default());
                if let ModelOutcome::CommittedSuccess { usage } = &outcome {
                    state.native_usage.input_tokens += usage.input_tokens;
                    state.native_usage.output_tokens += usage.output_tokens;
                }
                let effect = apply_hooks(state.context, model_result_value(&outcome), model_hooks);
                state.context = effect.context;
                state.node_outcomes.push(NodeOutcome::Model {
                    node_id: op.node_id.clone(),
                    outcome,
                    result: effect.result,
                    replaced: effect.replaced,
                });
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
                state.node_outcomes.push(NodeOutcome::ProgramNew {
                    node_id: op.node_id.clone(),
                    outcome,
                });
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
                state.node_outcomes.push(NodeOutcome::ProgramInvoke {
                    node_id: op.node_id.clone(),
                    outcome,
                });
            }
            SemanticOpKind::AwaitEvent => {
                let selector = operand_str(op, "event_selector").unwrap_or_default();
                let outcome = ports
                    .events
                    .await_event(EventAwait {
                        node_id: op.node_id.clone(),
                        selector: selector.clone(),
                    })
                    .await;
                if suspend_on_park && matches!(outcome, EventOutcome::Parked) {
                    return Ok(DriveEnd::Parked {
                        state,
                        wait_key: selector,
                        next_op_index: index + 1,
                    });
                }
                state.node_outcomes.push(NodeOutcome::AwaitEvent {
                    node_id: op.node_id.clone(),
                    outcome,
                });
            }
        }
    }
    Ok(DriveEnd::RanToEnd(state))
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
            evidence_batch: state.batch,
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
    model_hooks: &[Hook<Value, Value>],
) -> Result<RunReport, ExecutionError> {
    let state = DriveState::new(initial_context);
    let end = drive_from(
        ports,
        &request.air,
        &request.model_admission,
        0,
        state,
        model_hooks,
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
    continuation: &dyn ContinuationPort,
    request: ExecutionRequest,
    initial_context: Value,
    model_hooks: &[Hook<Value, Value>],
) -> Result<RunOutcome, ExecutionError> {
    let state = DriveState::new(initial_context);
    let end = drive_from(
        ports,
        &request.air,
        &request.model_admission,
        0,
        state,
        model_hooks,
        true,
    )
    .await?;
    finish(ports, continuation, request_parts(request), end).await
}

/// Resume a parked execution: take the continuation for `wait_key`, record the
/// delivered value as the parked `await.event`'s fulfillment, and continue from
/// the next operation. Completing commits atomically; parking again re-persists.
///
/// # Errors
///
/// Returns [`ExecutionError::Continuation`] when no continuation is parked on
/// `wait_key`, plus the same operand/binding/store errors as
/// [`execute_resumable`].
pub async fn resume(
    ports: &ExecutionPorts,
    continuation: &dyn ContinuationPort,
    wait_key: &str,
    delivered: Value,
    model_hooks: &[Hook<Value, Value>],
) -> Result<RunOutcome, ExecutionError> {
    let parked = continuation
        .take(wait_key)
        .await
        .map_err(ExecutionError::Continuation)?
        .ok_or_else(|| {
            ExecutionError::Continuation(ContinuationError::NotParked {
                wait_key: wait_key.to_string(),
            })
        })?;

    let Continuation {
        air,
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
        wait_key: _,
    } = parked;

    let mut state = DriveState {
        node_outcomes: Vec::new(),
        native_usage,
        external_agent_evidence,
        context,
        batch: evidence_batch,
        seq: event_sequence,
    };

    // Record the delivered input as the parked `await.event`'s fulfillment so
    // the resumed run's evidence includes the turn the wake delivered.
    if next_op_index > 0
        && let Some(op) = air.semantic_operations.get(next_op_index - 1)
    {
        let payload = match &delivered {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        state.node_outcomes.push(NodeOutcome::AwaitEvent {
            node_id: op.node_id.clone(),
            outcome: EventOutcome::Fulfilled { payload },
        });
    }

    let end = drive_from(
        ports,
        &air,
        &model_admission,
        next_op_index,
        state,
        model_hooks,
        true,
    )
    .await?;
    let parts = CommitParts {
        air,
        model_admission,
        version_scope,
        commit_id,
        write_set,
    };
    finish(ports, continuation, parts, end).await
}

/// The commit-scope parts carried from a request or a resumed continuation,
/// reused to build the next continuation on a re-park.
struct CommitParts {
    air: AirModule,
    model_admission: ModelBindingAdmission,
    version_scope: String,
    commit_id: String,
    write_set: AtomicWriteSet,
}

fn request_parts(request: ExecutionRequest) -> CommitParts {
    CommitParts {
        air: request.air,
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
    continuation: &dyn ContinuationPort,
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
            wait_key,
            next_op_index,
        } => {
            let cont = Continuation {
                air: parts.air,
                model_admission: parts.model_admission,
                next_op_index,
                context: state.context,
                native_usage: state.native_usage,
                external_agent_evidence: state.external_agent_evidence,
                evidence_batch: state.batch,
                event_sequence: state.seq,
                version_scope: parts.version_scope,
                commit_id: parts.commit_id,
                write_set: parts.write_set,
                wait_key: wait_key.clone(),
            };
            continuation
                .persist(cont)
                .await
                .map_err(ExecutionError::Continuation)?;
            Ok(RunOutcome::Suspended { wait_key })
        }
    }
}
