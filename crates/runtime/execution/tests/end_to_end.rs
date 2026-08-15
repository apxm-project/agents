//! End-to-end canonical execution: drive a five-operation AIR program through
//! the injected kernel ports and commit atomically. Deterministic in-crate fakes
//! stand in for the admitted ports (test doubles, non-admissible).

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use apxm_inference::{
    AttemptDisposition, IdempotencyKey, InferenceTargetCommitment, InferenceUsageLineage,
    ModelBindingAdmission, ModelCallPreparation, ModelCallRequest, ModelCallRequestMetadata,
    ModelCallRequestMetadataPort, ModelContextEnvelopeRef, ModelInferencePort, ModelOutcome,
    ModelStreamMode, ModelTargetRef, ResolvedModelBinding, TypedError, Usage,
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
use apxm_program::runtime_evidence::{
    Fact, FactKind, ModelAttemptRecordedFact, PermissionDecision, PermissionLayer,
    ResolvedPermission,
};

use apxm_execution::{
    CapabilityGrantSet, CapabilityInvocationAdmission, CapabilityOutcome, CapabilityPort,
    CapabilityRequest, CommittedNativeModelUsage, CommittedNativeModelUsageError,
    CommittedNativeModelUsageGateError, CommittedNativeModelUsageOutcome,
    CommittedNativeModelUsagePort, CompositionOutcome, CompositionPort, CompositionReceiver,
    CompositionRequest, EventAwait, EventOutcome, EventPort, EvidencePositionRef,
    EvidencePositionRefType, ExecutionError, ExecutionPortBundle, ExecutionPorts, ExecutionRequest,
    NodeOutcome, StaticHookHandlerPort, StaticHookResult, execute,
};

/// The decision every admission in these fixtures carries unless a test
/// replaces it. There is no "no decision" admission to write.
fn code_allow() -> ResolvedPermission {
    ResolvedPermission {
        decision: PermissionDecision::allow(),
        layer: PermissionLayer::Code,
    }
}

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

/// The Capability grants this file's AIR corpora run under.
///
/// These programs exist to pin *lowering and dispatch shape*, not effect
/// behaviour: their Capability references are opaque on purpose, name no
/// catalogue id, and are answered by in-crate fakes that never look anything
/// up. `for_conformance_corpus` is how that is stated. A composition root that
/// dispatches for real builds its grant set with
/// `CapabilityGrantSet::from_registered_implementations` instead, and an
/// authored reference outside it fails at admission. The two are told apart by
/// which constructor was called — by name, here — never by inspecting the
/// reference and guessing.
fn corpus_grants() -> CapabilityGrantSet {
    CapabilityGrantSet::for_conformance_corpus([
        "cap.search",
        "cap.other",
        "external-agent:acp:claude-code",
    ])
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
        "schema_version": "apxm.air",
        "value_assemblies": [
            {"value_id": "value.model.request", "expression": {"kind": "object", "fields": [{"name": "prompt", "value": {"kind": "string", "value": "test"}}]}},
            {"value_id": "value.cap.arguments", "expression": {"kind": "object", "fields": [{"name": "query", "value": {"kind": "string", "value": "release checklist"}}]}},
            {"value_id": "session.1", "expression": {"kind": "string", "value": "session.1"}},
            {"value_id": "value.hook.context", "expression": {"kind": "object", "fields": [{"name": "iterations", "value": {"kind": "integer", "value": 1}}]}}
        ],
        "semantic_operations": [
            {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}},
            {"node_id": "n.cap", "op": "capability.invoke", "parent_region_id": "r.fn", "execution_order": 2, "operands": [{"slot": "capability_ref", "value_id": "cap.search", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.cap.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.cap.output", "type_ref": "CapabilityOutput"}},
            {"node_id": "n.acp", "op": "capability.invoke", "parent_region_id": "r.fn", "execution_order": 3, "operands": [{"slot": "capability_ref", "value_id": "external-agent:acp:claude-code", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "session.1", "type_ref": "ExternalAgentSessionRef"}], "result": {"value_id": "value.acp.output", "type_ref": "ExternalAgentOutput"}},
            {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.fn", "execution_order": 4, "operands": [{"slot": "program_ref", "value_id": "Specialist", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
            {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.fn", "execution_order": 5, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}},
            {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.fn", "execution_order": 6, "operands": [{"slot": "event_ref", "value_id": "evt.done", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0},
            {"region_id": "hook.after.model.body", "kind": "region", "parent_region_id": "r.fn", "execution_order": 1},
            {"region_id": "r.return", "kind": "return", "parent_region_id": "r.fn", "execution_order": 7}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("valid AIR");
    assert!(air.verify().is_accepted());
    air
}

fn admission() -> ModelBindingAdmission {
    admission_at_generation(0)
}

fn admission_at_generation(generation: u64) -> ModelBindingAdmission {
    ModelBindingAdmission::new(ResolvedModelBinding::from_target_commitment(
        InferenceTargetCommitment::commit(
            "model.target",
            digest('9'),
            "deploy.default",
            digest('a'),
            digest('b'),
            digest('c'),
            generation,
        )
        .expect("target commitment"),
    ))
}

fn committed_attempt(
    fact_id: &str,
    attempt_index: u32,
    input_tokens: u64,
    output_tokens: u64,
) -> ModelAttemptRecordedFact {
    let target_commitment = InferenceTargetCommitment::commit(
        "model.target",
        digest('9'),
        "deploy.default",
        digest('a'),
        digest('b'),
        digest('c'),
        0,
    )
    .expect("target commitment");
    ModelAttemptRecordedFact {
        fact_id: fact_id.into(),
        event_sequence: 3,
        program_invocation_id: "invocation.1".into(),
        node_execution_id: "node-execution.invocation.1.n.model.3".into(),
        air_node_id: "n.model".into(),
        attempt_id: format!("model-attempt.node-execution.invocation.1.n.model.3.{attempt_index}"),
        attempt_index,
        model_effect_id: "model-effect.test".into(),
        request_digest: digest('e'),
        model_target_ref: "model.target".into(),
        model_target_digest: digest('9'),
        model_deployment_ref: "deploy.default".into(),
        exact_port_binding_digest: digest('a'),
        target_commitment_digest: target_commitment.commit_digest,
        generation_cohort_digest: target_commitment.generation_cohort_digest,
        target_generation: target_commitment.target_generation,
        target_port_contract_digest: target_commitment.port_contract_digest,
        target_composition_digest: target_commitment.composition_digest,
        native_input_tokens: input_tokens,
        native_output_tokens: output_tokens,
    }
}

fn lineage_backed_usage() -> CommittedNativeModelUsage {
    let attempt = committed_attempt("fact.invocation.1.3", 0, 10, 20);
    let target_commitment = InferenceTargetCommitment::commit(
        attempt.model_target_ref.clone(),
        digest('9'),
        attempt.model_deployment_ref.clone(),
        attempt.exact_port_binding_digest.clone(),
        digest('b'),
        digest('c'),
        0,
    )
    .expect("target commitment");
    let mut lineage = InferenceUsageLineage::seal_with_target_commitment(
        attempt.model_effect_id.clone(),
        attempt.attempt_index,
        attempt.request_digest.clone(),
        &target_commitment,
        Usage {
            input_tokens: attempt.native_input_tokens,
            output_tokens: attempt.native_output_tokens,
        },
        77,
        None,
    )
    .expect("seal lineage");
    lineage
        .bind_evidence(attempt.fact_id.clone(), "c1")
        .expect("bind lineage");
    CommittedNativeModelUsage::from_lineage(
        "c1",
        EvidencePositionRef {
            ref_type: EvidencePositionRefType::EvidencePositionRef,
            r#ref: "evidence:1".into(),
        },
        attempt,
        &lineage,
    )
    .expect("lineage-backed usage")
}

fn lineage_for_attempt(
    attempt: &ModelAttemptRecordedFact,
    request_digest: String,
    deployment_ref: String,
) -> InferenceUsageLineage {
    let target_commitment = InferenceTargetCommitment::commit(
        attempt.model_target_ref.clone(),
        digest('9'),
        deployment_ref,
        attempt.exact_port_binding_digest.clone(),
        digest('b'),
        digest('c'),
        0,
    )
    .expect("target commitment");
    InferenceUsageLineage::seal_with_target_commitment(
        attempt.model_effect_id.clone(),
        attempt.attempt_index,
        request_digest,
        &target_commitment,
        Usage {
            input_tokens: attempt.native_input_tokens,
            output_tokens: attempt.native_output_tokens,
        },
        12,
        None,
    )
    .expect("seal lineage")
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
        AttemptDisposition::Success {
            usage: Usage {
                input_tokens: 10,
                output_tokens: 20,
            },
            output: serde_json::Value::Null,
        }
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
            AttemptDisposition::Success {
                usage: Usage {
                    input_tokens: 7,
                    output_tokens: 11,
                },
                output: serde_json::Value::Null,
            }
        }
    }
}

struct ZeroUsageModel;
impl ModelInferencePort for ZeroUsageModel {
    fn attempt(&self, _request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        AttemptDisposition::Success {
            usage: Usage::default(),
            output: serde_json::Value::Null,
        }
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

struct RecordingCapability {
    requests: Mutex<Vec<CapabilityRequest>>,
    results: Mutex<VecDeque<String>>,
}

impl RecordingCapability {
    fn with_results(results: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            results: Mutex::new(results.into_iter().map(Into::into).collect()),
        }
    }

    fn requests(&self) -> Vec<CapabilityRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Default for RecordingCapability {
    fn default() -> Self {
        Self::with_results(["ok"])
    }
}

#[async_trait]
impl CapabilityPort for RecordingCapability {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome {
        self.requests.lock().unwrap().push(request);
        CapabilityOutcome::Completed {
            result: self
                .results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| "ok".into()),
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

struct NeverCalledAcp;

#[async_trait]
impl ExternalAgentCapabilityPort for NeverCalledAcp {
    async fn prompt(&self, _request: AcpPromptRequest) -> AcpPromptOutcome {
        panic!("rejected external-agent admission must not dispatch an effect")
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
        invocation: apxm_execution::StaticHookInvocation<'_>,
    ) -> Result<StaticHookResult, apxm_execution::StaticHookExecutionError> {
        assert_eq!(invocation.binding.handler_ref, "hooks.after_model");
        Ok(StaticHookResult::Replace {
            assigned_context: Some(json!({"iterations": 1})),
            result: json!("hooked"),
        })
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
    ports_with_model_composition_capability_and_external(
        commit,
        model,
        composition,
        capability,
        Arc::new(FakeAcpPeer),
    )
}

fn ports_with_model_composition_capability_and_external(
    commit: Arc<FakeCommit>,
    model: Arc<dyn ModelInferencePort + Send + Sync>,
    composition: Arc<dyn CompositionPort>,
    capability: Arc<dyn CapabilityPort>,
    external_agent: Arc<dyn ExternalAgentCapabilityPort>,
) -> ExecutionPorts {
    ports_with_model_composition_capability_external_and_hooks(
        commit,
        model,
        composition,
        capability,
        external_agent,
        Arc::new(StaticHooks),
    )
}

fn ports_with_model_composition_capability_external_and_hooks(
    commit: Arc<FakeCommit>,
    model: Arc<dyn ModelInferencePort + Send + Sync>,
    composition: Arc<dyn CompositionPort>,
    capability: Arc<dyn CapabilityPort>,
    external_agent: Arc<dyn ExternalAgentCapabilityPort>,
    hooks: Arc<dyn StaticHookHandlerPort>,
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
        (PortSlot::ExecutionCommit, contract("apxm.execution-commit")),
        (PortSlot::ModelInference, contract("apxm.model-inference")),
        (PortSlot::Capability, contract("apxm.capability-invocation")),
        (
            PortSlot::ExternalAgentCapability,
            contract("apxm.external-agent"),
        ),
    ]);
    let kernel_bundle = PortBundle::construct(
        &spec,
        vec![
            (
                binding(PortSlot::ExecutionCommit, "apxm.execution-commit"),
                PortImplementation::ExecutionCommit(commit),
            ),
            (
                binding(PortSlot::ModelInference, "apxm.model-inference"),
                PortImplementation::ModelInference(model),
            ),
            (
                binding(PortSlot::Capability, "apxm.capability-invocation"),
                PortImplementation::Capability(capability),
            ),
            (
                binding(PortSlot::ExternalAgentCapability, "apxm.external-agent"),
                PortImplementation::ExternalAgentCapability(external_agent),
            ),
        ],
    )
    .expect("test ports satisfy the admitted bundle");
    let bundle = ExecutionPortBundle::construct(
        Arc::new(kernel_bundle),
        contract("apxm.durable-event"),
        binding(PortSlot::DurableEvent, "apxm.durable-event"),
        Arc::new(FakeEvents),
        contract("apxm.program-composition"),
        binding(PortSlot::ProgramComposition, "apxm.program-composition"),
        composition,
    )
    .expect("driver ports satisfy their exact admitted bindings");
    ExecutionPorts::from_admitted_bundle(&bundle, Arc::new(TestModelRequestMetadata), hooks)
        .expect("bundle contains every runtime effect port")
}

fn typed_tool_loop_air() -> AirModule {
    let air: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "value_assemblies": [
            {"value_id": "value.request.initial", "expression": {"kind": "object", "fields": [{"name": "messages", "value": {"kind": "array", "items": []}}]}},
            {"value_id": "value.search.arguments", "expression": {"kind": "projection", "root": {"kind": "ssa", "value_id": "value.response.current"}, "property_path": ["tool_request", "arguments"]}},
            {"value_id": "value.request.next", "expression": {"kind": "object", "fields": [{"name": "tool_result", "value": {"kind": "ssa", "value_id": "value.search.result"}}]}}
        ],
        "semantic_operations": [
            {"node_id": "n.model.initial", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.initial", "type_ref": "ModelRequest"}], "result": {"value_id": "value.response.initial", "type_ref": "ModelResponse"}},
            {"node_id": "n.search", "op": "capability.invoke", "parent_region_id": "r.branch.then", "execution_order": 1, "operands": [{"slot": "capability_ref", "value_id": "cap.search", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.search.arguments", "type_ref": "SearchRequest"}], "result": {"value_id": "value.search.result", "type_ref": "SearchResult"}},
            {"node_id": "n.model.next", "op": "model.call", "parent_region_id": "r.branch.then", "execution_order": 3, "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.next", "type_ref": "ModelRequest"}], "result": {"value_id": "value.response.next", "type_ref": "ModelResponse"}}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0},
            {"region_id": "r.loop", "kind": "ais.loop", "parent_region_id": "r.fn", "execution_order": 1, "block_arguments": [{"value_id": "value.response.current", "type_ref": "ModelResponse"}], "operands": [{"slot": "initial", "value_id": "value.response.initial", "type_ref": "ModelResponse"}, {"slot": "carried", "value_id": "value.response.next", "type_ref": "ModelResponse"}], "predicate": {"root_value_id": "value.response.current", "property_path": ["kind"], "comparator": "equals", "literal": {"scalar_type": "string", "value": "tool_request"}}},
            {"region_id": "r.branch", "kind": "branch", "parent_region_id": "r.loop", "execution_order": 0, "predicate": {"root_value_id": "value.response.current", "property_path": ["tool_request", "kind"], "comparator": "equals", "literal": {"scalar_type": "string", "value": "search_web"}}},
            {"region_id": "r.branch.then", "kind": "region", "parent_region_id": "r.branch", "execution_order": 0},
            {"region_id": "hook.before.body", "kind": "region", "parent_region_id": "r.branch.then", "execution_order": 0},
            {"region_id": "hook.after.body", "kind": "region", "parent_region_id": "r.branch.then", "execution_order": 2},
            {"region_id": "r.branch.else", "kind": "region", "parent_region_id": "r.branch", "execution_order": 1},
            {"region_id": "r.unknown.throw", "kind": "throw", "parent_region_id": "r.branch.else", "execution_order": 0},
            {"region_id": "r.response.branch", "kind": "branch", "parent_region_id": "r.fn", "execution_order": 2, "predicate": {"root_value_id": "value.response.current", "property_path": ["kind"], "comparator": "not_equals", "literal": {"scalar_type": "string", "value": "final"}}},
            {"region_id": "r.response.unknown", "kind": "region", "parent_region_id": "r.response.branch", "execution_order": 0},
            {"region_id": "r.response.throw", "kind": "throw", "parent_region_id": "r.response.unknown", "execution_order": 0},
            {"region_id": "r.return", "kind": "return", "parent_region_id": "r.fn", "execution_order": 3}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": [{"region_id": "r.loop", "annotation": "structural_loop"}]}
    }))
    .expect("typed conditional Tool-loop AIR");
    assert!(air.verify().is_accepted());
    air
}

struct TypedResponses {
    outputs: Mutex<VecDeque<Value>>,
    request_digests: Mutex<Vec<String>>,
    context_digests: Mutex<Vec<String>>,
    authored_requests: Mutex<Vec<Value>>,
}

impl TypedResponses {
    fn new(outputs: impl IntoIterator<Item = Value>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into_iter().collect()),
            request_digests: Mutex::new(Vec::new()),
            context_digests: Mutex::new(Vec::new()),
            authored_requests: Mutex::new(Vec::new()),
        }
    }
}

impl ModelInferencePort for TypedResponses {
    fn attempt(&self, request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        self.request_digests
            .lock()
            .unwrap()
            .push(request.request_digest().to_string());
        self.context_digests
            .lock()
            .unwrap()
            .push(request.context_digest().to_string());
        self.authored_requests
            .lock()
            .unwrap()
            .push(request.authored_request().clone());
        AttemptDisposition::Success {
            usage: Usage::default(),
            output: self
                .outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("test supplies every typed Model response"),
        }
    }
}

#[derive(Default)]
struct OrderedToolHooks {
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl StaticHookHandlerPort for OrderedToolHooks {
    async fn execute(
        &self,
        invocation: apxm_execution::StaticHookInvocation<'_>,
    ) -> Result<StaticHookResult, apxm_execution::StaticHookExecutionError> {
        self.calls
            .lock()
            .unwrap()
            .push(invocation.binding.hook_id.clone());
        Ok(StaticHookResult::Keep {
            assigned_context: None,
        })
    }
}

fn typed_tool_request(air: AirModule, hook_bindings: Vec<HookBinding>) -> ExecutionRequest {
    ExecutionRequest {
        air,
        initial_values: BTreeMap::new(),
        hook_bindings,
        model_admission: admission(),
        capability_invocations: BTreeMap::from([(
            "n.search".to_string(),
            corpus_grants()
                .admit(
                    "cap.search",
                    CapabilityInvocationAuthority::new(
                        "principal.user.1",
                        "agent.gao.1",
                        "grant.search.1",
                        Vec::new(),
                    )
                    .expect("valid test authority"),
                    code_allow(),
                )
                .expect("corpus grant"),
        )]),
        program_instance_ref: ProgramInstanceRef::new("instance.typed-loop"),
        program_invocation_ref: ProgramInvocationRef::new("invocation.typed-loop"),
        commit_id: "typed-loop".into(),
        write_set: write_set(),
    }
}

fn tool_hooks() -> Vec<HookBinding> {
    [HookPhase::Before, HookPhase::After]
        .into_iter()
        .enumerate()
        .map(|(order, phase)| HookBinding {
            hook_id: format!(
                "hook.{}",
                if phase == HookPhase::Before {
                    "before"
                } else {
                    "after"
                }
            ),
            scope: HookScope::Capability,
            phase,
            target_selector: "n.search".into(),
            declaration_order: order as u32,
            handler_ref: format!("handlers.{order}"),
            handler_digest: digest(if order == 0 { 'a' } else { 'b' }),
            input_type_ref: "SearchResult".into(),
            output_type_ref: "Unit".into(),
            return_mode: HookReturnMode::Observe,
            body_region_id: format!(
                "hook.{}.body",
                if phase == HookPhase::Before {
                    "before"
                } else {
                    "after"
                }
            ),
            assigned_context_value_id: None,
        })
        .collect()
}

async fn run_typed_tool_loop(
    outputs: impl IntoIterator<Item = Value>,
) -> Result<
    (
        Arc<TypedResponses>,
        Arc<RecordingCapability>,
        Arc<OrderedToolHooks>,
    ),
    ExecutionError,
> {
    let model = Arc::new(TypedResponses::new(outputs));
    let capability = Arc::new(RecordingCapability::with_results(["first", "second"]));
    let hooks = Arc::new(OrderedToolHooks::default());
    execute(
        &ports_with_model_composition_capability_external_and_hooks(
            Arc::new(FakeCommit::new()),
            model.clone(),
            Arc::new(FakeComposition),
            capability.clone(),
            Arc::new(FakeAcpPeer),
            hooks.clone(),
        ),
        typed_tool_request(typed_tool_loop_air(), tool_hooks()),
        json!({"messages": []}),
    )
    .await?;
    Ok((model, capability, hooks))
}

#[tokio::test]
async fn typed_final_response_skips_the_declared_tool() {
    let (_, capability, hooks) = run_typed_tool_loop([json!({"kind": "final", "content": "done"})])
        .await
        .expect("final response exits the loop");
    assert!(capability.requests().is_empty());
    assert!(hooks.calls.lock().unwrap().is_empty());
}

/// The canonical handler installs exactly what a Hook's captured body assigned,
/// so a replacing Hook that assigned nothing has no replacement to install and
/// stops the invocation instead of silently observing.
#[tokio::test]
async fn a_replacing_hook_whose_body_assigned_nothing_fails_closed() {
    let mut request = request();
    request.hook_bindings[0].assigned_context_value_id = None;
    let error = execute(
        &ports_with_model_composition_capability_external_and_hooks(
            Arc::new(FakeCommit::new()),
            Arc::new(FakeModel),
            Arc::new(FakeComposition),
            Arc::new(FakeCapability),
            Arc::new(FakeAcpPeer),
            Arc::new(apxm_execution::CapturedHookBodyHandler),
        ),
        request,
        json!({"iterations": 0}),
    )
    .await
    .expect_err("a replacing Hook names the Context value its body assigned");
    assert!(matches!(error, ExecutionError::StaticHook(_)));
}

/// The one authority bit a binding carries about itself. An injected port that
/// hands back a replacement for a Hook the compiler derived as observing is
/// refused, rather than being applied because the handler said so.
#[tokio::test]
async fn an_observing_hook_whose_handler_replaces_fails_closed() {
    struct ReplacingHandler;

    #[async_trait::async_trait]
    impl StaticHookHandlerPort for ReplacingHandler {
        async fn execute(
            &self,
            _invocation: apxm_execution::StaticHookInvocation<'_>,
        ) -> Result<StaticHookResult, apxm_execution::StaticHookExecutionError> {
            Ok(StaticHookResult::Replace {
                assigned_context: Some(json!({"iterations": 99})),
                result: json!("smuggled"),
            })
        }
    }

    let mut request = request();
    request.hook_bindings[0].return_mode = HookReturnMode::Observe;
    request.hook_bindings[0].assigned_context_value_id = None;
    let error = execute(
        &ports_with_model_composition_capability_external_and_hooks(
            Arc::new(FakeCommit::new()),
            Arc::new(FakeModel),
            Arc::new(FakeComposition),
            Arc::new(FakeCapability),
            Arc::new(FakeAcpPeer),
            Arc::new(ReplacingHandler),
        ),
        request,
        json!({"iterations": 0}),
    )
    .await
    .expect_err("an observing Hook mutates nothing");
    let ExecutionError::StaticHook(error) = error else {
        panic!("an observing Hook's replacement is a Hook execution failure");
    };
    assert_eq!(error.hook_id, "hook.after.model");
}

/// A replacement has to reach the outcome the evidence records. It used to
/// patch a `model.call` outcome only, so a Capability Hook could hand the
/// program one answer while the committed Capability outcome still reported the
/// original — two different answers to the same question.
#[tokio::test]
async fn a_replacing_capability_hook_patches_the_outcome_the_run_reports() {
    struct ReplacingCapabilityHook;

    #[async_trait::async_trait]
    impl StaticHookHandlerPort for ReplacingCapabilityHook {
        async fn execute(
            &self,
            _invocation: apxm_execution::StaticHookInvocation<'_>,
        ) -> Result<StaticHookResult, apxm_execution::StaticHookExecutionError> {
            Ok(StaticHookResult::Replace {
                assigned_context: None,
                result: json!("redacted"),
            })
        }
    }

    let mut bindings = tool_hooks();
    bindings.retain(|binding| binding.phase == HookPhase::After);
    bindings[0].return_mode = HookReturnMode::ReplaceResult;

    let report = execute(
        &ports_with_model_composition_capability_external_and_hooks(
            Arc::new(FakeCommit::new()),
            Arc::new(TypedResponses::new([
                json!({"kind": "tool_request", "tool_request": {"kind": "search_web", "arguments": {"query": "first query"}}}),
                json!({"kind": "final", "content": "done"}),
            ])),
            Arc::new(FakeComposition),
            Arc::new(RecordingCapability::with_results(["first"])),
            Arc::new(FakeAcpPeer),
            Arc::new(ReplacingCapabilityHook),
        ),
        typed_tool_request(typed_tool_loop_air(), bindings),
        json!({"messages": []}),
    )
    .await
    .expect("a replacing Capability Hook completes the run");

    let capability = report
        .node_outcomes
        .iter()
        .find_map(|outcome| match outcome {
            NodeOutcome::Capability {
                node_id,
                outcome,
                replaced,
            } if node_id == "n.search" => Some((outcome, *replaced)),
            _ => None,
        })
        .expect("the Tool node reports an outcome");
    assert!(
        capability.1,
        "the Capability outcome records the replacement"
    );
    assert_eq!(
        *capability.0,
        apxm_program::capability::CapabilityOutcome::Completed {
            result: "redacted".into()
        }
    );
}

#[tokio::test]
async fn typed_tool_loop_reenters_and_binds_tool_results_without_hook_masking() {
    let (model, capability, hooks) = run_typed_tool_loop([
        json!({"kind": "tool_request", "tool_request": {"kind": "search_web", "arguments": {"query": "first query"}}}),
        json!({"kind": "tool_request", "tool_request": {"kind": "search_web", "arguments": {"query": "second query"}}}),
        json!({"kind": "final", "content": "done"}),
    ])
    .await
    .expect("declared Tool responses re-enter until final");
    assert_eq!(capability.requests().len(), 2);
    assert_eq!(
        capability.requests()[0].arguments().value().unwrap(),
        json!({"query": "first query"})
    );
    assert_eq!(
        capability.requests()[1].arguments().value().unwrap(),
        json!({"query": "second query"})
    );
    assert_eq!(
        *hooks.calls.lock().unwrap(),
        ["hook.before", "hook.after", "hook.before", "hook.after"]
    );
    let request_digests = model.request_digests.lock().unwrap();
    assert_eq!(request_digests.len(), 3);
    assert_ne!(request_digests[0], request_digests[1]);
    assert_ne!(request_digests[1], request_digests[2]);
    let context_digests = model.context_digests.lock().unwrap();
    let initial_context_digest = format!("sha256:{:x}", Sha256::digest(b"{\"messages\":[]}"));
    assert_eq!(context_digests[0], initial_context_digest);
    assert_eq!(context_digests[1], initial_context_digest);
    assert_eq!(context_digests[2], initial_context_digest);
    assert_eq!(
        *model.authored_requests.lock().unwrap(),
        [
            json!({"messages": []}),
            json!({"tool_result": "first"}),
            json!({"tool_result": "second"}),
        ]
    );
}

#[tokio::test]
async fn undeclared_typed_tool_request_fails_closed_before_dispatch() {
    let error = match run_typed_tool_loop([
        json!({"kind": "tool_request", "tool_request": {"kind": "undeclared"}}),
    ])
    .await
    {
        Ok(_) => panic!("unknown typed Tool discriminant must fail closed"),
        Err(error) => error,
    };
    assert!(matches!(error, ExecutionError::ProgramThrew { .. }));
}

#[tokio::test]
async fn undeclared_typed_model_response_kind_fails_closed() {
    let error = match run_typed_tool_loop([json!({"kind": "undeclared"})]).await {
        Ok(_) => panic!("unknown typed Model discriminant must fail closed"),
        Err(error) => error,
    };
    assert!(matches!(error, ExecutionError::ProgramThrew { .. }));
}

#[tokio::test]
async fn malformed_loop_phi_fails_before_runtime_can_truncate_it() {
    let mut air = typed_tool_loop_air();
    air.structural_ir
        .iter_mut()
        .find(|region| region.region_id == "r.loop")
        .unwrap()
        .operands
        .retain(|operand| operand.slot != "carried");
    assert!(!air.verify().is_accepted());
    let model = Arc::new(TypedResponses::new([json!({"kind": "final"})]));
    let capability = Arc::new(RecordingCapability::default());
    let error = execute(
        &ports_with_model_composition_capability_external_and_hooks(
            Arc::new(FakeCommit::new()),
            model,
            Arc::new(FakeComposition),
            capability.clone(),
            Arc::new(FakeAcpPeer),
            Arc::new(OrderedToolHooks::default()),
        ),
        typed_tool_request(air, tool_hooks()),
        json!({"messages": []}),
    )
    .await
    .expect_err("runtime rejects malformed phi even without prior AIR verification");
    assert!(matches!(error, ExecutionError::InvalidAir { .. }));
    assert!(capability.requests().is_empty());
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
        initial_values: BTreeMap::from([]),
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
            body_region_id: "hook.after.model.body".into(),
            assigned_context_value_id: Some("value.hook.context".into()),
        }],
        model_admission: admission(),
        capability_invocations: BTreeMap::from([
            (
                "n.cap".to_string(),
                corpus_grants()
                    .admit(
                        "cap.search",
                        CapabilityInvocationAuthority::new(
                            "principal.user.1",
                            "agent.gao.1",
                            "grant.search.1",
                            ["approval.search.1".to_string()],
                        )
                        .expect("valid test authority"),
                        code_allow(),
                    )
                    .expect("corpus grant"),
            ),
            (
                "n.acp".to_string(),
                corpus_grants()
                    .admit(
                        "external-agent:acp:claude-code",
                        CapabilityInvocationAuthority::new(
                            "principal.user.1",
                            "agent.gao.1",
                            "grant.acp.1",
                            ["approval.acp.1".to_string()],
                        )
                        .expect("valid external-agent authority"),
                        code_allow(),
                    )
                    .expect("corpus grant"),
            ),
        ]),
        program_instance_ref: ProgramInstanceRef::new("instance.1"),
        program_invocation_ref: ProgramInvocationRef::new("invocation.1"),
        commit_id: "c1".into(),
        write_set: write_set(),
    }
}

fn assembled_control_predicate_air() -> AirModule {
    serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "value_assemblies": [{
            "value_id": "value.control",
            "expression": {
                "kind": "object",
                "fields": [{
                    "name": "enabled",
                    "value": {"kind": "boolean", "value": true}
                }]
            }
        }],
        "semantic_operations": [],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0},
            {"region_id": "r.branch", "kind": "branch", "parent_region_id": "r.fn", "execution_order": 0,
             "predicate": {"root_value_id": "value.control", "property_path": ["enabled"], "comparator": "truthy"}},
            {"region_id": "r.then", "kind": "region", "parent_region_id": "r.branch", "execution_order": 0},
            {"region_id": "r.else", "kind": "region", "parent_region_id": "r.branch", "execution_order": 1}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("assembled control predicate AIR")
}

#[tokio::test]
async fn control_predicate_materializes_permitted_value_assembly_root() {
    let mut execution = request();
    execution.air = assembled_control_predicate_air();
    execution.hook_bindings.clear();
    execution.capability_invocations.clear();
    let report = execute(&ports(Arc::new(FakeCommit::new())), execution, Value::Null)
        .await
        .expect("assembled predicate root executes");
    assert!(report.node_outcomes.is_empty());
}

#[tokio::test]
async fn control_predicate_refuses_unmaterialized_future_value() {
    let mut execution = request();
    execution.air = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "value_assemblies": [{"value_id": "value.request", "expression": {"kind": "object", "fields": []}}],
        "semantic_operations": [{
            "node_id": "n.future", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 1,
            "operands": [
                {"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"},
                {"slot": "request", "value_id": "value.request", "type_ref": "ModelRequest"}
            ],
            "result": {"value_id": "value.future", "type_ref": "ModelResponse"}
        }],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0},
            {"region_id": "r.branch", "kind": "branch", "parent_region_id": "r.fn", "execution_order": 0,
             "predicate": {"root_value_id": "value.future", "property_path": [], "comparator": "truthy"}},
            {"region_id": "r.then", "kind": "region", "parent_region_id": "r.branch", "execution_order": 0},
            {"region_id": "r.else", "kind": "region", "parent_region_id": "r.branch", "execution_order": 1}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("future predicate AIR");
    execution.hook_bindings.clear();
    execution.capability_invocations.clear();
    let error = execute(&ports(Arc::new(FakeCommit::new())), execution, Value::Null)
        .await
        .expect_err("future predicate root must refuse before model execution");
    assert!(
        matches!(error, ExecutionError::MissingControlValue { value_id, .. } if value_id == "value.future")
    );
}

#[tokio::test]
async fn control_predicate_refuses_future_result_in_initial_values() {
    let mut execution = request();
    execution.air = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "value_assemblies": [],
        "semantic_operations": [{
            "node_id": "n.future", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 1,
            "operands": [
                {"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"},
                {"slot": "request", "value_id": "value.request", "type_ref": "ModelRequest"}
            ],
            "result": {"value_id": "value.future", "type_ref": "ModelResponse"}
        }],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0},
            {"region_id": "r.branch", "kind": "branch", "parent_region_id": "r.fn", "execution_order": 0,
             "predicate": {"root_value_id": "value.future", "property_path": [], "comparator": "truthy"}},
            {"region_id": "r.then", "kind": "region", "parent_region_id": "r.branch", "execution_order": 0},
            {"region_id": "r.else", "kind": "region", "parent_region_id": "r.branch", "execution_order": 1}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("future predicate injection AIR");
    execution
        .initial_values
        .insert("value.future".into(), json!(true));
    execution.hook_bindings.clear();
    execution.capability_invocations.clear();
    let error = execute(&ports(Arc::new(FakeCommit::new())), execution, Value::Null)
        .await
        .expect_err("initial values must not seed a future predicate result");
    assert!(
        matches!(error, ExecutionError::InvalidAir { message } if message.contains("value.future"))
    );
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
        NodeOutcome::Capability { .. }
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

    // Every capability invocation uses the generic capability contract; no
    // identity-specific evidence path is synthesized by the runtime.
    assert!(report.external_agent_evidence.is_empty());
    assert_eq!(
        report.native_usage.input_tokens, 10,
        "capability usage never enters native usage"
    );

    // The committed evidence records one NodeExecution per node, the successful
    // native model attempt, the static handler,
    // its explicit Context transition, lifecycle facts, one ChildAttached
    // lineage fact for each of program.new and program.invoke, and one
    // permission decision per capability invocation.
    assert_eq!(commit.facts().len(), 2 + 6 + 1 + 1 + 1 + 1 + 2 + 2);
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
    assert_eq!(requests.len(), 2);
    let request = requests
        .iter()
        .find(|request| request.capability_ref() == "cap.search")
        .expect("search capability request");
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

/// A resolved denial must stop the effect *and* be readable afterwards. Before
/// this, a refused capability left nothing in evidence naming what was refused
/// or who refused it, and the capability id was recoverable only by re-parsing
/// the AIR.
#[tokio::test]
async fn a_denied_capability_is_refused_before_the_port_and_recorded_in_evidence() {
    let capability = Arc::new(RecordingCapability::with_results(["never", "never"]));
    let commit = Arc::new(FakeCommit::new());
    let mut denied = request();
    denied
        .capability_invocations
        .get_mut("n.cap")
        .expect("test admission")
        .permission = ResolvedPermission {
        decision: PermissionDecision::deny("no egress from this deployment"),
        layer: PermissionLayer::Deployment,
    };

    let report = execute(
        &ports_with_model_composition_and_capability(
            Arc::clone(&commit),
            Arc::new(FakeModel),
            Arc::new(FakeComposition),
            Arc::clone(&capability) as Arc<dyn CapabilityPort>,
        ),
        denied,
        Value::Null,
    )
    .await
    .expect("a refused capability is a typed outcome, not a driver error");

    assert!(
        capability
            .requests()
            .iter()
            .all(|request| request.capability_ref() != "cap.search"),
        "a denied capability must not reach the port at all"
    );
    let refused = report
        .node_outcomes
        .iter()
        .find_map(|outcome| match outcome {
            NodeOutcome::Capability {
                outcome: CapabilityOutcome::Failed { message },
                ..
            } => Some(message.clone()),
            _ => None,
        })
        .expect("the denied node reports a failed capability outcome");
    assert!(
        refused.contains("no egress from this deployment") && refused.contains("deployment"),
        "the refusal names the reason and the layer that gave it: {refused}"
    );

    // One decision, one fact — for every capability the program invokes, not
    // only the refused one.
    let decided = commit
        .facts()
        .into_iter()
        .filter_map(|fact| {
            fact.is_kind(FactKind::CapabilityAttemptRecorded)
                .then(|| fact.runtime().expect("runtime fact").clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(decided.len(), 2, "one decision per capability invocation");
    let refused_fact = decided
        .iter()
        .find(|fact| fact.capability_ref.as_deref() == Some("cap.search"))
        .expect("the refused capability has its own decision fact");
    assert_eq!(
        refused_fact.permission_decision,
        Some(ResolvedPermission {
            decision: PermissionDecision::deny("no egress from this deployment"),
            layer: PermissionLayer::Deployment,
        }),
        "evidence carries the decision and the layer that produced it"
    );
}

/// An admitted allow changes nothing about the effect, and still leaves the
/// decision auditable.
#[tokio::test]
async fn an_admitted_allow_still_records_which_layer_allowed_it() {
    let capability = Arc::new(RecordingCapability::with_results(["ok", "ok"]));
    let commit = Arc::new(FakeCommit::new());
    let mut allowed = request();
    allowed
        .capability_invocations
        .get_mut("n.cap")
        .expect("test admission")
        .permission = ResolvedPermission {
        decision: PermissionDecision::allow(),
        layer: PermissionLayer::Code,
    };

    execute(
        &ports_with_model_composition_and_capability(
            Arc::clone(&commit),
            Arc::new(FakeModel),
            Arc::new(FakeComposition),
            Arc::clone(&capability) as Arc<dyn CapabilityPort>,
        ),
        allowed,
        Value::Null,
    )
    .await
    .expect("an allowed capability runs");

    assert!(
        capability
            .requests()
            .iter()
            .any(|request| request.capability_ref() == "cap.search"),
        "an allowed capability still reaches the port"
    );
    let decided = commit
        .facts()
        .into_iter()
        .filter(|fact| fact.is_kind(FactKind::CapabilityAttemptRecorded))
        .filter_map(|fact| fact.runtime().cloned())
        .find(|fact| fact.capability_ref.as_deref() == Some("cap.search"))
        .expect("the allowed decision is recorded too");
    assert_eq!(
        decided.permission_decision.map(|resolved| resolved.layer),
        Some(PermissionLayer::Code)
    );
}

/// An admission with no permission decision used to be neither an allow nor a
/// refusal: the driver skipped the evidence fact and fell through to the effect,
/// so the capability ran undecided and left nothing saying no decision existed.
/// The decision is now a required field, so a payload that omits it — the only
/// way such an admission can be built, since the struct no longer has that
/// state — fails to deserialize instead. This covers the `Continuation` case:
/// persisted bytes with no decision are refused at rehydration, not obeyed.
#[test]
fn an_admission_with_no_permission_decision_cannot_be_deserialized() {
    let admitted = request()
        .capability_invocations
        .remove("n.cap")
        .expect("test admission");
    let mut wire = serde_json::to_value(&admitted).expect("admission serializes");

    // With the decision present these are the exact bytes a composition root
    // writes, and they round-trip.
    let round_tripped: CapabilityInvocationAdmission =
        serde_json::from_value(wire.clone()).expect("a decided admission round-trips");
    assert_eq!(round_tripped, admitted);

    // Drop only the decision. Nothing else about the payload changes, so the
    // refusal below is the missing decision and not an unrelated shape problem.
    wire.as_object_mut()
        .expect("admission is a JSON object")
        .remove("permission")
        .expect("a decided admission serializes its decision");
    let error = serde_json::from_value::<CapabilityInvocationAdmission>(wire)
        .expect_err("an admission carrying no decision is not an admission");
    assert!(
        error.to_string().contains("permission"),
        "the refusal names the missing decision field: {error}"
    );
}

/// A Hook binding is matched to AIR by `body_region_id`, and the schedule
/// emits a Hook step only where a region matches. A binding naming a region
/// this AIR does not have used to be silently inert — the Hook's return
/// contract never applied and no evidence said a Hook was meant to run. Two
/// bindings on one region were the same failure by a different route: the
/// schedule's map kept the last one and dropped the other. The compile path
/// caught both against a FrontendGraph; this boundary, where AIR and bindings
/// arrive from separate sources, caught neither.
#[tokio::test]
async fn a_hook_binding_that_matches_no_region_is_refused_before_execution() {
    let mut stale = request();
    stale.hook_bindings[0].body_region_id = "hook.body.that.moved".into();
    let error = execute(&ports(Arc::new(FakeCommit::new())), stale, Value::Null)
        .await
        .expect_err("a Hook binding naming no region is refused, not ignored");
    let ExecutionError::InvalidAir { message } = &error else {
        panic!("expected an InvalidAir refusal, got {error}");
    };
    assert!(
        message.contains("hook.after.model") && message.contains("hook.body.that.moved"),
        "the refusal names the Hook and the region it could not find: {message}"
    );
}

#[tokio::test]
async fn two_hook_bindings_cannot_claim_the_same_body_region() {
    let mut duplicated = request();
    let mut second = duplicated.hook_bindings[0].clone();
    second.hook_id = "hook.after.model.duplicate".into();
    duplicated.hook_bindings.push(second);
    let error = execute(&ports(Arc::new(FakeCommit::new())), duplicated, Value::Null)
        .await
        .expect_err("two Hooks on one body region is a refusal, not a last-wins race");
    let ExecutionError::InvalidAir { message } = &error else {
        panic!("expected an InvalidAir refusal, got {error}");
    };
    assert!(
        message.contains("hook.after.model.duplicate") && message.contains("hook.after.model.body"),
        "the refusal names the losing Hook and the contested region: {message}"
    );
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
async fn capability_arguments_cannot_be_preloaded_from_a_future_ssa_definition() {
    let mut bypass = request();
    let capability = bypass
        .air
        .semantic_operations
        .iter_mut()
        .find(|operation| operation.node_id == "n.cap")
        .expect("capability operation");
    capability
        .operands
        .iter_mut()
        .find(|operand| operand.slot == "arguments")
        .expect("capability arguments")
        .value_id = "value.acp.output".into();
    bypass
        .initial_values
        .insert("value.acp.output".into(), json!({"preloaded": true}));

    let error = execute(&ports(Arc::new(FakeCommit::new())), bypass, Value::Null)
        .await
        .expect_err("future SSA value must not be an executable input");
    assert!(matches!(error, ExecutionError::InvalidAir { .. }));
}

#[tokio::test]
async fn capability_arguments_cannot_be_initialized_through_an_entry_block_argument() {
    let mut bypass = request();
    let mut air = serde_json::to_value(&bypass.air).expect("AIR encodes");
    air["semantic_operations"][1]["operands"][1]["value_id"] = json!("value.cap.entry_argument");
    air["structural_ir"][0]["block_arguments"] = json!([{
        "value_id": "value.cap.entry_argument",
        "type_ref": "CapabilityArguments"
    }]);
    bypass.air = serde_json::from_value(air).expect("entry argument AIR");
    bypass.initial_values =
        BTreeMap::from([("value.cap.entry_argument".into(), json!({"attacker": true}))]);

    assert!(bypass.air.verify().is_accepted());
    let error = execute(&ports(Arc::new(FakeCommit::new())), bypass, Value::Null)
        .await
        .expect_err("entry block argument must not become an external Tool input");
    assert!(matches!(error, ExecutionError::InvalidAir { .. }));
}

#[tokio::test]
async fn capability_argument_assemblies_cannot_read_initial_values() {
    let mut bypass = request();
    let mut air = serde_json::to_value(&bypass.air).expect("AIR encodes");
    air["value_assemblies"] = json!([
        {
            "value_id": "value.cap.arguments",
            "expression": {
                "kind": "object",
                "fields": [{
                    "name": "query",
                    "value": {"kind": "ssa", "value_id": "value.cap.entry_argument"}
                }]
            }
        },
        {
            "value_id": "session.1",
            "expression": {"kind": "string", "value": "session.1"}
        }
    ]);
    air["structural_ir"][0]["block_arguments"] = json!([{
        "value_id": "value.cap.entry_argument",
        "type_ref": "String"
    }]);
    bypass.air = serde_json::from_value(air).expect("assembly dependency AIR");
    bypass.initial_values =
        BTreeMap::from([("value.cap.entry_argument".into(), json!("attacker-query"))]);

    assert!(bypass.air.verify().is_accepted());
    let error = execute(&ports(Arc::new(FakeCommit::new())), bypass, Value::Null)
        .await
        .expect_err("assembly must not read an externally initialized value");
    assert!(matches!(error, ExecutionError::InvalidAir { .. }));
}

#[tokio::test]
async fn capability_dispatch_requires_exact_invocation_admission_for_every_ref() {
    let mut missing = request();
    missing.capability_invocations.remove("n.acp");
    let error = execute(
        &ports_with_model_composition_capability_and_external(
            Arc::new(FakeCommit::new()),
            Arc::new(FakeModel),
            Arc::new(FakeComposition),
            Arc::new(FakeCapability),
            Arc::new(NeverCalledAcp),
        ),
        missing,
        Value::Null,
    )
    .await
    .expect_err("external-agent admission is required");
    assert!(matches!(
        error,
        ExecutionError::MissingCapabilityInvocationAdmission { node_id }
            if node_id == "n.acp"
    ));

    let mut mismatched = request();
    mismatched
        .capability_invocations
        .get_mut("n.acp")
        .expect("external-agent admission")
        .capability_ref = "external-agent:acp:other".into();
    let error = execute(
        &ports_with_model_composition_capability_and_external(
            Arc::new(FakeCommit::new()),
            Arc::new(FakeModel),
            Arc::new(FakeComposition),
            Arc::new(FakeCapability),
            Arc::new(NeverCalledAcp),
        ),
        mismatched,
        Value::Null,
    )
    .await
    .expect_err("external-agent binding mismatch");
    assert!(matches!(
        error,
        ExecutionError::CapabilityInvocationAdmissionMismatch {
            node_id,
            authored,
            admitted,
        } if node_id == "n.acp"
            && authored == "external-agent:acp:claude-code"
            && admitted == "external-agent:acp:other"
    ));
}

#[tokio::test]
async fn invalid_atomic_commit_request_rejects_before_any_effect_path() {
    let commit = Arc::new(FakeCommit::new());
    let mut invalid = request();
    invalid.write_set.runtime_evidence_batch_digest = "not-a-digest".into();

    let error = execute(
        &ports_with_model_composition_capability_and_external(
            commit.clone(),
            Arc::new(FakeModel),
            Arc::new(FakeComposition),
            Arc::new(FakeCapability),
            Arc::new(NeverCalledAcp),
        ),
        invalid,
        Value::Null,
    )
    .await
    .expect_err("malformed atomic request");
    assert!(matches!(error, ExecutionError::InvalidCommitRequest { .. }));
    assert!(
        commit.invocation_refs().is_empty(),
        "rejected commit inputs publish no commit or effect evidence"
    );
}

#[tokio::test]
async fn committed_native_model_usage_is_fail_closed_without_lineage_evidence() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::default());
    let ports = ports(commit.clone()).with_committed_native_model_usage_port(usage.clone());

    let report = execute(&ports, request(), json!({})).await.expect("run");

    assert_eq!(
        report.operational_usage,
        CommittedNativeModelUsageOutcome::Published
    );
    assert_eq!(usage.calls().len(), 1);
    let committed_facts = commit.facts();
    let committed_attempt = committed_facts
        .iter()
        .find_map(Fact::model_attempt_recorded)
        .expect("attempt is committed atomically");
    assert_eq!(committed_attempt.program_invocation_id, "invocation.1");
    assert_eq!(
        committed_attempt.node_execution_id,
        "node-execution.invocation.1.n.model.3"
    );
    assert_eq!(committed_attempt.air_node_id, "n.model");
    assert_eq!(
        committed_attempt.attempt_id,
        "model-attempt.node-execution.invocation.1.n.model.3.0"
    );
    assert_eq!(committed_attempt.attempt_index, 0);
    assert!(
        committed_attempt
            .model_effect_id
            .starts_with("model-effect.")
    );
    assert!(committed_attempt.request_digest.starts_with("sha256:"));
    assert_eq!(committed_attempt.model_target_ref, "model.target");
    assert_eq!(committed_attempt.model_deployment_ref, "deploy.default");
    assert_eq!(committed_attempt.exact_port_binding_digest, digest('a'));
    assert_eq!(committed_attempt.target_generation, 0);
    assert_eq!(usage.calls()[0].attempt.target_generation, 0);
    assert_eq!(committed_attempt.native_input_tokens, 10);
    assert_eq!(committed_attempt.native_output_tokens, 20);
    assert!(
        report.external_agent_evidence.is_empty(),
        "all capability invocations use the generic capability contract"
    );
}

#[tokio::test]
async fn production_dispatch_preserves_nonzero_generation_in_usage_evidence() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::default());
    let mut request = request();
    request.model_admission = admission_at_generation(9);
    let ports = ports(commit).with_committed_native_model_usage_port(usage.clone());

    let report = execute(&ports, request, json!({})).await.expect("run");

    assert_eq!(
        report.operational_usage,
        CommittedNativeModelUsageOutcome::Published
    );
    assert_eq!(usage.calls().len(), 1);
    assert_eq!(usage.calls()[0].attempt.target_generation, 9);
    assert_ne!(
        usage.calls()[0].attempt.generation_cohort_digest,
        usage.calls()[0].attempt.target_commitment_digest
    );
}

#[tokio::test]
async fn production_dispatch_rejects_moving_commitment_before_model_port() {
    let commit = Arc::new(FakeCommit::new());
    let mut request = request();
    let moving = InferenceTargetCommitment::commit(
        "model.target",
        digest('9'),
        "deploy.default",
        digest('a'),
        digest('b'),
        digest('c'),
        9,
    )
    .expect("target commitment");
    let mut moving = moving;
    moving.state = apxm_inference::TargetCommitState::Moving;
    request.model_admission =
        ModelBindingAdmission::new(ResolvedModelBinding::from_target_commitment(moving));

    let error = execute(&ports(commit), request, json!({}))
        .await
        .expect_err("moving commitment must fail before dispatch");
    assert!(matches!(
        error,
        ExecutionError::TargetCommitment(apxm_inference::TargetCommitmentError::Moving)
    ));
}

#[tokio::test]
async fn committed_native_model_usage_decode_rejects_missing_and_unknown_fields() {
    let canonical = serde_json::to_value(lineage_backed_usage()).expect("closed usage serializes");
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

#[test]
fn committed_native_model_usage_allows_commit_bound_sealed_lineage() {
    let usage = lineage_backed_usage();
    let schema_bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../contracts/schemas/apxm.committed-native-model-usage.json"),
    )
    .expect("owning schema bytes");
    assert_eq!(usage.commit_id, "c1");
    assert_eq!(usage.evidence_position_ref.r#ref, "evidence:1");
    assert_eq!(usage.attempt.fact_id, "fact.invocation.1.3");
    assert_eq!(usage.attempt.attempt_index, 0);
    assert_eq!(
        usage.source_contract_digest,
        format!("sha256:{:x}", Sha256::digest(schema_bytes))
    );
    assert_eq!(
        usage.usage_measurement_id,
        CommittedNativeModelUsage::measurement_id(&usage.commit_id, &usage.attempt.fact_id)
    );
}

#[test]
fn committed_native_model_usage_rejects_target_commitment_drift() {
    let attempt = committed_attempt("fact.invocation.1.6", 0, 7, 11);
    let mut lineage = lineage_for_attempt(
        &attempt,
        attempt.request_digest.clone(),
        attempt.model_deployment_ref.clone(),
    );
    lineage
        .bind_evidence(attempt.fact_id.clone(), "c1")
        .expect("bind lineage");
    let mut tampered_attempt = attempt;
    tampered_attempt.target_commitment_digest = digest('f');

    let error = CommittedNativeModelUsage::from_lineage(
        "c1",
        EvidencePositionRef {
            ref_type: EvidencePositionRefType::EvidencePositionRef,
            r#ref: "evidence:1".into(),
        },
        tampered_attempt,
        &lineage,
    )
    .expect_err("target commitment drift must be rejected before publication");
    assert!(matches!(
        error,
        CommittedNativeModelUsageGateError::EvidenceMismatch("target_commitment_digest")
    ));
}

#[test]
fn committed_native_model_usage_rejects_mismatched_lineage() {
    let attempt = committed_attempt("fact.invocation.1.4", 1, 7, 11);
    let mut lineage = lineage_for_attempt(
        &attempt,
        attempt.request_digest.clone(),
        attempt.model_deployment_ref.clone(),
    );
    lineage
        .bind_evidence("fact.other", "c1")
        .expect("bind lineage");

    let error = CommittedNativeModelUsage::from_lineage(
        "c1",
        EvidencePositionRef {
            ref_type: EvidencePositionRefType::EvidencePositionRef,
            r#ref: "evidence:1".into(),
        },
        attempt,
        &lineage,
    )
    .expect_err("mismatched evidence fact must be rejected");
    assert!(matches!(
        error,
        CommittedNativeModelUsageGateError::EvidenceMismatch("fact_id")
    ));
}

#[test]
fn committed_native_model_usage_rejects_request_or_deployment_drift() {
    let attempt = committed_attempt("fact.invocation.1.5", 0, 5, 8);
    let mut lineage = lineage_for_attempt(&attempt, digest('f'), "deploy.other".into());
    lineage
        .bind_evidence(attempt.fact_id.clone(), "c1")
        .expect("bind lineage");

    let error = CommittedNativeModelUsage::from_lineage(
        "c1",
        EvidencePositionRef {
            ref_type: EvidencePositionRefType::EvidencePositionRef,
            r#ref: "evidence:1".into(),
        },
        attempt.clone(),
        &lineage,
    )
    .expect_err("request digest drift must be rejected before publication");
    assert!(matches!(
        error,
        CommittedNativeModelUsageGateError::EvidenceMismatch("request_digest")
    ));

    let mut deployment_lineage = lineage_for_attempt(
        &attempt,
        attempt.request_digest.clone(),
        "deploy.other".into(),
    );
    deployment_lineage
        .bind_evidence(attempt.fact_id.clone(), "c1")
        .expect("bind lineage");

    let error = CommittedNativeModelUsage::from_lineage(
        "c1",
        EvidencePositionRef {
            ref_type: EvidencePositionRefType::EvidencePositionRef,
            r#ref: "evidence:1".into(),
        },
        attempt,
        &deployment_lineage,
    )
    .expect_err("deployment drift must be rejected before publication");
    assert!(matches!(
        error,
        CommittedNativeModelUsageGateError::EvidenceMismatch("model_deployment_ref")
    ));
}

#[tokio::test]
async fn each_native_model_call_publishes_its_commit_bound_lineage() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::default());
    let ports = ports(commit).with_committed_native_model_usage_port(usage.clone());
    let two_models: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "semantic_operations": [
            {"node_id": "n.model.first", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.first.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.first.output", "type_ref": "ModelOutput"}},
            {"node_id": "n.model.second", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 1, "operands": [{"slot": "model_ref", "value_id": "model.target.second", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.second.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.second.output", "type_ref": "ModelOutput"}}
        ],
        "structural_ir": [{"region_id": "r.fn", "kind": "function", "execution_order": 0}],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("valid multi-model AIR");
    assert!(two_models.verify().is_accepted());
    let mut req = request();
    req.air = two_models;
    req.initial_values = BTreeMap::from([
        (
            "value.model.first.request".into(),
            json!({"prompt": "first"}),
        ),
        (
            "value.model.second.request".into(),
            json!({"prompt": "second"}),
        ),
    ]);
    req.hook_bindings = Vec::new();
    req.model_admission = ModelBindingAdmission::for_invocation(vec![
        admission()
            .validate(&ModelTargetRef("model.target".into()))
            .expect("first model binding"),
        ResolvedModelBinding::from_target_commitment(
            InferenceTargetCommitment::commit(
                "model.target.second",
                digest('8'),
                "deploy.second",
                digest('f'),
                digest('b'),
                digest('c'),
                0,
            )
            .expect("target commitment"),
        ),
    ]);

    let report = execute(&ports, req, json!({})).await.expect("run");

    assert_eq!(
        report.operational_usage,
        CommittedNativeModelUsageOutcome::Published
    );
    assert_eq!(usage.calls().len(), 2);
    assert_eq!(
        report.native_usage,
        Usage {
            input_tokens: 20,
            output_tokens: 40
        }
    );
}

#[tokio::test]
async fn retrying_model_usage_publishes_the_successful_attempt_coordinate() {
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
    assert_eq!(usage.calls().len(), 1);
    let request_identities = retrying_model.request_identities();
    assert_eq!(request_identities.len(), 2);
    assert_eq!(request_identities[0], request_identities[1]);
}

#[tokio::test]
async fn publisher_failure_is_reported_after_commit_bound_usage_delivery() {
    let commit = Arc::new(FakeCommit::new());
    let usage = Arc::new(RecordingOperationalUsage::failing(
        CommittedNativeModelUsageError::Unavailable,
    ));
    let ports = ports(commit.clone()).with_committed_native_model_usage_port(usage.clone());

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
    let facts = commit.facts();
    assert!(
        facts
            .iter()
            .any(|fact| fact.model_attempt_recorded().is_some()),
        "canonical attempt evidence remains after exporter loss"
    );
    assert!(
        !apxm_kernel::diagnostic_may_override_evidence(),
        "diagnostics/exporter paths cannot override owner evidence"
    );
}

#[tokio::test]
async fn zero_native_usage_and_uncommitted_execution_emit_nothing() {
    let usage = Arc::new(RecordingOperationalUsage::default());
    let no_model_air: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "value_assemblies": [
            {"value_id": "value.cap.arguments", "expression": {"kind": "object", "fields": [{"name": "query", "value": {"kind": "string", "value": "release checklist"}}]}}
        ],
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
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
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
    let published_count = usage.calls().len();

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
    assert_eq!(usage.calls().len(), published_count);
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

    assert_eq!(usage.calls().len(), 2);
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
        "schema_version": "apxm.air",
        "semantic_operations": [
            {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "program_ref", "value_id": "Specialist", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
            {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.fn", "execution_order": 1, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
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
        "schema_version": "apxm.air",
        "semantic_operations": [
            {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.fn", "execution_order": 0}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .unwrap();
    assert!(!air.verify().is_accepted());
    let mut req = request();
    req.air = air;
    req.hook_bindings = Vec::new();
    let result = execute(&ports(commit.clone()), req, json!({})).await;
    assert!(
        matches!(result, Err(ExecutionError::InvalidAir { .. })),
        "a program.invoke with no receiver must fail closed, not fall back"
    );
}

#[tokio::test]
async fn unbound_model_target_fails_closed() {
    let commit = Arc::new(FakeCommit::new());
    let bad_air: AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "semantic_operations": [
            {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.fn", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.unbound", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}}
        ],
        "structural_ir": [
            {"region_id": "r.fn", "kind": "function", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .unwrap();
    assert!(bad_air.verify().is_accepted());
    let request = ExecutionRequest {
        air: bad_air,
        initial_values: BTreeMap::from([("value.model.request".into(), json!({"prompt": "test"}))]),
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
