//! End-to-end canonical execution: drive a five-operation AIR program through
//! the injected kernel ports and commit atomically. Deterministic in-crate fakes
//! stand in for the admitted ports (test doubles, non-admissible).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use apxm_inference::{
    AttemptDisposition, ExactModelTargetRef, ExactPortBindingRef, IdempotencyKey,
    ModelBindingAdmission, ModelCallPreparation, ModelCallRequest, ModelCallRequestMetadata,
    ModelCallRequestMetadataPort, ModelContextEnvelopeRef, ModelDeploymentRef, ModelInferencePort,
    ModelOutcome, ModelStreamMode, ModelTargetRef, ResolvedModelBinding, TypedError, Usage,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AtomicWriteSet, ExactPortBinding, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExternalAgentCapabilityPort, PortBundle,
    PortBundleSpec, PortImplementation, PortSlot, ProgramInstanceRef, ProgramInvocationRef,
    PromptEffectState,
};
use apxm_program::air::AirModule;
use apxm_program::artifact::SchemaDigestRef;
use apxm_program::capability::CapabilityInvocationAuthority;
use apxm_program::external_agent::{AttributedEvent, AttributedEventKind, PeerUsage};
use apxm_program::frontend_graph::{HookBinding, HookPhase, HookReturnMode, HookScope};
use apxm_program::runtime_evidence::Fact;

use apxm_execution::{
    CapabilityInvocationAdmission, CapabilityOutcome, CapabilityPort, CapabilityRequest,
    CommittedNativeModelUsage, CommittedNativeModelUsageError, CommittedNativeModelUsageOutcome,
    CommittedNativeModelUsagePort, CompositionOutcome, CompositionPort, CompositionReceiver,
    CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionError, ExecutionPortBundle,
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
    let air: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}},
            {"node_id": "n.cap", "op": "capability.invoke", "parent_region_id": "r.fn", "execution_order": 1, "operands": [{"slot": "capability_ref", "value_id": "cap.search", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.cap.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.cap.output", "type_ref": "CapabilityOutput"}},
            {"node_id": "n.acp", "op": "capability.invoke", "parent_region_id": "r.fn", "execution_order": 2, "operands": [{"slot": "capability_ref", "value_id": "external-agent:acp:claude-code", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "session.1", "type_ref": "ExternalAgentSessionRef"}], "result": {"value_id": "value.acp.output", "type_ref": "ExternalAgentOutput"}},
            {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.fn", "execution_order": 3, "operands": [{"slot": "program_ref", "value_id": "Specialist", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
            {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.fn", "execution_order": 4, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}},
            {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.fn", "execution_order": 5, "operands": [{"slot": "event_ref", "value_id": "evt.done", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0},
            {"region_id": "r.return", "kind": "return", "parent_region_id": "r.fn", "execution_order": 6}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("valid AIR");
    assert!(air.verify().is_accepted());
    air
}

fn admission() -> ModelBindingAdmission {
    ModelBindingAdmission::new(ResolvedModelBinding {
        model_target: ExactModelTargetRef {
            reference: ModelTargetRef("model.target.v1".into()),
            target_digest: digest('9'),
        },
        model_deployment_ref: ModelDeploymentRef("deploy.default".into()),
        exact_port_binding: ExactPortBindingRef {
            binding_digest: digest('a'),
            port_contract_digest: digest('b'),
        },
        composition_digest: digest('c'),
    })
}

struct TestModelRequestMetadata;

impl ModelCallRequestMetadataPort for TestModelRequestMetadata {
    fn materialize(
        &self,
        _preparation: &ModelCallPreparation,
    ) -> Result<ModelCallRequestMetadata, TypedError> {
        Ok(ModelCallRequestMetadata {
            model_context_envelope_ref: ModelContextEnvelopeRef {
                context_id: "context.test".into(),
                sealed_digest: digest('d'),
            },
            idempotency: IdempotencyKey {
                key_id: "idempotency.test".into(),
                scope_ref: "scope.test".into(),
            },
            stream_mode: ModelStreamMode::Buffered,
        })
    }
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

#[derive(Default)]
struct RetryingModel {
    request_identities: Mutex<Vec<(String, String)>>,
}

impl RetryingModel {
    fn request_identities(&self) -> Vec<(String, String)> {
        self.request_identities.lock().unwrap().clone()
    }
}

impl ModelInferencePort for RetryingModel {
    fn attempt(&self, request: &ModelCallRequest, attempt: u32) -> AttemptDisposition {
        self.request_identities.lock().unwrap().push((
            request.effect_id().to_string(),
            request.request_digest().to_string(),
        ));
        if attempt == 0 {
            AttemptDisposition::FailedBeforeSend(apxm_inference::TypedError {
                category: apxm_inference::ErrorCategory::Unavailable,
                code: "retryable".into(),
                message: "retry".into(),
            })
        } else {
            AttemptDisposition::Success(Usage {
                input_tokens: 7,
                output_tokens: 11,
            })
        }
    }
}

struct ZeroUsageModel;
impl ModelInferencePort for ZeroUsageModel {
    fn attempt(&self, _request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        AttemptDisposition::Success(Usage::default())
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

#[derive(Default)]
struct RecordingCapability {
    requests: Mutex<Vec<CapabilityRequest>>,
}

impl RecordingCapability {
    fn requests(&self) -> Vec<CapabilityRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl CapabilityPort for RecordingCapability {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome {
        self.requests.lock().unwrap().push(request);
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
            assigned_context: Some(json!({"iterations": 1})),
            result: json!("hooked"),
        }
    }
}

struct FakeCommit {
    state: Mutex<(u64, Vec<Fact>)>,
    invocation_refs: Mutex<Vec<String>>,
    fail: bool,
}
impl FakeCommit {
    fn new() -> Self {
        Self {
            state: Mutex::new((0, Vec::new())),
            invocation_refs: Mutex::new(Vec::new()),
            fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            state: Mutex::new((0, Vec::new())),
            invocation_refs: Mutex::new(Vec::new()),
            fail: true,
        }
    }
    fn facts(&self) -> Vec<Fact> {
        self.state.lock().unwrap().1.clone()
    }

    fn invocation_refs(&self) -> Vec<String> {
        self.invocation_refs.lock().unwrap().clone()
    }
}
#[async_trait]
impl ExecutionCommitPort for FakeCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        if self.fail {
            return ExecutionCommitResult::CompareConflict {
                current_program_state_version: request.expected_program_state_version + 1,
            };
        }
        let mut state = self.state.lock().unwrap();
        self.invocation_refs
            .lock()
            .unwrap()
            .push(request.program_invocation_ref.as_str().to_string());
        state.0 = request.expected_program_state_version + 1;
        state.1.extend(request.evidence_batch);
        ExecutionCommitResult::Committed {
            new_program_state_version: state.0,
            evidence_position_ref: "evidence:1".into(),
        }
    }
    async fn current_version(&self, _program_instance_ref: &ProgramInstanceRef) -> u64 {
        self.state.lock().unwrap().0
    }

    /// The end-to-end fixture runs to completion, so it parks no continuation.
    async fn load_continuation(&self, _program_instance_ref: &ProgramInstanceRef) -> Option<Value> {
        None
    }
}

#[derive(Default)]
struct RecordingOperationalUsage {
    calls: Mutex<Vec<CommittedNativeModelUsage>>,
    fail: Option<CommittedNativeModelUsageError>,
}

impl RecordingOperationalUsage {
    fn failing(error: CommittedNativeModelUsageError) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: Some(error),
        }
    }

    fn calls(&self) -> Vec<CommittedNativeModelUsage> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl CommittedNativeModelUsagePort for RecordingOperationalUsage {
    async fn publish(
        &self,
        request: CommittedNativeModelUsage,
    ) -> Result<(), CommittedNativeModelUsageError> {
        self.calls.lock().unwrap().push(request);
        match &self.fail {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

fn ports(commit: Arc<FakeCommit>) -> ExecutionPorts {
    ports_with_model_composition_and_capability(
        commit,
        Arc::new(FakeModel),
        Arc::new(FakeComposition),
        Arc::new(FakeCapability),
    )
}

fn ports_with_composition(
    commit: Arc<FakeCommit>,
    composition: Arc<dyn CompositionPort>,
) -> ExecutionPorts {
    ports_with_model_composition_and_capability(
        commit,
        Arc::new(FakeModel),
        composition,
        Arc::new(FakeCapability),
    )
}

fn ports_with_model(
    commit: Arc<FakeCommit>,
    model: Arc<dyn ModelInferencePort + Send + Sync>,
) -> ExecutionPorts {
    ports_with_model_composition_and_capability(
        commit,
        model,
        Arc::new(FakeComposition),
        Arc::new(FakeCapability),
    )
}

fn ports_with_model_composition_and_capability(
    commit: Arc<FakeCommit>,
    model: Arc<dyn ModelInferencePort + Send + Sync>,
    composition: Arc<dyn CompositionPort>,
    capability: Arc<dyn CapabilityPort>,
) -> ExecutionPorts {
    let contract = |schema_id: &str| SchemaDigestRef {
        schema_id: schema_id.into(),
        digest: digest('e'),
    };
    let binding = |slot, schema_id| ExactPortBinding {
        slot,
        port_contract: contract(schema_id),
        binding_digest: digest('b'),
        proof_digest: digest('c'),
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
        (
            PortSlot::Capability,
            contract("apxm.capability-invocation.v1"),
        ),
        (
            PortSlot::ExternalAgentCapability,
            contract("apxm.external-agent.v1"),
        ),
    ]);
    let kernel_bundle = PortBundle::construct(
        &spec,
        vec![
            (
                binding(PortSlot::ExecutionCommit, "apxm.execution-commit.v1"),
                PortImplementation::ExecutionCommit(commit),
            ),
            (
                binding(PortSlot::ModelInference, "apxm.model-inference.v1"),
                PortImplementation::ModelInference(model),
            ),
            (
                binding(PortSlot::Capability, "apxm.capability-invocation.v1"),
                PortImplementation::Capability(capability),
            ),
            (
                binding(PortSlot::ExternalAgentCapability, "apxm.external-agent.v1"),
                PortImplementation::ExternalAgentCapability(Arc::new(FakeAcpPeer)),
            ),
        ],
    )
    .expect("test ports satisfy the admitted bundle");
    let bundle = ExecutionPortBundle::construct(
        Arc::new(kernel_bundle),
        contract("apxm.durable-event.v1"),
        binding(PortSlot::DurableEvent, "apxm.durable-event.v1"),
        Arc::new(FakeEvents),
        contract("apxm.program-composition.v1"),
        binding(PortSlot::ProgramComposition, "apxm.program-composition.v1"),
        composition,
    )
    .expect("driver ports satisfy their exact admitted bindings");
    ExecutionPorts::from_admitted_bundle(
        &bundle,
        Arc::new(TestModelRequestMetadata),
        Arc::new(StaticHooks),
    )
    .expect("bundle contains every runtime effect port")
}

fn ports_with_capability(
    commit: Arc<FakeCommit>,
    capability: Arc<dyn CapabilityPort>,
) -> ExecutionPorts {
    ports_with_model_composition_and_capability(
        commit,
        Arc::new(FakeModel),
        Arc::new(FakeComposition),
        capability,
    )
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
        capability_invocations: BTreeMap::from([(
            "n.cap".to_string(),
            CapabilityInvocationAdmission {
                capability_ref: "cap.search".into(),
                arguments: json!({"query": "release checklist"}),
                authority: CapabilityInvocationAuthority::new(
                    "principal.user.1",
                    "agent.gao.1",
                    "grant.search.1",
                    ["approval.search.1".to_string()],
                )
                .expect("valid test authority"),
            },
        )]),
        program_instance_ref: ProgramInstanceRef::new("instance.1"),
        program_invocation_ref: ProgramInvocationRef::new("invocation.1"),
        commit_id: "c1".into(),
        write_set: write_set(),
    }
}

#[tokio::test]
async fn executes_all_five_ops_and_commits_atomically() {
    let commit = Arc::new(FakeCommit::new());

    let report = execute(&ports(commit.clone()), request(), json!({"iterations": 0}))
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
    assert_eq!(commit.invocation_refs(), vec!["invocation.1"]);

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
    assert_eq!(report.final_context, json!({"iterations": 1}));

    // Peer usage is isolated in External Agent evidence, never in native usage.
    assert_eq!(report.external_agent_evidence.len(), 1);
    assert_eq!(report.external_agent_evidence[0].session_ref, "session.1");
    assert_eq!(
        report.external_agent_evidence[0].peer_usage[0].reported_value,
        "555"
    );
    assert_eq!(
        report.native_usage.input_tokens, 10,
        "peer 555 never enters native usage"
    );

    // The committed evidence records one NodeExecution per node, the successful
    // native model attempt, the static handler,
    // its explicit Context transition, lifecycle facts, and one ChildAttached
    // lineage fact for each of program.new and program.invoke.
    assert_eq!(commit.facts().len(), 2 + 6 + 1 + 1 + 1 + 1 + 2);
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
async fn capability_port_receives_arguments_authority_identity_and_stable_effect_facts() {
    let capability = Arc::new(RecordingCapability::default());
    execute(
        &ports_with_capability(Arc::new(FakeCommit::new()), capability.clone()),
        request(),
        Value::Null,
    )
    .await
    .expect("canonical execution");

    let requests = capability.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.capability_ref(), "cap.search");
    assert_eq!(request.arguments().type_ref(), "CapabilityArguments");
    assert_eq!(
        request.arguments().value().expect("canonical arguments"),
        json!({"query": "release checklist"})
    );
    assert_eq!(
        request.correlation().program_invocation_ref.target,
        "invocation.1"
    );
    assert!(
        request
            .correlation()
            .node_execution_id
            .starts_with("node-execution.invocation.1.n.cap.")
    );
    assert_eq!(
        request.authority().acting_principal_ref().target,
        "principal.user.1"
    );
    assert_eq!(
        request.authority().agent_identity_ref().target,
        "agent.gao.1"
    );
    assert_eq!(
        request.authority().capability_grant_ref().target,
        "grant.search.1"
    );
    assert_eq!(
        request.authority().approval_refs()[0].target,
        "approval.search.1"
    );
    assert_eq!(request.effect().idempotency_key.scope_ref, "invocation.1");
    assert_eq!(
        request.effect().idempotency_key.key_id,
        request.effect().effect_id
    );
    assert_eq!(
        request.effect().idempotency_key.request_digest,
        request.effect().request_digest
    );
    assert!(request.effect().request_digest.starts_with("sha256:"));
}

#[tokio::test]
async fn capability_dispatch_fails_closed_without_exact_invocation_admission() {
    let mut missing = request();
    missing.capability_invocations.clear();
    let error = execute(&ports(Arc::new(FakeCommit::new())), missing, Value::Null)
        .await
        .expect_err("missing Capability admission");
    assert!(matches!(
        error,
        ExecutionError::MissingCapabilityInvocationAdmission { node_id }
            if node_id == "n.cap"
    ));

    let mut mismatched = request();
    mismatched
        .capability_invocations
        .get_mut("n.cap")
        .expect("test admission")
        .capability_ref = "cap.other".into();
    let error = execute(&ports(Arc::new(FakeCommit::new())), mismatched, Value::Null)
        .await
        .expect_err("mismatched Capability admission");
    assert!(matches!(
        error,
        ExecutionError::CapabilityInvocationAdmissionMismatch {
            node_id,
            authored,
            admitted,
        } if node_id == "n.cap" && authored == "cap.search" && admitted == "cap.other"
    ));
}

#[tokio::test]
async fn committed_native_model_usage_carries_runtime_coordinates_and_excludes_acp_usage() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::default());
    let ports = ports(commit.clone()).with_committed_native_model_usage_port(usage.clone());

    let report = execute(&ports, request(), json!({})).await.expect("run");

    assert_eq!(
        report.operational_usage,
        CommittedNativeModelUsageOutcome::Published
    );
    let calls = usage.calls();
    assert_eq!(calls.len(), 1);
    let usage_fact = &calls[0];
    let fact = &usage_fact.attempt;
    assert_eq!(usage_fact.commit_id, "c1");
    assert_eq!(usage_fact.evidence_position_ref.r#ref, "evidence:1");
    assert_eq!(fact.program_invocation_id, "invocation.1");
    assert_eq!(
        fact.node_execution_id,
        "node-execution.invocation.1.n.model.3"
    );
    assert_eq!(fact.air_node_id, "n.model");
    assert_eq!(
        fact.attempt_id,
        "model-attempt.node-execution.invocation.1.n.model.3.0"
    );
    assert_eq!(fact.attempt_index, 0);
    assert!(fact.model_effect_id.starts_with("model-effect."));
    assert!(fact.request_digest.starts_with("sha256:"));
    assert_eq!(fact.model_target_ref, "model.target.v1");
    assert_eq!(fact.model_deployment_ref, "deploy.default");
    assert_eq!(fact.exact_port_binding_digest, digest('a'));
    assert_eq!(fact.native_input_tokens, 10);
    assert_eq!(fact.native_output_tokens, 20);
    let schema_bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../contracts/schemas/apxm.committed-native-model-usage.v1.json"),
    )
    .expect("owning schema bytes");
    assert_eq!(
        usage_fact.source_contract_digest,
        format!("sha256:{:x}", Sha256::digest(schema_bytes))
    );
    assert_eq!(
        usage_fact.usage_measurement_id,
        CommittedNativeModelUsage::measurement_id(&usage_fact.commit_id, &fact.fact_id)
    );
    let committed_facts = commit.facts();
    let committed_attempt = committed_facts
        .iter()
        .find_map(Fact::model_attempt_recorded)
        .expect("attempt is committed atomically");
    assert_eq!(committed_attempt, fact);
    assert_eq!(
        report.external_agent_evidence[0].peer_usage[0].reported_value,
        "555"
    );
    assert_ne!(
        usage.calls()[0].attempt.native_input_tokens,
        555,
        "ACP peer usage never becomes native operational usage"
    );
}

#[tokio::test]
async fn committed_native_model_usage_decode_rejects_missing_and_unknown_fields() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::default());
    let ports = ports(commit).with_committed_native_model_usage_port(usage.clone());
    execute(&ports, request(), json!({}))
        .await
        .expect("usage-producing execution commits");

    let canonical = serde_json::to_value(&usage.calls()[0]).expect("closed usage serializes");
    let mut missing = canonical.clone();
    missing
        .as_object_mut()
        .expect("usage object")
        .remove("source_contract_digest");
    assert!(serde_json::from_value::<CommittedNativeModelUsage>(missing).is_err());

    let mut unknown = canonical.clone();
    unknown
        .as_object_mut()
        .expect("usage object")
        .insert("company_ref".into(), json!("company.forbidden"));
    assert!(serde_json::from_value::<CommittedNativeModelUsage>(unknown).is_err());

    let mut nested_unknown = canonical;
    nested_unknown["attempt"]["price_microunits"] = json!(1);
    assert!(serde_json::from_value::<CommittedNativeModelUsage>(nested_unknown).is_err());
}

#[tokio::test]
async fn each_native_model_call_is_published_as_a_distinct_measurement() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::default());
    let ports = ports(commit).with_committed_native_model_usage_port(usage.clone());
    let two_models: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "n.model.first", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.first.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.first.output", "type_ref": "ModelOutput"}},
            {"node_id": "n.model.second", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 1, "operands": [{"slot": "model_ref", "value_id": "model.target.v2", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.second.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.second.output", "type_ref": "ModelOutput"}}
        ],
        "structural_ir": [{"region_id": "r.fn", "kind": "function", "execution_order": 0}],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("valid multi-model AIR");
    assert!(two_models.verify().is_accepted());
    let mut req = request();
    req.air = two_models;
    req.hook_bindings = Vec::new();
    req.model_admission = ModelBindingAdmission::for_invocation(vec![
        admission()
            .validate(&ModelTargetRef("model.target.v1".into()))
            .expect("first model binding"),
        ResolvedModelBinding {
            model_target: ExactModelTargetRef {
                reference: ModelTargetRef("model.target.v2".into()),
                target_digest: digest('8'),
            },
            model_deployment_ref: ModelDeploymentRef("deploy.second".into()),
            exact_port_binding: ExactPortBindingRef {
                binding_digest: digest('f'),
                port_contract_digest: digest('b'),
            },
            composition_digest: digest('c'),
        },
    ]);

    let report = execute(&ports, req, json!({})).await.expect("run");

    assert_eq!(
        report.operational_usage,
        CommittedNativeModelUsageOutcome::Published
    );
    let calls = usage.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].attempt.air_node_id, "n.model.first");
    assert_eq!(calls[1].attempt.air_node_id, "n.model.second");
    assert_ne!(
        calls[0].attempt.node_execution_id,
        calls[1].attempt.node_execution_id
    );
    assert_ne!(calls[0].attempt.attempt_id, calls[1].attempt.attempt_id);
    assert_eq!(calls[0].attempt.attempt_index, 0);
    assert_eq!(calls[1].attempt.attempt_index, 0);
    assert_eq!(calls[0].attempt.model_target_ref, "model.target.v1");
    assert_eq!(calls[1].attempt.model_target_ref, "model.target.v2");
    assert_ne!(
        calls[0].attempt.model_effect_id,
        calls[1].attempt.model_effect_id
    );
    assert_ne!(
        calls[0].attempt.request_digest,
        calls[1].attempt.request_digest
    );
    assert_eq!(
        report.native_usage,
        Usage {
            input_tokens: 20,
            output_tokens: 40
        }
    );
}

#[tokio::test]
async fn retrying_model_usage_keeps_the_successful_attempt_coordinate() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::default());
    let retrying_model = Arc::new(RetryingModel::default());
    let ports = ports_with_model(commit, retrying_model.clone())
        .with_committed_native_model_usage_port(usage.clone());

    let report = execute(&ports, request(), json!({})).await.expect("run");

    assert_eq!(
        report.operational_usage,
        CommittedNativeModelUsageOutcome::Published
    );
    let calls = usage.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].attempt.attempt_index, 1);
    assert!(calls[0].attempt.attempt_id.ends_with(".1"));
    assert_eq!(calls[0].attempt.native_input_tokens, 7);
    assert_eq!(calls[0].attempt.native_output_tokens, 11);
    let request_identities = retrying_model.request_identities();
    assert_eq!(request_identities.len(), 2);
    assert_eq!(request_identities[0], request_identities[1]);
}

#[tokio::test]
async fn usage_presentation_failure_is_reported_after_commit_not_as_success_or_rollback() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::failing(
        CommittedNativeModelUsageError::Unavailable,
    ));
    let ports = ports(commit).with_committed_native_model_usage_port(usage.clone());

    let report = execute(&ports, request(), json!({}))
        .await
        .expect("execution commits");

    assert!(matches!(
        report.commit,
        ExecutionCommitResult::Committed { .. }
    ));
    assert_eq!(
        report.operational_usage,
        CommittedNativeModelUsageOutcome::Failed(CommittedNativeModelUsageError::Unavailable)
    );
    assert_eq!(usage.calls().len(), 1);
}

#[tokio::test]
async fn zero_native_usage_and_uncommitted_execution_emit_nothing() {
    let usage = Arc::new(RecordingOperationalUsage::default());
    let no_model_air: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [{
            "node_id": "n.cap", "op": "capability.invoke", "parent_region_id": "r.fn", "execution_order": 0,
            "operands": [
                {"slot": "capability_ref", "value_id": "cap.search", "type_ref": "CapabilityRef"},
                {"slot": "arguments", "value_id": "value.cap.arguments", "type_ref": "CapabilityArguments"}
            ],
            "result": {"value_id": "value.cap.output", "type_ref": "CapabilityOutput"}
        }],
        "structural_ir": [{"region_id": "r.fn", "kind": "function", "execution_order": 0}],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("valid no-model AIR");
    assert!(no_model_air.verify().is_accepted());
    let mut no_model_request = request();
    no_model_request.air = no_model_air;
    no_model_request.hook_bindings = Vec::new();
    let zero_ports =
        ports(Arc::new(FakeCommit::new())).with_committed_native_model_usage_port(usage.clone());

    let zero_report = execute(&zero_ports, no_model_request, json!({}))
        .await
        .expect("zero-usage execution commits");
    assert_eq!(
        zero_report.operational_usage,
        CommittedNativeModelUsageOutcome::NotApplicable
    );
    assert!(usage.calls().is_empty());

    let zero_model_ports = ports_with_model(Arc::new(FakeCommit::new()), Arc::new(ZeroUsageModel))
        .with_committed_native_model_usage_port(usage.clone());
    let zero_model_report = execute(&zero_model_ports, request(), json!({}))
        .await
        .expect("zero model usage commits");
    assert_eq!(
        zero_model_report.operational_usage,
        CommittedNativeModelUsageOutcome::Published
    );
    assert_eq!(usage.calls().len(), 1);
    assert_eq!(usage.calls()[0].attempt.native_input_tokens, 0);
    assert_eq!(usage.calls()[0].attempt.native_output_tokens, 0);

    let uncommitted_ports = ports(Arc::new(FakeCommit::failing()))
        .with_committed_native_model_usage_port(usage.clone());
    let uncommitted_report = execute(&uncommitted_ports, request(), json!({}))
        .await
        .expect("driver reports failed commit");
    assert!(matches!(
        uncommitted_report.commit,
        ExecutionCommitResult::CompareConflict { .. }
    ));
    assert_eq!(
        uncommitted_report.operational_usage,
        CommittedNativeModelUsageOutcome::NotApplicable
    );
    assert_eq!(usage.calls().len(), 1);
}

#[tokio::test]
async fn repeated_instance_invocations_keep_evidence_identities_disjoint() {
    let commit = Arc::new(FakeCommit::new());
    execute(&ports(commit.clone()), request(), json!({"invocation": 1}))
        .await
        .expect("first invocation");

    let mut second = request();
    second.commit_id = "c2".into();
    second.program_invocation_ref = ProgramInvocationRef::new("invocation.2");
    execute(&ports(commit.clone()), second, json!({"invocation": 2}))
        .await
        .expect("second invocation");

    let facts = commit.facts();
    let fact_ids: std::collections::HashSet<_> = facts.iter().map(Fact::fact_id).collect();
    assert_eq!(fact_ids.len(), facts.len());
    assert!(
        fact_ids
            .iter()
            .any(|fact_id| fact_id.contains(".invocation.1."))
    );
    assert!(
        fact_ids
            .iter()
            .any(|fact_id| fact_id.contains(".invocation.2."))
    );
}

#[tokio::test]
async fn distinct_invocations_do_not_reuse_published_invocation_coordinates() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::default());
    let ports = ports(commit).with_committed_native_model_usage_port(usage.clone());

    execute(&ports, request(), json!({}))
        .await
        .expect("first invocation commits");
    let mut second = request();
    second.commit_id = "c2".into();
    second.program_invocation_ref = ProgramInvocationRef::new("invocation.2");
    execute(&ports, second, json!({}))
        .await
        .expect("second invocation commits");

    let calls = usage.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].attempt.program_invocation_id, "invocation.1");
    assert_eq!(calls[1].attempt.program_invocation_id, "invocation.2");
    assert_ne!(
        calls[0].attempt.node_execution_id,
        calls[1].attempt.node_execution_id
    );
    assert_ne!(
        calls[0].attempt.model_effect_id,
        calls[1].attempt.model_effect_id
    );
    assert_ne!(
        calls[0].attempt.request_digest,
        calls[1].attempt.request_digest
    );
    assert_ne!(calls[0].usage_measurement_id, calls[1].usage_measurement_id);
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
            {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "program_ref", "value_id": "Specialist", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
            {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.fn", "execution_order": 1, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .unwrap();
    assert!(air.verify().is_accepted());
    let ports = ports_with_composition(
        commit.clone(),
        Arc::new(RecordingComposition {
            receivers: receivers.clone(),
        }),
    );
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
            program_instance_ref: "value.program.instance".into()
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
    assert!(!air.verify().is_accepted());
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
            {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.unbound", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .unwrap();
    assert!(bad_air.verify().is_accepted());
    let request = ExecutionRequest {
        air: bad_air,
        hook_bindings: Vec::new(),
        model_admission: admission(),
        capability_invocations: BTreeMap::new(),
        program_instance_ref: ProgramInstanceRef::new("instance.1"),
        program_invocation_ref: ProgramInvocationRef::new("invocation.unbound"),
        commit_id: "c1".into(),
        write_set: write_set(),
    };
    let err = execute(&ports(commit), request, json!(null))
        .await
        .expect_err("no binding");
    assert!(matches!(err, apxm_execution::ExecutionError::Binding(_)));
}
