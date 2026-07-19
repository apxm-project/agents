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
//! fallback, and no first-available selection: a model effect resolves exactly
//! one admitted binding or fails closed.

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

/// One canonical execution request: the AIR to run, the exact model-binding
/// admission, and the commit scope and prepared write set.
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
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingOperand { node_id, operand } => {
                write!(f, "node {node_id} is missing operand {operand}")
            }
            Self::Binding(error) => write!(f, "model binding error: {error}"),
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

/// Execute a canonical AIR program and commit its effects atomically.
///
/// # Errors
///
/// Returns [`ExecutionError`] if an operation is missing a required operand or a
/// model effect cannot resolve exactly one admitted binding.
pub async fn execute(
    ports: &ExecutionPorts,
    request: ExecutionRequest,
    initial_context: Value,
    model_hooks: &[Hook<Value, Value>],
) -> Result<RunReport, ExecutionError> {
    let mut node_outcomes = Vec::new();
    let mut native_usage = Usage::default();
    let mut external_agent_evidence = Vec::new();
    let mut context = initial_context;
    let mut batch: Vec<Fact> = Vec::new();
    let mut seq = 0u64;

    seq += 1;
    batch.push(fact(seq, FactKind::InstanceCreated, Some(InstanceState::Ready), None, None, None));
    seq += 1;
    batch.push(fact(seq, FactKind::InvocationAdmitted, None, Some(InvocationState::Running), None, None));

    for op in &request.air.semantic_operations {
        seq += 1;
        batch.push(fact(seq, FactKind::AttemptRecorded, None, None, Some(op.node_id.clone()), None));

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
                    &request.model_admission,
                )
                .map_err(ExecutionError::Binding)?;
                let outcome = run_model(&*ports.model_inference, &call, RetryPolicy::default());
                if let ModelOutcome::CommittedSuccess { usage } = &outcome {
                    native_usage.input_tokens += usage.input_tokens;
                    native_usage.output_tokens += usage.output_tokens;
                }
                let effect = apply_hooks(context, model_result_value(&outcome), model_hooks);
                context = effect.context;
                node_outcomes.push(NodeOutcome::Model {
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
                            session_ref: operand_str(op, "external_agent_session").unwrap_or_default(),
                            profile_ref: profile.to_string(),
                            prompt: String::new(),
                        })
                        .await;
                    let evidence = assemble_evidence(op.node_id.clone(), &outcome);
                    external_agent_evidence.push(evidence.clone());
                    node_outcomes.push(NodeOutcome::ExternalAgent {
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
                    node_outcomes.push(NodeOutcome::Capability {
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
                        program_ref: operand_str(op, "program_ref").unwrap_or_else(|| op.node_id.clone()),
                    })
                    .await;
                node_outcomes.push(NodeOutcome::ProgramNew {
                    node_id: op.node_id.clone(),
                    outcome,
                });
            }
            SemanticOpKind::ProgramInvoke => {
                let outcome = ports
                    .composition
                    .program_invoke(CompositionRequest {
                        node_id: op.node_id.clone(),
                        program_ref: operand_str(op, "program_ref").unwrap_or_else(|| op.node_id.clone()),
                    })
                    .await;
                node_outcomes.push(NodeOutcome::ProgramInvoke {
                    node_id: op.node_id.clone(),
                    outcome,
                });
            }
            SemanticOpKind::AwaitEvent => {
                let outcome = ports
                    .events
                    .await_event(EventAwait {
                        node_id: op.node_id.clone(),
                        selector: operand_str(op, "event_selector").unwrap_or_default(),
                    })
                    .await;
                node_outcomes.push(NodeOutcome::AwaitEvent {
                    node_id: op.node_id.clone(),
                    outcome,
                });
            }
        }
    }

    let expected = ports
        .execution_commit
        .current_version(&request.version_scope)
        .await;
    seq += 1;
    batch.push(fact(
        seq,
        FactKind::InvocationCommitted,
        None,
        Some(InvocationState::CommittedReturn),
        None,
        Some(expected + 1),
    ));

    let commit = ports
        .execution_commit
        .commit(ExecutionCommitRequest {
            commit_id: request.commit_id.clone(),
            invocation_ref: request.version_scope.clone(),
            idempotency_key: format!("idem.{}", request.commit_id),
            expected_program_state_version: expected,
            write_set: request.write_set,
            evidence_batch: batch,
        })
        .await;

    Ok(RunReport {
        node_outcomes,
        native_usage,
        external_agent_evidence,
        final_context: context,
        commit,
    })
}
