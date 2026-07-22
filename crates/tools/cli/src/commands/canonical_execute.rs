//! Execute canonical `apxm.air.v1` through the canonical runtime driver.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use apxm_execution::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionPorts, ExecutionRequest,
    NodeOutcome, NoopStaticHookHandler, execute,
};
use apxm_inference::{
    AttemptDisposition, ExactPortBindingRef, ModelBindingAdmission, ModelCallRequest,
    ModelDeploymentRef, ModelInferencePort, ModelOutcome, ModelTargetRef, ResolvedModelBinding,
    Usage,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AtomicWriteSet, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExternalAgentCapabilityPort, PromptEffectState,
};
use apxm_program::air::{AirModule, SemanticOpKind};
use apxm_program::external_agent::{AttributedEvent, AttributedEventKind, PeerUsage};

const DEV_BINDING_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub async fn execute_canonical_command(input: PathBuf, _json_output: bool) -> Result<()> {
    let air = load_canonical_air(&input)?;
    let target = first_model_target(&air).unwrap_or_else(|| "model.default".to_string());
    let request = ExecutionRequest {
        air,
        hook_bindings: Vec::new(),
        model_admission: dev_model_admission(target),
        version_scope: "dev.instance".to_string(),
        commit_id: "dev.commit.1".to_string(),
        write_set: dev_write_set(),
    };
    let commit = Arc::new(DevCommit::default());
    let report = execute(&dev_ports(commit), request, Value::Null)
        .await
        .map_err(|err| anyhow::anyhow!(err))?;

    let output = json!({
        "schema_version": "apxm.local-execute-result.v1",
        "runtime": "apxm_execution",
        "status": "completed",
        "content": report.final_context,
        "results": {
            "node_outcomes": report.node_outcomes.iter().map(node_outcome_json).collect::<Vec<_>>(),
            "external_agent_evidence": report.external_agent_evidence,
        },
        "stats": {
            "executed_nodes": report.node_outcomes.len(),
            "failed_nodes": 0,
            "duration_ms": 0,
        },
        "llm_usage": {
            "input_tokens": report.native_usage.input_tokens,
            "output_tokens": report.native_usage.output_tokens,
            "total_requests": report
                .node_outcomes
                .iter()
                .filter(|outcome| matches!(outcome, NodeOutcome::Model { .. }))
                .count(),
        },
        "commit": commit_result_json(&report.commit),
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn load_canonical_air(input: &PathBuf) -> Result<AirModule> {
    let text = std::fs::read_to_string(input)
        .with_context(|| format!("failed to read canonical AIR from {}", input.display()))?;
    let air: AirModule = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} must contain canonical apxm.air.v1 JSON",
            input.display()
        )
    })?;
    let verdict = air.verify();
    if !verdict.is_accepted() {
        let diagnostics = verdict
            .into_diagnostics()
            .into_iter()
            .map(|diagnostic| {
                json!({
                    "code": diagnostic.code.slug(),
                    "location": diagnostic.location,
                    "message": diagnostic.message,
                })
            })
            .collect::<Vec<_>>();
        anyhow::bail!(
            "canonical AIR verification failed: {}",
            serde_json::to_string(&diagnostics)?
        );
    }
    Ok(air)
}

fn first_model_target(air: &AirModule) -> Option<String> {
    air.semantic_operations.iter().find_map(|op| {
        if op.op != SemanticOpKind::ModelCall {
            return None;
        }
        op.operands
            .as_ref()?
            .get("model_target_ref")?
            .as_str()
            .map(str::to_string)
    })
}

fn dev_model_admission(target: String) -> ModelBindingAdmission {
    ModelBindingAdmission::new(ResolvedModelBinding {
        model_target_ref: ModelTargetRef(target),
        model_deployment_ref: ModelDeploymentRef("dev-profile.model".into()),
        exact_port_binding: ExactPortBindingRef {
            binding_digest: DEV_BINDING_DIGEST.into(),
        },
    })
}

fn dev_write_set() -> AtomicWriteSet {
    AtomicWriteSet {
        next_program_state_digest: digest('1'),
        continuation_digest: digest('2'),
        checkpoint_effect_outcomes_digest: digest('3'),
        runtime_evidence_batch_digest: digest('4'),
        usage_facts_digest: digest('5'),
        session_output_refs_digest: digest('6'),
    }
}

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

struct DevModel;
impl ModelInferencePort for DevModel {
    fn attempt(&self, _request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        AttemptDisposition::Success(Usage {
            input_tokens: 1,
            output_tokens: 1,
        })
    }
}

struct DevCapability;
#[async_trait]
impl CapabilityPort for DevCapability {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::Completed {
            result: format!("capability:{}", request.capability_ref),
        }
    }
}

struct DevExternalAgent;
#[async_trait]
impl ExternalAgentCapabilityPort for DevExternalAgent {
    async fn prompt(&self, request: AcpPromptRequest) -> AcpPromptOutcome {
        AcpPromptOutcome {
            session_ref: request.session_ref,
            state: PromptEffectState::Completed {
                stop_reason: Some("dev_profile_completed".into()),
            },
            nested_events: vec![AttributedEvent {
                event_sequence: 0,
                kind: AttributedEventKind::Message,
                detail: Some("dev-profile external agent completed".into()),
                reverse_operation: None,
                reverse_target: None,
                reverse_decision: None,
            }],
            peer_usage: vec![PeerUsage {
                reported_by: "dev-profile".into(),
                metric_scope: "peer.usage.unavailable".into(),
                reported_value: "unavailable_not_observed".into(),
                availability_partial: None,
            }],
        }
    }
}

struct DevEvents;
#[async_trait]
impl EventPort for DevEvents {
    async fn await_event(&self, request: EventAwait) -> EventOutcome {
        EventOutcome::Fulfilled {
            payload: format!("event:{}", request.event_ref),
            event_ref: request.event_ref,
        }
    }
}

struct DevComposition;
#[async_trait]
impl CompositionPort for DevComposition {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Created {
            child_instance_ref: format!("dev.child.{}", request.program_ref),
        }
    }

    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Invoked {
            child_instance_ref: format!("dev.child.{}", request.program_ref),
        }
    }
}

#[derive(Default)]
struct DevCommit {
    version: Mutex<u64>,
}

#[async_trait]
impl ExecutionCommitPort for DevCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut version = self.version.lock().expect("dev commit mutex poisoned");
        if *version != request.expected_program_state_version {
            return ExecutionCommitResult::CompareConflict {
                current_program_state_version: *version,
            };
        }
        *version += 1;
        ExecutionCommitResult::Committed {
            new_program_state_version: *version,
            evidence_position_ref: "dev.evidence.1".into(),
        }
    }

    async fn current_version(&self, _invocation_ref: &str) -> u64 {
        *self.version.lock().expect("dev commit mutex poisoned")
    }
}

fn dev_ports(commit: Arc<DevCommit>) -> ExecutionPorts {
    ExecutionPorts {
        model_inference: Arc::new(DevModel),
        capability: Arc::new(DevCapability),
        external_agent: Arc::new(DevExternalAgent),
        events: Arc::new(DevEvents),
        composition: Arc::new(DevComposition),
        execution_commit: commit,
        hook_handlers: Arc::new(NoopStaticHookHandler),
    }
}

fn node_outcome_json(outcome: &NodeOutcome) -> Value {
    match outcome {
        NodeOutcome::Model {
            node_id,
            outcome,
            result,
            replaced,
        } => json!({
            "node_id": node_id,
            "kind": "model.call",
            "outcome": model_outcome_json(outcome),
            "result": result,
            "replaced": replaced,
        }),
        NodeOutcome::Capability { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "capability.invoke",
            "outcome": capability_outcome_json(outcome),
        }),
        NodeOutcome::ExternalAgent { node_id, evidence } => json!({
            "node_id": node_id,
            "kind": "capability.invoke.external_agent",
            "outcome": "completed",
            "evidence": evidence,
        }),
        NodeOutcome::ProgramNew { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "program.new",
            "outcome": composition_outcome_json(outcome),
        }),
        NodeOutcome::ProgramInvoke { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "program.invoke",
            "outcome": composition_outcome_json(outcome),
        }),
        NodeOutcome::AwaitEvent { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "await.event",
            "outcome": event_outcome_json(outcome),
        }),
    }
}

fn model_outcome_json(outcome: &ModelOutcome) -> Value {
    match outcome {
        ModelOutcome::CommittedSuccess { usage } => json!({
            "status": "committed_success",
            "usage": usage,
        }),
        ModelOutcome::TypedFailure { error } => json!({
            "status": "typed_failure",
            "error": error,
        }),
        ModelOutcome::Cancelled => json!({"status": "cancelled"}),
        ModelOutcome::ModelOutcomeUnknown { uncertain_usage } => json!({
            "status": "model_outcome_unknown",
            "uncertain_usage": uncertain_usage,
        }),
    }
}

fn capability_outcome_json(outcome: &CapabilityOutcome) -> Value {
    match outcome {
        CapabilityOutcome::Completed { result } => {
            json!({"status": "completed", "result": result})
        }
        CapabilityOutcome::Failed { message } => json!({"status": "failed", "message": message}),
        CapabilityOutcome::OutcomeUnknown { message } => {
            json!({"status": "outcome_unknown", "message": message})
        }
    }
}

fn composition_outcome_json(outcome: &CompositionOutcome) -> Value {
    match outcome {
        CompositionOutcome::Created { child_instance_ref } => {
            json!({"status": "created", "child_instance_ref": child_instance_ref})
        }
        CompositionOutcome::Invoked { child_instance_ref } => {
            json!({"status": "invoked", "child_instance_ref": child_instance_ref})
        }
        CompositionOutcome::Failed { message } => json!({"status": "failed", "message": message}),
    }
}

fn event_outcome_json(outcome: &EventOutcome) -> Value {
    match outcome {
        EventOutcome::Fulfilled { event_ref, payload } => {
            json!({"status": "fulfilled", "event_ref": event_ref, "payload": payload})
        }
        EventOutcome::Parked => json!({"status": "parked"}),
        EventOutcome::Expired => json!({"status": "expired"}),
        EventOutcome::Mismatched {
            delivered_event_ref,
        } => json!({
            "status": "mismatched",
            "delivered_event_ref": delivered_event_ref,
        }),
    }
}

fn commit_result_json(result: &ExecutionCommitResult) -> Value {
    match result {
        ExecutionCommitResult::Committed {
            new_program_state_version,
            evidence_position_ref,
        } => json!({
            "status": "committed",
            "new_program_state_version": new_program_state_version,
            "evidence_position_ref": evidence_position_ref,
        }),
        ExecutionCommitResult::CompareConflict {
            current_program_state_version,
        } => json!({
            "status": "compare_conflict",
            "current_program_state_version": current_program_state_version,
        }),
        ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => json!({
            "status": "outcome_unknown",
            "reconciliation_ref": reconciliation_ref,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn executes_canonical_air_with_dev_ports() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.root", "execution_order": 0, "operands": {"model_target_ref": "model.default"}},
                {"node_id": "n.cap", "op": "capability.invoke", "parent_region_id": "r.root", "execution_order": 1, "operands": {"capability_ref": "cap.search"}},
                {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.root", "execution_order": 2, "operands": {"program_ref": "child"}},
                {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.root", "execution_order": 3, "operands": {"program_ref": "child"}},
                {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.root", "execution_order": 4, "operands": {"event_ref": "ready"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air");

        let commit = Arc::new(DevCommit::default());
        let report = execute(
            &dev_ports(commit),
            ExecutionRequest {
                air,
                hook_bindings: Vec::new(),
                model_admission: dev_model_admission("model.default".into()),
                version_scope: "test.instance".into(),
                commit_id: "test.commit".into(),
                write_set: dev_write_set(),
            },
            Value::Null,
        )
        .await
        .expect("canonical execution");

        assert_eq!(report.node_outcomes.len(), 5);
        assert!(matches!(
            report.commit,
            ExecutionCommitResult::Committed { .. }
        ));
    }
}
