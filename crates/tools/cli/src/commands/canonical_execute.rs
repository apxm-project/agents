//! Execute canonical `apxm.air.v1` through the canonical runtime driver.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use apxm_execution::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionReceiver, CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionPorts,
    ExecutionRequest, NodeOutcome, NoopStaticHookHandler, execute,
};
use apxm_inference::{
    AttemptDisposition, ExactModelTargetRef, ExactPortBindingRef, ModelBindingAdmission,
    ModelCallPreparation, ModelCallRequest, ModelCallRequestMetadata, ModelCallRequestMetadataPort,
    ModelDeploymentRef, ModelInferencePort, ModelOutcome, ModelTargetRef, ResolvedModelBinding,
    TypedError, Usage,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AtomicWriteSet, ExactPortBinding, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExternalAgentCapabilityPort, PortBundle,
    PortBundleSpec, PortImplementation, PortSlot, ProgramInstanceRef, ProgramInvocationRef,
    PromptEffectState,
};
use apxm_program::air::{AirModule, SemanticOpKind};
use apxm_program::artifact::SchemaDigestRef;
use apxm_program::external_agent::{AttributedEvent, AttributedEventKind, PeerUsage};

const DEV_BINDING_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub async fn execute_canonical_command(input: PathBuf, _json_output: bool) -> Result<()> {
    let air = load_canonical_air(&input)?;
    let request = ExecutionRequest {
        model_admission: dev_model_admission(&air),
        air,
        hook_bindings: Vec::new(),
        program_instance_ref: ProgramInstanceRef::new("dev.instance"),
        program_invocation_ref: ProgramInvocationRef::new("dev.invocation.1"),
        commit_id: "dev.commit.1".to_string(),
        write_set: dev_write_set(),
    };
    let commit = Arc::new(DevCommit::default());
    let ports = dev_ports(commit, Arc::new(UnavailableModelRequestMetadata))?;
    let report = execute(&ports, request, Value::Null)
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

fn model_targets(air: &AirModule) -> Vec<String> {
    let mut targets = Vec::new();
    for op in &air.semantic_operations {
        if op.op != SemanticOpKind::ModelCall {
            continue;
        }
        let Some(target) = op
            .operands
            .iter()
            .find(|operand| operand.slot == "model_ref")
            .map(|operand| operand.value_id.clone())
        else {
            continue;
        };
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}

fn dev_model_admission(air: &AirModule) -> ModelBindingAdmission {
    ModelBindingAdmission::for_invocation(
        model_targets(air)
            .into_iter()
            .map(|target| ResolvedModelBinding {
                model_target: ExactModelTargetRef {
                    reference: ModelTargetRef(target),
                    target_digest: digest('b'),
                },
                model_deployment_ref: ModelDeploymentRef("dev-profile.model".into()),
                exact_port_binding: ExactPortBindingRef {
                    binding_digest: DEV_BINDING_DIGEST.into(),
                    port_contract_digest: digest('c'),
                },
                composition_digest: digest('d'),
            })
            .collect(),
    )
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

struct UnavailableModelRequestMetadata;

impl ModelCallRequestMetadataPort for UnavailableModelRequestMetadata {
    fn materialize(
        &self,
        _preparation: &ModelCallPreparation,
    ) -> Result<ModelCallRequestMetadata, TypedError> {
        Err(TypedError {
            category: apxm_inference::ErrorCategory::Configuration,
            code: "model_request_metadata_unavailable".into(),
            message: "canonical local execution has no admitted model request metadata source"
                .into(),
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
            child_instance_ref: format!("dev.child.{}", request.receiver.reference()),
        }
    }

    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        // A stateful instance receiver resolves to the already-created child;
        // a program receiver is a one-shot invocation.
        let child = match &request.receiver {
            CompositionReceiver::Instance {
                program_instance_ref,
            } => program_instance_ref.clone(),
            CompositionReceiver::Program { program_ref } => format!("dev.child.{program_ref}"),
        };
        CompositionOutcome::Invoked {
            child_instance_ref: child,
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

    async fn current_version(&self, _program_instance_ref: &ProgramInstanceRef) -> u64 {
        *self.version.lock().expect("dev commit mutex poisoned")
    }
}

fn dev_ports(
    commit: Arc<DevCommit>,
    model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
) -> Result<ExecutionPorts> {
    let contract = |schema_id: &str| SchemaDigestRef {
        schema_id: schema_id.into(),
        digest: DEV_BINDING_DIGEST.into(),
    };
    let binding = |slot, schema_id| ExactPortBinding {
        slot,
        port_contract: contract(schema_id),
        binding_digest: DEV_BINDING_DIGEST.into(),
        proof_digest: DEV_BINDING_DIGEST.into(),
    };
    let spec = PortBundleSpec::new(vec![
        (
            PortSlot::ExecutionCommit,
            contract("apxm.execution-commit.v1"),
        ),
        (
            PortSlot::ModelInference,
            contract("apxm.model-inference.v1"),
        ),
        (PortSlot::Capability, contract("apxm.capability.v1")),
        (
            PortSlot::ExternalAgentCapability,
            contract("apxm.external-agent.v1"),
        ),
    ]);
    let bundle = PortBundle::construct(
        &spec,
        vec![
            (
                binding(PortSlot::ExecutionCommit, "apxm.execution-commit.v1"),
                PortImplementation::ExecutionCommit(commit),
            ),
            (
                binding(PortSlot::ModelInference, "apxm.model-inference.v1"),
                PortImplementation::ModelInference(Arc::new(DevModel)),
            ),
            (
                binding(PortSlot::Capability, "apxm.capability.v1"),
                PortImplementation::Capability(Arc::new(DevCapability)),
            ),
            (
                binding(PortSlot::ExternalAgentCapability, "apxm.external-agent.v1"),
                PortImplementation::ExternalAgentCapability(Arc::new(DevExternalAgent)),
            ),
        ],
    )?;
    Ok(ExecutionPorts::from_admitted_bundle(
        &bundle,
        model_call_request_metadata,
        Arc::new(DevEvents),
        Arc::new(DevComposition),
        Arc::new(NoopStaticHookHandler),
    )?)
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

    struct TestModelRequestMetadata;

    impl ModelCallRequestMetadataPort for TestModelRequestMetadata {
        fn materialize(
            &self,
            preparation: &ModelCallPreparation,
        ) -> Result<ModelCallRequestMetadata, TypedError> {
            Ok(ModelCallRequestMetadata {
                model_context_envelope_ref: apxm_inference::ModelContextEnvelopeRef {
                    context_id: format!(
                        "test.context.{}",
                        preparation.node_execution_id().as_str()
                    ),
                    sealed_digest: digest('e'),
                },
                idempotency: apxm_inference::IdempotencyKey {
                    key_id: format!("test.idempotency.{}", preparation.effect_id()),
                    scope_ref: "test.idempotency.scope".into(),
                },
                stream_mode: apxm_inference::ModelStreamMode::Buffered,
            })
        }
    }

    #[tokio::test]
    async fn executes_canonical_air_with_dev_ports() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}},
                {"node_id": "n.cap", "op": "capability.invoke", "parent_region_id": "r.root", "execution_order": 1, "operands": [{"slot": "capability_ref", "value_id": "cap.search", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.capability.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.capability.output", "type_ref": "CapabilityOutput"}},
                {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.root", "execution_order": 2, "operands": [{"slot": "program_ref", "value_id": "child", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
                {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.root", "execution_order": 3, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}},
                {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.root", "execution_order": 4, "operands": [{"slot": "event_ref", "value_id": "ready", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air");
        assert!(air.verify().is_accepted());

        let commit = Arc::new(DevCommit::default());
        let ports = dev_ports(commit, Arc::new(TestModelRequestMetadata))
            .expect("development ports form an admitted bundle");
        let model_admission = dev_model_admission(&air);
        let report = execute(
            &ports,
            ExecutionRequest {
                air,
                hook_bindings: Vec::new(),
                model_admission,
                program_instance_ref: ProgramInstanceRef::new("test.instance"),
                program_invocation_ref: ProgramInvocationRef::new("test.invocation"),
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

    #[tokio::test]
    async fn executes_canonical_air_without_a_model_binding() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "n.cap", "op": "capability.invoke", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "capability_ref", "value_id": "cap.search", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.capability.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.capability.output", "type_ref": "CapabilityOutput"}},
                {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.root", "execution_order": 1, "operands": [{"slot": "program_ref", "value_id": "child", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
                {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.root", "execution_order": 2, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}},
                {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.root", "execution_order": 3, "operands": [{"slot": "event_ref", "value_id": "ready", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air without a model call");
        assert!(air.verify().is_accepted());

        let model_admission = dev_model_admission(&air);
        assert!(matches!(
            model_admission.validate(&ModelTargetRef("model.absent".into())),
            Err(apxm_inference::BindingError::MissingTarget(_))
        ));

        let commit = Arc::new(DevCommit::default());
        let ports = dev_ports(commit, Arc::new(UnavailableModelRequestMetadata))
            .expect("development ports form an admitted bundle");
        let report = execute(
            &ports,
            ExecutionRequest {
                model_admission,
                air,
                hook_bindings: Vec::new(),
                program_instance_ref: ProgramInstanceRef::new("test.instance.no-model"),
                program_invocation_ref: ProgramInvocationRef::new("test.invocation.no-model"),
                commit_id: "test.commit.no-model".into(),
                write_set: dev_write_set(),
            },
            Value::Null,
        )
        .await
        .expect("canonical execution without a model binding");

        assert_eq!(report.node_outcomes.len(), 4);
        assert!(matches!(
            report.commit,
            ExecutionCommitResult::Committed { .. }
        ));
    }

    #[test]
    fn admits_each_distinct_authored_model_target_once() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "n.model.first", "op": "model.call", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.first", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.first", "type_ref": "ModelRequest"}], "result": {"value_id": "value.output.first", "type_ref": "ModelOutput"}},
                {"node_id": "n.model.second", "op": "model.call", "parent_region_id": "r.root", "execution_order": 1, "operands": [{"slot": "model_ref", "value_id": "model.target.second", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.second", "type_ref": "ModelRequest"}], "result": {"value_id": "value.output.second", "type_ref": "ModelOutput"}},
                {"node_id": "n.model.first.again", "op": "model.call", "parent_region_id": "r.root", "execution_order": 2, "operands": [{"slot": "model_ref", "value_id": "model.target.first", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.third", "type_ref": "ModelRequest"}], "result": {"value_id": "value.output.third", "type_ref": "ModelOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air with distinct model targets");
        assert!(air.verify().is_accepted());

        let admission = dev_model_admission(&air);
        for target in ["model.target.first", "model.target.second"] {
            let resolved = admission
                .validate(&ModelTargetRef(target.into()))
                .expect("authored target has exactly one local binding");
            assert_eq!(resolved.model_target.reference.0, target);
        }
    }
}
