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
    CompositionReceiver, CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionError,
    ExecutionPorts, ExecutionRequest, NodeOutcome, StaticHookHandlerPort, StaticHookResult,
    execute,
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
            {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.default", "type_ref": "ModelTargetRef"}]},
            {"node_id": "n.cap", "op": "capability.invoke", "parent_region_id": "r.fn", "execution_order": 1, "operands": [{"slot": "capability_ref", "value_id": "cap.search", "type_ref": "CapabilityRef"}]},
            {"node_id": "n.acp", "op": "capability.invoke", "parent_region_id": "r.fn", "execution_order": 2, "operands": [{"slot": "capability_ref", "value_id": "external-agent:acp:claude-code", "type_ref": "CapabilityRef"}, {"slot": "external_agent_session", "value_id": "session.1", "type_ref": "ExternalAgentSessionRef"}]},
            {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.fn", "execution_order": 3, "operands": [{"slot": "program_ref", "value_id": "Specialist", "type_ref": "ProgramRef"}]},
            {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.fn", "execution_order": 4, "operands": [{"slot": "receiver", "value_id": "n.new", "type_ref": "ProgramInstanceRef"}]},
            {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.fn", "execution_order": 5, "operands": [{"slot": "event_ref", "value_id": "evt.done", "type_ref": "EventRef"}]}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0},
            {"region_id": "r.return", "kind": "return", "parent_region_id": "r.fn", "execution_order": 6}
        ],
        "context_flow": [],
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
            child_instance_ref: format!("child.{}", request.receiver.reference()),
        }
    }
    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Invoked {
            child_instance_ref: format!("child.{}", request.receiver.reference()),
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

    let report = execute(&ports(commit.clone()), request(), json!({"turns": 0}))
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
    // its explicit Context transition, lifecycle facts, and one ChildAttached
    // lineage fact for each of program.new and program.invoke.
    assert_eq!(commit.facts().len(), 2 + 6 + 1 + 1 + 1 + 2);
    assert!(
        commit
            .facts()
            .iter()
            .any(|fact| fact.is_kind(apxm_program::runtime_evidence::FactKind::HookExecuted))
    );
    assert!(commit
        .facts()
        .iter()
        .any(|fact| fact.is_kind(apxm_program::runtime_evidence::FactKind::ContextTransitioned)));
    // Cross-instance lineage: program.new and program.invoke each emit a
    // ChildAttached fact, and the invoke's fact carries parent lineage back to
    // the created instance's NodeExecution.
    let committed_facts = commit.facts();
    let child_attached: Vec<_> = committed_facts
        .iter()
        .filter(|fact| fact.is_kind(apxm_program::runtime_evidence::FactKind::ChildAttached))
        .collect();
    assert_eq!(
        child_attached.len(),
        2,
        "one ChildAttached per composition op"
    );
    assert!(
        child_attached.iter().any(|fact| fact
            .runtime()
            .and_then(|r| r.parent_node_execution_id.as_ref())
            .is_some()),
        "program.invoke ChildAttached carries parent lineage"
    );
}

#[tokio::test]
async fn repeated_instance_invocations_keep_evidence_identities_disjoint() {
    let commit = Arc::new(FakeCommit::new());
    execute(&ports(commit.clone()), request(), json!({"invocation": 1}))
        .await
        .expect("first invocation");

    let mut second = request();
    second.commit_id = "c2".into();
    execute(&ports(commit.clone()), second, json!({"invocation": 2}))
        .await
        .expect("second invocation");

    let facts = commit.facts();
    let fact_ids: std::collections::HashSet<_> = facts.iter().map(Fact::fact_id).collect();
    assert_eq!(fact_ids.len(), facts.len());
    assert!(fact_ids.iter().any(|fact_id| fact_id.contains(".c1.")));
    assert!(fact_ids.iter().any(|fact_id| fact_id.contains(".c2.")));
}

/// A composition port that records every receiver it is handed, so a test can
/// prove `program.invoke` dispatched against the created ProgramInstanceRef
/// rather than the retired top-level `program_ref` operand or a node-id fallback.
struct RecordingComposition {
    receivers: Arc<Mutex<Vec<CompositionReceiver>>>,
}
#[async_trait]
impl CompositionPort for RecordingComposition {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome {
        self.receivers
            .lock()
            .unwrap()
            .push(request.receiver.clone());
        CompositionOutcome::Created {
            child_instance_ref: format!("child.{}", request.receiver.reference()),
        }
    }
    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        self.receivers
            .lock()
            .unwrap()
            .push(request.receiver.clone());
        CompositionOutcome::Invoked {
            child_instance_ref: request.receiver.reference().to_string(),
        }
    }
}

#[tokio::test]
async fn program_invoke_dispatches_to_the_created_instance_receiver() {
    let commit = Arc::new(FakeCommit::new());
    let receivers = Arc::new(Mutex::new(Vec::new()));
    let air: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "program_ref", "value_id": "Specialist", "type_ref": "ProgramRef"}]},
            {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.fn", "execution_order": 1, "operands": [{"slot": "receiver", "value_id": "n.new", "type_ref": "ProgramInstanceRef"}]}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .unwrap();
    let mut ports = ports(commit.clone());
    ports.composition = Arc::new(RecordingComposition {
        receivers: receivers.clone(),
    });
    let mut req = request();
    req.air = air;
    req.hook_bindings = Vec::new();
    execute(&ports, req, json!({}))
        .await
        .expect("composition executes");

    let seen = receivers.lock().unwrap();
    assert_eq!(seen.len(), 2, "program.new then program.invoke");
    assert_eq!(
        seen[0],
        CompositionReceiver::Program {
            program_ref: "Specialist".into()
        },
        "program.new resolves its ProgramRef operand"
    );
    assert_eq!(
        seen[1],
        CompositionReceiver::Instance {
            program_instance_ref: "n.new".into()
        },
        "program.invoke dispatches to the created instance, not a program_ref fallback"
    );
}

#[tokio::test]
async fn program_invoke_without_a_receiver_fails_closed() {
    let commit = Arc::new(FakeCommit::new());
    let air: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.fn", "execution_order": 0}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .unwrap();
    let mut req = request();
    req.air = air;
    req.hook_bindings = Vec::new();
    let result = execute(&ports(commit.clone()), req, json!({})).await;
    assert!(
        matches!(
            result,
            Err(ExecutionError::MissingOperand {
                operand: "receiver",
                ..
            })
        ),
        "a program.invoke with no receiver must fail closed, not fall back"
    );
}

#[tokio::test]
async fn unbound_model_target_fails_closed() {
    let commit = Arc::new(FakeCommit::new());
    let bad_air: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.unbound", "type_ref": "ModelTargetRef"}]}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0}
        ],
        "context_flow": [],
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
