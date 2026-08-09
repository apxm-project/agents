//! Generic structural-loop execution and atomic evidence conformance.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use apxm_execution::{
    CapabilityInvocationAdmission, CapabilityOutcome, CapabilityPort, CapabilityRequest,
    CompositionOutcome, CompositionPort, CompositionRequest, EventAwait, EventOutcome, EventPort,
    ExecutionPortBundle, ExecutionPorts, ExecutionRequest, NoopStaticHookHandler, RunOutcome,
    execute, execute_resumable,
};
use apxm_inference::{
    AttemptDisposition, IdempotencyKey, InferenceTargetCommitment, ModelBindingAdmission,
    ModelCallPreparation, ModelCallRequest, ModelCallRequestMetadata, ModelCallRequestMetadataPort,
    ModelContextEnvelopeRef, ModelInferencePort, ModelStreamMode, ResolvedModelBinding, TypedError,
    Usage,
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
use apxm_program::runtime_evidence::{
    Fact, LoopIterationCompletedFact, ProgramIdentity, RuntimeEvidence, RuntimeEvidenceVersion,
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

fn admission() -> ModelBindingAdmission {
    ModelBindingAdmission::new(ResolvedModelBinding::from_target_commitment(
        InferenceTargetCommitment::commit(
            "model.target.v1",
            digest('9'),
            "deployment.default",
            digest('a'),
            digest('b'),
            digest('c'),
            0,
        )
        .expect("target commitment"),
    ))
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

fn decode_air(value: Value) -> AirModule {
    let air: AirModule = serde_json::from_value(value).expect("typed generic AIR");
    assert!(air.verify().is_accepted());
    air
}

fn example_artifact_air(artifact: &str) -> AirModule {
    let value: Value = serde_json::from_str(artifact).expect("example-built executable artifact");
    assert_eq!(
        value["schema_version"], "apxm.executable-artifact.v1",
        "runtime proof consumes the immutable artifact, not handwritten AIR",
    );
    decode_air(value["air"].clone())
}

fn conversational_python_example_air() -> AirModule {
    example_artifact_air(include_str!(
        "../../../machine/program/tests/fixtures/example-artifacts/conversational-python.v1.json"
    ))
}

fn conversational_typescript_example_air() -> AirModule {
    example_artifact_air(include_str!(
        "../../../machine/program/tests/fixtures/example-artifacts/conversational-typescript.v1.json"
    ))
}

fn request(air: AirModule, commit_id: &str) -> ExecutionRequest {
    let capability_invocations = air
        .semantic_operations
        .iter()
        .filter(|operation| operation.op == apxm_program::SemanticOpKind::CapabilityInvoke)
        .filter_map(|operation| {
            let capability_ref = operation
                .operands
                .iter()
                .find(|operand| operand.slot == "capability_ref")?
                .value_id
                .clone();
            if capability_ref.starts_with("external-agent:") {
                return None;
            }
            Some((
                operation.node_id.clone(),
                CapabilityInvocationAdmission {
                    capability_ref,
                    arguments: Value::Null,
                    authority: CapabilityInvocationAuthority::new(
                        "principal.test",
                        "agent.test",
                        "grant.test",
                        Vec::new(),
                    )
                    .expect("valid test authority"),
                },
            ))
        })
        .collect::<BTreeMap<_, _>>();
    ExecutionRequest {
        air,
        hook_bindings: Vec::new(),
        model_admission: admission(),
        capability_invocations,
        program_instance_ref: ProgramInstanceRef::new("instance.1"),
        program_invocation_ref: ProgramInvocationRef::new(format!("invocation.{commit_id}")),
        commit_id: commit_id.into(),
        write_set: write_set(),
    }
}

fn source_map(loop_ids: &[&str]) -> Value {
    json!({
        "schema_version": "apxm.source-map.v1",
        "source_language": "python",
        "node_spans": [],
        "region_annotations": loop_ids
            .iter()
            .map(|loop_id| json!({"region_id": loop_id, "annotation": "structural_loop"}))
            .collect::<Vec<_>>()
    })
}

fn nested_sibling_air() -> AirModule {
    decode_air(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {
                "node_id": "node.outer.before",
                "op": "model.call",
                "parent_region_id": "loop.outer",
                "execution_order": 0,
                "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.outer.before.request", "type_ref": "ModelRequest"}],
                "result": {"value_id": "value.outer.before.output", "type_ref": "ModelOutput"}
            },
            {
                "node_id": "node.inner",
                "op": "model.call",
                "parent_region_id": "loop.inner",
                "execution_order": 0,
                "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.inner.request", "type_ref": "ModelRequest"}],
                "result": {"value_id": "value.inner.output", "type_ref": "ModelOutput"}
            },
            {
                "node_id": "node.outer.after",
                "op": "model.call",
                "parent_region_id": "loop.outer",
                "execution_order": 2,
                "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.outer.after.request", "type_ref": "ModelRequest"}],
                "result": {"value_id": "value.outer.after.output", "type_ref": "ModelOutput"}
            },
            {
                "node_id": "node.sibling",
                "op": "model.call",
                "parent_region_id": "loop.sibling",
                "execution_order": 0,
                "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.sibling.request", "type_ref": "ModelRequest"}],
                "result": {"value_id": "value.sibling.output", "type_ref": "ModelOutput"}
            }
        ],
        "structural_ir": [
            {"region_id": "region.root", "kind": "region", "execution_order": 0},
            {
                "region_id": "loop.outer",
                "kind": "ais.loop",
                "parent_region_id": "region.root",
                "execution_order": 0
            },
            {
                "region_id": "loop.inner",
                "kind": "ais.loop",
                "parent_region_id": "loop.outer",
                "execution_order": 1
            },
            {
                "region_id": "loop.sibling",
                "kind": "ais.loop",
                "parent_region_id": "region.root",
                "execution_order": 1
            }
        ],
        "context_flow": [],
        "source_map": source_map(&["loop.outer", "loop.inner", "loop.sibling"])
    }))
}

fn two_node_loop_air() -> AirModule {
    decode_air(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {
                "node_id": "node.first",
                "op": "model.call",
                "parent_region_id": "loop.main",
                "execution_order": 0,
                "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.first.request", "type_ref": "ModelRequest"}],
                "result": {"value_id": "value.first.output", "type_ref": "ModelOutput"}
            },
            {
                "node_id": "node.second",
                "op": "model.call",
                "parent_region_id": "loop.main",
                "execution_order": 1,
                "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.second.request", "type_ref": "ModelRequest"}],
                "result": {"value_id": "value.second.output", "type_ref": "ModelOutput"}
            }
        ],
        "structural_ir": [
            {"region_id": "loop.main", "kind": "ais.loop", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": source_map(&["loop.main"])
    }))
}

fn interrupted_loop_air(interrupt_kind: &str) -> AirModule {
    let interrupt = if interrupt_kind == "park" {
        None
    } else {
        Some(json!({
            "region_id": format!("control.{interrupt_kind}"),
            "kind": interrupt_kind,
            "parent_region_id": "loop.main",
            "execution_order": 1
        }))
    };
    let mut structural = vec![json!({
        "region_id": "loop.main",
        "kind": "ais.loop",
        "execution_order": 0
    })];
    if let Some(interrupt) = interrupt {
        structural.push(interrupt);
    }
    let operation = if interrupt_kind == "park" {
        json!({
            "node_id": "node.park",
            "op": "await.event",
            "parent_region_id": "loop.main",
            "execution_order": 1,
            "operands": [{"slot": "event_ref", "value_id": "event.input", "type_ref": "EventRef"}],
            "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}
        })
    } else {
        json!({
            "node_id": "node.after",
            "op": "model.call",
            "parent_region_id": "loop.main",
            "execution_order": 2,
            "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.after.request", "type_ref": "ModelRequest"}],
            "result": {"value_id": "value.after.output", "type_ref": "ModelOutput"}
        })
    };
    decode_air(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {
                "node_id": "node.before",
                "op": "model.call",
                "parent_region_id": "loop.main",
                "execution_order": 0,
                "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.before.request", "type_ref": "ModelRequest"}],
                "result": {"value_id": "value.before.output", "type_ref": "ModelOutput"}
            },
            operation
        ],
        "structural_ir": structural,
        "context_flow": [],
        "source_map": source_map(&["loop.main"])
    }))
}

struct SequencedModel {
    outcomes: Mutex<VecDeque<AttemptDisposition>>,
}

impl SequencedModel {
    fn successful() -> Self {
        Self {
            outcomes: Mutex::new(VecDeque::new()),
        }
    }

    fn with(outcomes: impl IntoIterator<Item = AttemptDisposition>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
        }
    }
}

impl ModelInferencePort for SequencedModel {
    fn attempt(&self, _request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| AttemptDisposition::Success(Usage::default()))
    }
}

struct Capability;
#[async_trait]
impl CapabilityPort for Capability {
    async fn invoke(&self, _request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::Completed {
            result: "ok".into(),
        }
    }
}

struct ExternalAgent;
#[async_trait]
impl ExternalAgentCapabilityPort for ExternalAgent {
    async fn prompt(&self, request: AcpPromptRequest) -> AcpPromptOutcome {
        AcpPromptOutcome {
            session_ref: request.session_ref,
            state: PromptEffectState::Completed { stop_reason: None },
            nested_events: Vec::new(),
            peer_usage: Vec::new(),
        }
    }
}

struct Composition;
#[async_trait]
impl CompositionPort for Composition {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Created {
            child_instance_ref: request.receiver.reference().to_string(),
        }
    }

    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Invoked {
            child_instance_ref: request.receiver.reference().to_string(),
        }
    }
}

struct Events {
    park: bool,
}
#[async_trait]
impl EventPort for Events {
    async fn await_event(&self, request: EventAwait) -> EventOutcome {
        if self.park {
            EventOutcome::Parked
        } else {
            EventOutcome::Fulfilled {
                event_ref: request.event_ref,
                payload: "ok".into(),
            }
        }
    }
}

struct RecordingCommit {
    version: Mutex<u64>,
    durable_evidence: Mutex<Vec<Fact>>,
    conflict: bool,
}

impl RecordingCommit {
    fn new(conflict: bool) -> Self {
        Self {
            version: Mutex::new(0),
            durable_evidence: Mutex::new(Vec::new()),
            conflict,
        }
    }

    fn completions(&self) -> Vec<LoopIterationCompletedFact> {
        self.durable_evidence
            .lock()
            .unwrap()
            .iter()
            .filter_map(Fact::loop_iteration_completed)
            .cloned()
            .collect()
    }

    fn evidence(&self) -> RuntimeEvidence {
        RuntimeEvidence {
            schema_version: RuntimeEvidenceVersion::V1,
            program_identity: ProgramIdentity {
                artifact_digest: digest('f'),
                entrypoint: "run".into(),
                agent_identity_binding: "agent.1".into(),
                program_instance_id: Some("instance.1".into()),
            },
            facts: self.durable_evidence.lock().unwrap().clone(),
        }
    }
}

#[async_trait]
impl ExecutionCommitPort for RecordingCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        if self.conflict {
            return ExecutionCommitResult::CompareConflict {
                current_program_state_version: *self.version.lock().unwrap(),
            };
        }
        let mut version = self.version.lock().unwrap();
        *version += 1;
        self.durable_evidence
            .lock()
            .unwrap()
            .extend(request.evidence_batch);
        ExecutionCommitResult::Committed {
            new_program_state_version: *version,
            evidence_position_ref: format!("evidence:{}", *version),
        }
    }

    async fn current_version(&self, _program_instance_ref: &ProgramInstanceRef) -> u64 {
        *self.version.lock().unwrap()
    }

    /// The loop-evidence fixture runs to completion, so it parks no continuation.
    async fn load_continuation(&self, _program_instance_ref: &ProgramInstanceRef) -> Option<Value> {
        None
    }
}

fn ports(
    model: Arc<SequencedModel>,
    commit: Arc<RecordingCommit>,
    park_event: bool,
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
                PortImplementation::Capability(Arc::new(Capability)),
            ),
            (
                binding(PortSlot::ExternalAgentCapability, "apxm.external-agent.v1"),
                PortImplementation::ExternalAgentCapability(Arc::new(ExternalAgent)),
            ),
        ],
    )
    .expect("test ports satisfy the admitted bundle");
    let bundle = ExecutionPortBundle::construct(
        Arc::new(kernel_bundle),
        contract("apxm.durable-event.v1"),
        binding(PortSlot::DurableEvent, "apxm.durable-event.v1"),
        Arc::new(Events { park: park_event }),
        contract("apxm.program-composition.v1"),
        binding(PortSlot::ProgramComposition, "apxm.program-composition.v1"),
        Arc::new(Composition),
    )
    .expect("driver ports satisfy their exact admitted bindings");
    ExecutionPorts::from_admitted_bundle(
        &bundle,
        Arc::new(TestModelRequestMetadata),
        Arc::new(NoopStaticHookHandler),
    )
    .expect("bundle contains every runtime effect port")
}

#[tokio::test]
async fn repository_example_artifacts_execute_only_generic_structural_semantics() {
    for (commit_id, air) in [
        (
            "example.conversational.python",
            conversational_python_example_air(),
        ),
        (
            "example.conversational.typescript",
            conversational_typescript_example_air(),
        ),
    ] {
        assert!(
            air.structural_ir
                .iter()
                .any(|region| region.kind.wire() == "ais.loop"),
            "{commit_id} must contain compiler-emitted ais.loop",
        );
        assert!(
            air.structural_ir
                .iter()
                .any(|region| region.kind.wire() == "yield"),
            "{commit_id} must include compiler-emitted yield",
        );
        let loop_ids = air
            .structural_ir
            .iter()
            .filter(|region| region.kind.wire() == "ais.loop")
            .map(|region| region.region_id.as_str())
            .collect::<Vec<_>>();
        let tool_loop_id = air
            .structural_ir
            .iter()
            .find(|region| {
                region.kind.wire() == "ais.loop"
                    && region
                        .parent_region_id
                        .as_deref()
                        .is_some_and(|parent| loop_ids.contains(&parent))
            })
            .map(|region| region.region_id.clone())
            .expect("the authored Tool-request loop is nested in the conversation loop");
        let encoded = serde_json::to_string(&air).expect("AIR JSON");
        assert!(!encoded.contains("conversational_loop"));

        let commit = Arc::new(RecordingCommit::new(false));
        let report = execute(
            &ports(
                Arc::new(SequencedModel::successful()),
                commit.clone(),
                false,
            ),
            request(air, commit_id),
            Value::Null,
        )
        .await
        .expect("execute example-built artifact");

        assert!(
            report
                .node_outcomes
                .iter()
                .any(|outcome| matches!(outcome, apxm_execution::NodeOutcome::Model { .. })),
            "{commit_id} executes the ordinary model.call operation",
        );
        assert!(
            report
                .node_outcomes
                .iter()
                .all(|outcome| !matches!(outcome, apxm_execution::NodeOutcome::AwaitEvent { .. })),
            "{commit_id} uses structural yield rather than an await.event stand-in",
        );
        assert_eq!(
            commit
                .completions()
                .iter()
                .map(|completion| completion.static_loop_id.as_str())
                .collect::<Vec<_>>(),
            [tool_loop_id.as_str()],
            "{commit_id} commits the inner Tool loop while outer yield remains resumable",
        );
        assert!(commit.evidence().verify().is_accepted());
    }
}

#[tokio::test]
async fn sibling_and_nested_loops_commit_independent_evidence() {
    let commit = Arc::new(RecordingCommit::new(false));
    execute(
        &ports(
            Arc::new(SequencedModel::successful()),
            commit.clone(),
            false,
        ),
        request(nested_sibling_air(), "nested"),
        Value::Null,
    )
    .await
    .expect("execute nested loops");

    let completions = commit.completions();
    assert_eq!(
        completions
            .iter()
            .map(|fact| fact.static_loop_id.as_str())
            .collect::<Vec<_>>(),
        ["loop.inner", "loop.outer", "loop.sibling"]
    );
    assert_eq!(completions[0].causal_node_execution_ids.len(), 1);
    assert_eq!(completions[1].causal_node_execution_ids.len(), 3);
    assert_eq!(completions[2].causal_node_execution_ids.len(), 1);
    assert!(completions.iter().all(|fact| fact.iteration_index == 0));
    assert!(commit.evidence().verify().is_accepted());
}

#[tokio::test]
async fn earlier_body_failure_is_not_erased_by_later_success() {
    let commit = Arc::new(RecordingCommit::new(false));
    let model = Arc::new(SequencedModel::with([
        AttemptDisposition::Cancelled,
        AttemptDisposition::Success(Usage::default()),
    ]));
    execute(
        &ports(model, commit.clone(), false),
        request(two_node_loop_air(), "failure"),
        Value::Null,
    )
    .await
    .expect("execute failed loop body");
    assert!(commit.completions().is_empty());
}

#[tokio::test]
async fn park_yield_and_return_suppress_loop_completion() {
    let parked_commit = Arc::new(RecordingCommit::new(false));
    let parked = execute_resumable(
        &ports(
            Arc::new(SequencedModel::successful()),
            parked_commit.clone(),
            true,
        ),
        request(interrupted_loop_air("park"), "park"),
        Value::Null,
    )
    .await
    .expect("park loop");
    assert!(matches!(parked, RunOutcome::Suspended { .. }));
    assert!(parked_commit.completions().is_empty());

    for kind in ["yield", "return"] {
        let commit = Arc::new(RecordingCommit::new(false));
        execute(
            &ports(
                Arc::new(SequencedModel::successful()),
                commit.clone(),
                false,
            ),
            request(interrupted_loop_air(kind), kind),
            Value::Null,
        )
        .await
        .expect("execute interrupted loop");
        assert!(
            commit.completions().is_empty(),
            "{kind} must suppress completion"
        );
    }
}

#[tokio::test]
async fn compare_conflict_publishes_no_prepared_completion() {
    let commit = Arc::new(RecordingCommit::new(true));
    let report = execute(
        &ports(
            Arc::new(SequencedModel::successful()),
            commit.clone(),
            false,
        ),
        request(two_node_loop_air(), "conflict"),
        Value::Null,
    )
    .await
    .expect("drive to conflicting commit");
    assert!(matches!(
        report.commit,
        ExecutionCommitResult::CompareConflict { .. }
    ));
    assert!(commit.completions().is_empty());
}
