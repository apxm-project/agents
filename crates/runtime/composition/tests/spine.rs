//! The agents Composition Root assembles the first-party adapter
//! implementations into a real kernel PortBundle and drives the canonical
//! `apxm-execution` driver through a five-operation program end-to-end, using
//! deterministic fake transports (no live vLLM/ACP/Server infra).

use std::sync::Arc;

use apxm_adapter_acp::{
    AdmittedProfiles, CeilingAuthorizer, ExternalAgentCapabilityAdapter, FakeAcpPeer, PeerEvent,
    PeerUsageEvidence, ReverseRequest, end_turn_exchange,
};
use apxm_adapter_runtime::{
    AdmittedSandboxConfinement, ConfinementType, FixedVllmTransport, InMemoryExecutionCommit,
    VllmHttpInferenceClient, VllmModelOutcome, VllmNativeUsage,
};

use apxm_composition::{AdmittedBinding, AdmittedPorts, assemble_bundle, execution_ports};
use apxm_execution::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionRequest, NodeOutcome,
    NoopStaticHookHandler, execute,
};
use apxm_inference::{
    ExactPortBindingRef, ModelBindingAdmission, ModelDeploymentRef, ModelInferencePort,
    ModelOutcome, ModelTargetRef, ResolvedModelBinding,
};
use apxm_kernel::{
    AtomicWriteSet, ConfinementPort, ExactPortBinding, ExecutionCommitPort,
    ExternalAgentCapabilityPort, PortSlot,
};
use apxm_program::air::{AirModule, AirVersion, SemanticOp, SemanticOpKind};
use apxm_program::artifact::SchemaDigestRef;
use apxm_program::source_map::{SourceLanguage, SourceMap, SourceMapVersion};

use serde_json::{Map, Value};

const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DIGEST_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const MODEL_TARGET: &str = "gpt-oss-120b";

fn contract(schema_id: &str) -> SchemaDigestRef {
    SchemaDigestRef {
        schema_id: schema_id.into(),
        digest: DIGEST_A.into(),
    }
}

fn admitted_binding(slot: PortSlot, schema_id: &str) -> AdmittedBinding {
    AdmittedBinding {
        slot,
        port_contract: contract(schema_id),
        binding: ExactPortBinding {
            slot,
            port_contract: contract(schema_id),
            binding_digest: DIGEST_B.into(),
            proof_digest: DIGEST_C.into(),
        },
    }
}

fn operands(pairs: &[(&str, &str)]) -> Option<Map<String, Value>> {
    let mut map = Map::new();
    for (k, v) in pairs {
        map.insert((*k).to_string(), Value::String((*v).to_string()));
    }
    Some(map)
}

fn five_op_air() -> AirModule {
    AirModule {
        schema_version: AirVersion::V1,
        semantic_operations: vec![
            SemanticOp {
                node_id: "n_model".into(),
                op: SemanticOpKind::ModelCall,
                operands: operands(&[("model_target_ref", MODEL_TARGET)]),
            },
            SemanticOp {
                node_id: "n_external_agent".into(),
                op: SemanticOpKind::CapabilityInvoke,
                operands: operands(&[
                    ("capability_ref", "external-agent:acp:claude-code"),
                    ("external_agent_session", "conn_1"),
                ]),
            },
            SemanticOp {
                node_id: "n_program_new".into(),
                op: SemanticOpKind::ProgramNew,
                operands: operands(&[("program_ref", "child_program")]),
            },
            SemanticOp {
                node_id: "n_program_invoke".into(),
                op: SemanticOpKind::ProgramInvoke,
                operands: operands(&[("program_ref", "child_program")]),
            },
            SemanticOp {
                node_id: "n_await".into(),
                op: SemanticOpKind::AwaitEvent,
                operands: operands(&[("event_ref", "webhook_ready")]),
            },
        ],
        structural_ir: vec![],
        source_map: SourceMap {
            schema_version: SourceMapVersion::V1,
            source_language: SourceLanguage::Python,
            node_spans: vec![],
            region_annotations: vec![],
        },
    }
}

fn write_set() -> AtomicWriteSet {
    AtomicWriteSet {
        next_program_state_digest: DIGEST_A.into(),
        continuation_digest: DIGEST_B.into(),
        checkpoint_effect_outcomes_digest: DIGEST_C.into(),
        runtime_evidence_batch_digest: DIGEST_A.into(),
        usage_facts_digest: DIGEST_B.into(),
        session_output_refs_digest: DIGEST_C.into(),
    }
}

fn model_admission() -> ModelBindingAdmission {
    ModelBindingAdmission::new(ResolvedModelBinding {
        model_target_ref: ModelTargetRef(MODEL_TARGET.into()),
        model_deployment_ref: ModelDeploymentRef("vllm-deploy-1".into()),
        exact_port_binding: ExactPortBindingRef {
            binding_digest: DIGEST_B.into(),
        },
    })
}

struct TestCapability;
#[async_trait::async_trait]
impl CapabilityPort for TestCapability {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::Completed {
            result: format!("capability:{}", request.capability_ref),
        }
    }
}

struct TestEvents;
#[async_trait::async_trait]
impl EventPort for TestEvents {
    async fn await_event(&self, request: EventAwait) -> EventOutcome {
        EventOutcome::Fulfilled {
            payload: format!("event:{}", request.event_ref),
            event_ref: request.event_ref,
        }
    }
}

struct TestComposition;
#[async_trait::async_trait]
impl CompositionPort for TestComposition {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Created {
            child_instance_ref: format!("child:{}", request.program_ref),
        }
    }
    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Invoked {
            child_instance_ref: format!("child:{}", request.program_ref),
        }
    }
}

fn admitted_ports() -> AdmittedPorts {
    let execution_commit: Arc<dyn ExecutionCommitPort> = Arc::new(InMemoryExecutionCommit::new());
    let confinement: Arc<dyn ConfinementPort> = Arc::new(AdmittedSandboxConfinement::new(
        &[(ConfinementType::Gvisor, DIGEST_A)],
        "host-signer-1",
        "2026-07-19T00:00:00Z",
    ));
    let model_inference: Arc<dyn ModelInferencePort + Send + Sync> =
        Arc::new(VllmHttpInferenceClient::new(FixedVllmTransport::new(
            VllmModelOutcome::CommittedSuccess {
                usage: VllmNativeUsage {
                    native_input_tokens: 42,
                    native_output_tokens: 100,
                },
            },
        )));
    let profile = AdmittedProfiles::v1()
        .get("acp:claude-code")
        .expect("claude-code admitted")
        .clone();
    let peer = FakeAcpPeer::new(end_turn_exchange(vec![
        PeerEvent::Message("planning".into()),
        PeerEvent::Reverse(ReverseRequest::PathRead("/workspace/src/lib.rs".into())),
        PeerEvent::Usage(PeerUsageEvidence::new(
            "acp:claude-code",
            "peer.tokens.total",
            "512",
        )),
    ]));
    let ceiling = CeilingAuthorizer::from_ceiling(
        &["/workspace"],
        &["git"],
        &["api.github.com"],
        &["read_file"],
    );
    let external_agent: Arc<dyn ExternalAgentCapabilityPort> =
        Arc::new(ExternalAgentCapabilityAdapter::new(peer, ceiling, profile));

    AdmittedPorts {
        execution_commit,
        confinement,
        model_inference,
        external_agent,
        hook_handlers: Arc::new(NoopStaticHookHandler),
    }
}

#[test]
fn composition_root_assembles_a_real_kernel_bundle() {
    let ports = admitted_ports();
    let bundle = assemble_bundle(
        [
            admitted_binding(PortSlot::ExecutionCommit, "apxm.execution-commit.v1"),
            admitted_binding(PortSlot::Confinement, "apxm.confinement-attestation.v1"),
            admitted_binding(PortSlot::ModelInference, "apxm.vllm-inference.v1"),
            admitted_binding(
                PortSlot::ExternalAgentCapability,
                "apxm.external-agent-evidence.v1",
            ),
        ],
        &ports,
    )
    .expect("adapters assemble into a real kernel PortBundle");
    assert!(bundle.confinement().is_some());
    assert!(bundle.model_inference().is_some());
    assert!(bundle.external_agent_capability().is_some());
}

#[tokio::test]
async fn composition_root_drives_canonical_execution_end_to_end() {
    let admitted = admitted_ports();
    let ports = execution_ports(
        &admitted,
        Arc::new(TestCapability),
        Arc::new(TestEvents),
        Arc::new(TestComposition),
    );

    let report = execute(
        &ports,
        ExecutionRequest {
            air: five_op_air(),
            hook_bindings: Vec::new(),
            model_admission: model_admission(),
            version_scope: "invoke_1".into(),
            commit_id: "commit_1".into(),
            write_set: write_set(),
        },
        Value::Null,
    )
    .await
    .expect("canonical execution runs end-to-end through the real adapters");

    // All five operations executed through their exact injected ports.
    assert_eq!(report.node_outcomes.len(), 5);
    assert!(matches!(report.node_outcomes[0], NodeOutcome::Model { .. }));
    assert!(matches!(
        report.node_outcomes[1],
        NodeOutcome::ExternalAgent { .. }
    ));
    assert!(matches!(
        report.node_outcomes[4],
        NodeOutcome::AwaitEvent { .. }
    ));

    // Native model usage comes from the vLLM client; peer usage stays isolated
    // in External Agent evidence and never enters native accounting.
    assert_eq!(report.native_usage.input_tokens, 42);
    assert_eq!(report.native_usage.output_tokens, 100);
    assert_eq!(report.external_agent_evidence.len(), 1);
    assert_eq!(
        report.external_agent_evidence[0].peer_usage[0].reported_value,
        "512"
    );

    // The whole run committed atomically through the one Execution Commit port.
    assert!(matches!(
        report.commit,
        apxm_kernel::ExecutionCommitResult::Committed { .. }
    ));
    if let NodeOutcome::Model { outcome, .. } = &report.node_outcomes[0] {
        assert!(matches!(outcome, ModelOutcome::CommittedSuccess { .. }));
    }
}
