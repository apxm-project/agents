//! End-to-end canonical execution: drive a five-operation AIR program through
//! the injected kernel ports and commit atomically. Deterministic in-crate fakes
//! stand in for the admitted ports (test doubles, non-admissible).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use apxm_inference::{
    AttemptDisposition, ExactPortBindingRef, ModelBindingAdmission, ModelCallRequest,
    ModelDeploymentRef, ModelInferencePort, ModelOutcome, ModelTargetRef, ResolvedModelBinding,
    Usage,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AtomicWriteSet, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExternalAgentCapabilityPort, PromptEffectState,
};
use apxm_program::air::AirModule;
use apxm_program::external_agent::{AttributedEvent, AttributedEventKind, PeerUsage};
use apxm_program::frontend_graph::{HookBinding, HookPhase, HookReturnMode, HookScope};
use apxm_program::runtime_evidence::Fact;

use apxm_execution::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionPorts, ExecutionRequest,
    NodeOutcome, StaticHookHandlerPort, StaticHookResult, execute,
};

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn write_set() -> AtomicWriteSet {
    AtomicWriteSet {
        next_program_state_digest: digest('1'),
        continuation_digest: digest('2'),
        checkpoint_effect_outcomes_digest: digest('3'),
        runtime_evidence_batch_digest: digest('4'),
        usage_facts_digest: digest('5'),
        session_output_refs_digest: digest('6'),
    }
}

fn air() -> AirModule {
    serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "n.model", "op": "model.call", "operands": {"model_target_ref": "model.default"}},
            {"node_id": "n.cap", "op": "capability.invoke", "operands": {"capability_ref": "cap.search"}},
            {"node_id": "n.acp", "op": "capability.invoke", "operands": {"capability_ref": "external-agent:acp:claude-code", "external_agent_session": "session.1"}},
            {"node_id": "n.new", "op": "program.new", "operands": {"program_ref": "Specialist"}},
            {"node_id": "n.invoke", "op": "program.invoke", "operands": {"program_ref": "Specialist"}},
            {"node_id": "n.await", "op": "await.event", "operands": {"event_ref": "evt.done"}}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function"},
            {"region_id": "r.return", "kind": "return"}
        ],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("valid AIR")
}

fn admission() -> ModelBindingAdmission {
    ModelBindingAdmission::new(ResolvedModelBinding {
        model_target_ref: ModelTargetRef("model.default".into()),
        model_deployment_ref: ModelDeploymentRef("deploy.default".into()),
        exact_port_binding: ExactPortBindingRef {
            binding_digest: digest('a'),
        },
    })
}

struct FakeModel;
impl ModelInferencePort for FakeModel {
    fn attempt(&self, _request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        AttemptDisposition::Success(Usage {
            input_tokens: 10,
            output_tokens: 20,
        })
    }
}

struct FakeCapability;
#[async_trait]
impl CapabilityPort for FakeCapability {
    async fn invoke(&self, _request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::Completed {
            result: "ok".into(),
        }
    }
}

struct FakeAcpPeer;
#[async_trait]
impl ExternalAgentCapabilityPort for FakeAcpPeer {
    async fn prompt(&self, request: AcpPromptRequest) -> AcpPromptOutcome {
        AcpPromptOutcome {
            session_ref: request.session_ref,
            state: PromptEffectState::Completed {
                stop_reason: Some("end_turn".into()),
            },
            nested_events: vec![
                AttributedEvent {
                    event_sequence: 0,
                    kind: AttributedEventKind::Message,
                    detail: None,
                    reverse_operation: None,
                    reverse_target: None,
                    reverse_decision: None,
                },
                AttributedEvent {
                    event_sequence: 1,
                    kind: AttributedEventKind::ToolCall,
                    detail: None,
                    reverse_operation: None,
                    reverse_target: None,
                    reverse_decision: None,
                },
            ],
            peer_usage: vec![PeerUsage {
                reported_by: "acp:claude-code".into(),
                metric_scope: "peer.tokens.total".into(),
                reported_value: "555".into(),
                availability_partial: None,
            }],
        }
    }
}

struct FakeEvents;
#[async_trait]
impl EventPort for FakeEvents {
    async fn await_event(&self, request: EventAwait) -> EventOutcome {
        EventOutcome::Fulfilled {
            event_ref: request.event_ref,
            payload: "done".into(),
        }
    }
}

struct FakeComposition;
#[async_trait]
impl CompositionPort for FakeComposition {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Created {
            child_instance_ref: format!("child.{}", request.program_ref),
        }
    }
    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Invoked {
            child_instance_ref: format!("child.{}", request.program_ref),
        }
    }
}

struct StaticHooks;
#[async_trait]
impl StaticHookHandlerPort for StaticHooks {
    async fn execute(
        &self,
        binding: &HookBinding,
        _context: &Value,
        _result: &Value,
    ) -> StaticHookResult {
        assert_eq!(binding.handler_ref, "hooks.after_model");
        StaticHookResult::Replace {
            assigned_context: Some(json!({"turns": 1})),
            result: json!("hooked"),
        }
    }
}

struct FakeCommit {
    state: Mutex<(u64, Vec<Fact>)>,
}
impl FakeCommit {
    fn new() -> Self {
        Self {
            state: Mutex::new((0, Vec::new())),
        }
    }
    fn facts(&self) -> Vec<Fact> {
        self.state.lock().unwrap().1.clone()
    }
}
#[async_trait]
impl ExecutionCommitPort for FakeCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut state = self.state.lock().unwrap();
        state.0 = request.expected_program_state_version + 1;
        state.1.extend(request.evidence_batch);
        ExecutionCommitResult::Committed {
            new_program_state_version: state.0,
            evidence_position_ref: "evidence:1".into(),
        }
    }
    async fn current_version(&self, _invocation_ref: &str) -> u64 {
        self.state.lock().unwrap().0
    }
}

fn ports(commit: Arc<FakeCommit>) -> ExecutionPorts {
    ExecutionPorts {
        model_inference: Arc::new(FakeModel),
        capability: Arc::new(FakeCapability),
        external_agent: Arc::new(FakeAcpPeer),
        events: Arc::new(FakeEvents),
        composition: Arc::new(FakeComposition),
        execution_commit: commit,
        hook_handlers: Arc::new(StaticHooks),
    }
}

fn request() -> ExecutionRequest {
    ExecutionRequest {
        air: air(),
        hook_bindings: vec![HookBinding {
            hook_id: "hook.after.model".into(),
            scope: HookScope::Model,
            phase: HookPhase::After,
            target_selector: "n.model".into(),
            declaration_order: 0,
            handler_ref: "hooks.after_model".into(),
            handler_digest: digest('b'),
            input_type_ref: "ModelResult".into(),
            output_type_ref: "ModelResult".into(),
            return_mode: HookReturnMode::ReplaceResult,
        }],
        model_admission: admission(),
        version_scope: "instance.1".into(),
        commit_id: "c1".into(),
        write_set: write_set(),
    }
}

#[tokio::test]
async fn executes_all_five_ops_and_commits_atomically() {
    let commit = Arc::new(FakeCommit::new());

    let report = execute(
        &ports(commit.clone()),
        request(),
        json!({"turns": 0}),
    )
    .await
    .expect("run");

    // All six nodes executed through their exact ports.
    assert_eq!(report.node_outcomes.len(), 6);
    assert!(matches!(report.node_outcomes[0], NodeOutcome::Model { .. }));
    assert!(matches!(
        report.node_outcomes[1],
        NodeOutcome::Capability { .. }
    ));
    assert!(matches!(
        report.node_outcomes[2],
        NodeOutcome::ExternalAgent { .. }
    ));
    assert!(matches!(
        report.node_outcomes[3],
        NodeOutcome::ProgramNew { .. }
    ));
    assert!(matches!(
        report.node_outcomes[4],
        NodeOutcome::ProgramInvoke { .. }
    ));
    assert!(matches!(
        report.node_outcomes[5],
        NodeOutcome::AwaitEvent { .. }
    ));

    // One atomic commit for the whole run.
    assert_eq!(
        report.commit,
        ExecutionCommitResult::Committed {
            new_program_state_version: 1,
            evidence_position_ref: "evidence:1".into()
        }
    );

    // model.call produced native usage; the Hook replaced the result and set context.
    assert_eq!(
        report.native_usage,
        Usage {
            input_tokens: 10,
            output_tokens: 20
        }
    );
    if let NodeOutcome::Model {
        result,
        replaced,
        outcome,
        ..
    } = &report.node_outcomes[0]
    {
        assert_eq!(*result, json!("hooked"));
        assert!(replaced);
        assert!(matches!(outcome, ModelOutcome::CommittedSuccess { .. }));
    }
    assert_eq!(report.final_context, json!({"turns": 1}));

    // Peer usage is isolated in External Agent evidence, never in native usage.
    assert_eq!(report.external_agent_evidence.len(), 1);
    assert_eq!(
        report.external_agent_evidence[0].peer_usage[0].reported_value,
        "555"
    );
    assert_eq!(
        report.native_usage.input_tokens, 10,
        "peer 555 never enters native usage"
    );

    // The committed evidence records one attempt per node, the static handler,
    // its explicit Context transition, and lifecycle facts.
    assert_eq!(commit.facts().len(), 2 + 6 + 1 + 1 + 1);
    assert!(commit
        .facts()
        .iter()
        .any(|fact| fact.fact_kind == apxm_program::runtime_evidence::FactKind::HookExecuted));
    assert!(commit
        .facts()
        .iter()
        .any(|fact| fact.fact_kind == apxm_program::runtime_evidence::FactKind::ContextTransitioned));
}

#[tokio::test]
async fn unbound_model_target_fails_closed() {
    let commit = Arc::new(FakeCommit::new());
    let bad_air: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "n.model", "op": "model.call", "operands": {"model_target_ref": "model.unbound"}}
        ],
        "structural_ir": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .unwrap();
    let request = ExecutionRequest {
        air: bad_air,
        hook_bindings: Vec::new(),
        model_admission: admission(),
        version_scope: "instance.1".into(),
        commit_id: "c1".into(),
        write_set: write_set(),
    };
    let err = execute(&ports(commit), request, json!(null))
        .await
        .expect_err("no binding");
    assert!(matches!(err, apxm_execution::ExecutionError::Binding(_)));
}
