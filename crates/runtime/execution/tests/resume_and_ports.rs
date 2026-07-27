//! Atomic structural continuation conformance for the canonical execution driver.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use apxm_execution::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, Continuation, EventAwait, EventOutcome, EventPort, EventRef,
    ExecutionPorts, ExecutionRequest, NoopStaticHookHandler, OperationalUsageOutcome, RunOutcome,
    execute_resumable, resume_event,
};
use apxm_inference::{
    AttemptDisposition, ExactPortBindingRef, ModelBindingAdmission, ModelCallRequest,
    ModelDeploymentRef, ModelInferencePort, ModelTargetRef, ResolvedModelBinding, Usage,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AtomicWriteSet, ExactPortBinding, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExternalAgentCapabilityPort, PortBundle,
    PortBundleSpec, PortImplementation, PortSlot, PromptEffectState,
};
use apxm_program::air::AirModule;
use apxm_program::artifact::SchemaDigestRef;

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn request(scope: &str) -> ExecutionRequest {
    let air = serde_json::from_value::<AirModule>(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "node.model", "op": "model.call", "parent_region_id": "loop.main", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}},
            {"node_id": "node.await", "op": "await.event", "parent_region_id": "loop.main", "execution_order": 1, "operands": [{"slot": "event_ref", "value_id": "evt-atomic", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}},
            {"node_id": "node.capability", "op": "capability.invoke", "parent_region_id": "loop.main", "execution_order": 2, "operands": [{"slot": "capability_ref", "value_id": "cap.finish", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.capability.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.capability.output", "type_ref": "CapabilityOutput"}}
        ],
        "structural_ir": [
            {"region_id": "region.root", "kind": "function", "execution_order": 0},
            {"region_id": "loop.main", "kind": "ais.loop", "parent_region_id": "region.root", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": [{"region_id": "loop.main", "annotation": "structural_loop"}]}
    }))
    .expect("canonical AIR");
    assert!(air.verify().is_accepted());
    ExecutionRequest {
        air,
        hook_bindings: Vec::new(),
        model_admission: ModelBindingAdmission::new(ResolvedModelBinding {
            model_target_ref: ModelTargetRef("model.target.v1".into()),
            model_deployment_ref: ModelDeploymentRef("deployment.default".into()),
            exact_port_binding: ExactPortBindingRef {
                binding_digest: digest('a'),
            },
        }),
        invocation_ref: format!("invocation.{scope}"),
        version_scope: scope.into(),
        commit_id: format!("commit.{scope}"),
        write_set: AtomicWriteSet {
            next_program_state_digest: digest('1'),
            continuation_digest: digest('2'),
            checkpoint_effect_outcomes_digest: digest('3'),
            runtime_evidence_batch_digest: digest('4'),
            usage_facts_digest: digest('5'),
            session_output_refs_digest: digest('6'),
        },
    }
}

struct Model;
impl ModelInferencePort for Model {
    fn attempt(&self, _request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        AttemptDisposition::Success(Usage {
            input_tokens: 3,
            output_tokens: 5,
        })
    }
}

struct Capability;
#[async_trait]
impl CapabilityPort for Capability {
    async fn invoke(&self, _request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::Completed {
            result: "completed".into(),
        }
    }
}

struct External;
#[async_trait]
impl ExternalAgentCapabilityPort for External {
    async fn prompt(&self, request: AcpPromptRequest) -> AcpPromptOutcome {
        AcpPromptOutcome {
            session_ref: request.session_ref,
            state: PromptEffectState::Completed { stop_reason: None },
            nested_events: Vec::new(),
            peer_usage: Vec::new(),
        }
    }
}

struct Events;
#[async_trait]
impl EventPort for Events {
    async fn await_event(&self, _request: EventAwait) -> EventOutcome {
        EventOutcome::Parked
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

#[derive(Default)]
struct Commit {
    version: Mutex<u64>,
    continuation: Mutex<Option<Value>>,
    tuples: Mutex<Vec<apxm_kernel::ExecutionCommitTuple>>,
    fail: bool,
}

#[async_trait]
impl ExecutionCommitPort for Commit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        if self.fail {
            return ExecutionCommitResult::OutcomeUnknown {
                reconciliation_ref: "crash.before.commit".into(),
            };
        }
        let mut version = self.version.lock().unwrap();
        *version = request.expected_program_state_version + 1;
        *self.continuation.lock().unwrap() = request.tuple.continuation.clone();
        self.tuples.lock().unwrap().push(request.tuple);
        ExecutionCommitResult::Committed {
            new_program_state_version: *version,
            evidence_position_ref: "evidence.atomic".into(),
        }
    }

    async fn current_version(&self, _invocation_ref: &str) -> u64 {
        *self.version.lock().unwrap()
    }

    async fn load_continuation(&self, _invocation_ref: &str) -> Option<Value> {
        self.continuation.lock().unwrap().clone()
    }
}

fn ports(commit: Arc<Commit>) -> ExecutionPorts {
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
                PortImplementation::ModelInference(Arc::new(Model)),
            ),
            (
                binding(PortSlot::Capability, "apxm.capability.v1"),
                PortImplementation::Capability(Arc::new(Capability)),
            ),
            (
                binding(PortSlot::ExternalAgentCapability, "apxm.external-agent.v1"),
                PortImplementation::ExternalAgentCapability(Arc::new(External)),
            ),
        ],
    )
    .expect("test ports satisfy the admitted bundle");
    ExecutionPorts::from_admitted_bundle(
        &bundle,
        Arc::new(Events),
        Arc::new(Composition),
        Arc::new(NoopStaticHookHandler),
    )
    .expect("bundle contains every runtime effect port")
}

#[tokio::test]
async fn park_commits_context_continuation_wait_effects_evidence_usage_and_outputs_together() {
    let commit = Arc::new(Commit::default());
    let outcome = execute_resumable(
        &ports(commit.clone()),
        request("instance.atomic"),
        json!({"iteration": 1}),
    )
    .await
    .expect("park commits atomically");

    assert!(matches!(
        outcome,
        RunOutcome::Suspended {
            continuation_id,
            event_ref: Some(event_ref),
            operational_usage: OperationalUsageOutcome::NotConfigured,
        } if continuation_id == "node.await" && event_ref.as_str() == "evt-atomic"
    ));
    assert_eq!(*commit.version.lock().unwrap(), 1);
    let tuples = commit.tuples.lock().unwrap();
    assert_eq!(tuples.len(), 1);
    let tuple = &tuples[0];
    assert_eq!(tuple.context, json!({"iteration": 1}));
    assert!(tuple.continuation.is_some());
    let continuation: Continuation =
        serde_json::from_value(tuple.continuation.clone().unwrap()).expect("typed continuation");
    assert_eq!(continuation.invocation_ref, "invocation.instance.atomic");
    assert!(continuation.next_schedule_position > 0);
    assert_eq!(continuation.loop_frames.len(), 1);
    assert_eq!(continuation.loop_frames[0].static_loop_id, "loop.main");
    assert_eq!(continuation.loop_frames[0].iteration_index, 0);
    assert!(continuation.loop_frames[0].parked);
    assert!(!continuation.loop_frames[0].failed);
    assert_eq!(
        continuation.loop_frames[0].causal_node_execution_ids.len(),
        1
    );
    assert_eq!(
        tuple.event_wait,
        Some(json!({
            "continuation_id": "node.await",
            "event_ref": "evt-atomic",
        }))
    );
    assert_eq!(tuple.usage, json!({"input_tokens": 3, "output_tokens": 5}));
    assert!(!tuple.evidence.is_empty());
}

#[tokio::test]
async fn crash_before_atomic_commit_leaves_no_context_or_continuation_to_replay() {
    let commit = Arc::new(Commit {
        fail: true,
        ..Commit::default()
    });
    let error = execute_resumable(
        &ports(commit.clone()),
        request("instance.crash"),
        json!({"iteration": 1}),
    )
    .await
    .expect_err("an uncertain atomic commit cannot report a durable suspension");

    assert!(matches!(error, apxm_execution::ExecutionError::Commit(_)));
    assert_eq!(*commit.version.lock().unwrap(), 0);
    assert!(commit.continuation.lock().unwrap().is_none());
    assert!(commit.tuples.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resume_reads_the_committed_structural_continuation() {
    let commit = Arc::new(Commit::default());
    execute_resumable(
        &ports(commit.clone()),
        request("instance.replay"),
        json!({"iteration": 1}),
    )
    .await
    .expect("park");

    let resumed = resume_event(
        &ports(commit.clone()),
        "instance.replay",
        EventRef::new("evt-atomic").expect("non-empty event ref"),
        json!({"iteration": 2}),
    )
    .await
    .expect("resume from committed state");
    assert!(matches!(resumed, RunOutcome::Completed(_)));
    assert_eq!(*commit.version.lock().unwrap(), 2);
    let tuples = commit.tuples.lock().unwrap();
    let completions = tuples
        .iter()
        .flat_map(|tuple| tuple.evidence.iter())
        .filter(|fact| fact.loop_iteration_completed().is_some())
        .count();
    assert_eq!(completions, 1);
}

#[tokio::test]
async fn final_await_resumes_directly_to_one_committed_back_edge() {
    let commit = Arc::new(Commit::default());
    let mut final_await = request("instance.final-await");
    final_await
        .air
        .semantic_operations
        .retain(|operation| operation.node_id != "node.capability");

    execute_resumable(
        &ports(commit.clone()),
        final_await,
        json!({"phase": "before"}),
    )
    .await
    .expect("final await parks");
    let continuation: Continuation = serde_json::from_value(
        commit
            .continuation
            .lock()
            .unwrap()
            .clone()
            .expect("committed continuation"),
    )
    .expect("typed continuation");
    assert_eq!(continuation.loop_frames.len(), 1);

    resume_event(
        &ports(commit.clone()),
        "instance.final-await",
        EventRef::new("evt-atomic").unwrap(),
        json!({"phase": "after"}),
    )
    .await
    .expect("resume final await");
    let tuples = commit.tuples.lock().unwrap();
    assert_eq!(
        tuples
            .iter()
            .flat_map(|tuple| tuple.evidence.iter())
            .filter(|fact| fact.loop_iteration_completed().is_some())
            .count(),
        1
    );
}

#[tokio::test]
async fn nested_loop_park_restores_exact_stack_without_duplicate_work() {
    let commit = Arc::new(Commit::default());
    let mut nested = request("instance.nested");
    nested.air = serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "node.outer.before", "op": "model.call", "parent_region_id": "loop.outer", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.outer.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.outer.output", "type_ref": "ModelOutput"}},
            {"node_id": "node.inner.await", "op": "await.event", "parent_region_id": "loop.inner", "execution_order": 0, "operands": [{"slot": "event_ref", "value_id": "evt-atomic", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}},
            {"node_id": "node.outer.after", "op": "capability.invoke", "parent_region_id": "loop.outer", "execution_order": 2, "operands": [{"slot": "capability_ref", "value_id": "cap.finish", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.capability.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.capability.output", "type_ref": "CapabilityOutput"}}
        ],
        "structural_ir": [
            {"region_id": "region.root", "kind": "function", "execution_order": 0},
            {"region_id": "loop.outer", "kind": "ais.loop", "parent_region_id": "region.root", "execution_order": 0},
            {"region_id": "loop.inner", "kind": "ais.loop", "parent_region_id": "loop.outer", "execution_order": 1}
        ],
        "context_flow": [],
        "source_map": {
            "schema_version": "apxm.source-map.v1",
            "source_language": "python",
            "node_spans": [],
            "region_annotations": [
                {"region_id": "loop.outer", "annotation": "structural_loop"},
                {"region_id": "loop.inner", "annotation": "structural_loop"}
            ]
        }
    }))
    .expect("nested AIR");
    assert!(nested.air.verify().is_accepted());

    execute_resumable(&ports(commit.clone()), nested, Value::Null)
        .await
        .expect("nested await parks");
    let continuation: Continuation =
        serde_json::from_value(commit.continuation.lock().unwrap().clone().unwrap())
            .expect("typed nested continuation");
    assert_eq!(
        continuation
            .loop_frames
            .iter()
            .map(|frame| frame.static_loop_id.as_str())
            .collect::<Vec<_>>(),
        ["loop.outer", "loop.inner"]
    );

    resume_event(
        &ports(commit.clone()),
        "instance.nested",
        EventRef::new("evt-atomic").unwrap(),
        Value::Null,
    )
    .await
    .expect("nested resume");

    let tuples = commit.tuples.lock().unwrap();
    let facts: Vec<_> = tuples
        .iter()
        .flat_map(|tuple| tuple.evidence.iter())
        .collect();
    assert_eq!(
        facts
            .iter()
            .filter(|fact| {
                fact.is_kind(apxm_program::runtime_evidence::FactKind::EventAwaitRegistered)
            })
            .count(),
        1
    );
    assert_eq!(
        facts
            .iter()
            .filter_map(|fact| fact.loop_iteration_completed())
            .count(),
        2
    );
    let mut node_ids = facts
        .iter()
        .filter_map(|fact| {
            fact.node_execution_recorded()
                .map(|node| node.node_execution_id.clone())
        })
        .collect::<Vec<_>>();
    let observed_node_ids = node_ids.clone();
    let total = node_ids.len();
    node_ids.sort();
    node_ids.dedup();
    assert_eq!(
        node_ids.len(),
        total,
        "resume duplicated a NodeExecution: {observed_node_ids:?}"
    );
}
