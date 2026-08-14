//! Atomic structural continuation conformance for the canonical execution driver.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use apxm_execution::{
    CapabilityInvocationAdmission, CapabilityOutcome, CapabilityPort, CapabilityRequest,
    CommittedNativeModelUsageOutcome, CompositionOutcome, CompositionPort, CompositionRequest,
    Continuation, EventAwait, EventOutcome, EventPort, EventRef, ExecutionPortBundle,
    ExecutionPorts, ExecutionRequest, NodeOutcome, NoopStaticHookHandler, RunOutcome,
    execute_resumable, resume, resume_event,
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

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn request(scope: &str) -> ExecutionRequest {
    let air = serde_json::from_value::<AirModule>(json!({
        "schema_version": "apxm.air",
        "semantic_operations": [
            {"node_id": "node.model", "op": "model.call", "parent_region_id": "loop.main", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}},
            {"node_id": "node.await", "op": "await.event", "parent_region_id": "loop.main", "execution_order": 1, "operands": [{"slot": "event_ref", "value_id": "evt-atomic", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}},
            {"node_id": "node.capability", "op": "capability.invoke", "parent_region_id": "loop.main", "execution_order": 2, "operands": [{"slot": "capability_ref", "value_id": "cap.finish", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.capability.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.capability.output", "type_ref": "CapabilityOutput"}}
        ],
        "structural_ir": [
            {"region_id": "region.root", "kind": "function", "execution_order": 0},
            {"region_id": "loop.main", "kind": "ais.loop", "parent_region_id": "region.root", "execution_order": 0}
        ],
        "value_assemblies": [{"value_id": "value.capability.arguments", "expression": {"kind": "object", "fields": [{"name": "status", "value": {"kind": "string", "value": "completed"}}]}}],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": [{"region_id": "loop.main", "annotation": "structural_loop"}]}
    }))
    .expect("canonical AIR");
    assert!(air.verify().is_accepted());
    let capability_invocations = BTreeMap::from([(
        "node.capability".to_string(),
        CapabilityInvocationAdmission {
            capability_ref: "cap.finish".into(),
            authority: CapabilityInvocationAuthority::new(
                "principal.test",
                "agent.test",
                "grant.finish",
                Vec::new(),
            )
            .expect("valid test authority"),
            permission: None,
        },
    )]);
    ExecutionRequest {
        initial_values: air
            .semantic_operations
            .iter()
            .flat_map(|operation| {
                operation
                    .operands
                    .iter()
                    .filter_map(|operand| match operand.slot.as_str() {
                        "request" => Some((operand.value_id.clone(), json!({"prompt": "test"}))),
                        "arguments" => None,
                        _ => None,
                    })
            })
            .collect(),
        air,
        hook_bindings: Vec::new(),
        model_admission: ModelBindingAdmission::new(ResolvedModelBinding::from_target_commitment(
            InferenceTargetCommitment::commit(
                "model.target",
                digest('9'),
                "deployment.default",
                digest('a'),
                digest('b'),
                digest('c'),
                0,
            )
            .expect("target commitment"),
        )),
        capability_invocations,
        program_instance_ref: ProgramInstanceRef::new(scope),
        program_invocation_ref: ProgramInvocationRef::new(format!("invocation.{scope}")),
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

struct Model;
impl ModelInferencePort for Model {
    fn attempt(&self, request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        AttemptDisposition::Success {
            usage: Usage {
                input_tokens: 3,
                output_tokens: 5,
            },
            output: request.authored_request().clone(),
        }
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
    instance_refs: Mutex<Vec<String>>,
    invocation_refs: Mutex<Vec<String>>,
    continuation_load_refs: Mutex<Vec<String>>,
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
        self.instance_refs
            .lock()
            .unwrap()
            .push(request.program_instance_ref.as_str().to_string());
        self.invocation_refs
            .lock()
            .unwrap()
            .push(request.program_invocation_ref.as_str().to_string());
        ExecutionCommitResult::Committed {
            new_program_state_version: *version,
            evidence_position_ref: "evidence.atomic".into(),
        }
    }

    async fn current_version(&self, program_instance_ref: &ProgramInstanceRef) -> u64 {
        self.instance_refs
            .lock()
            .unwrap()
            .push(program_instance_ref.as_str().to_string());
        *self.version.lock().unwrap()
    }

    async fn load_continuation(&self, program_instance_ref: &ProgramInstanceRef) -> Option<Value> {
        self.continuation_load_refs
            .lock()
            .unwrap()
            .push(program_instance_ref.as_str().to_string());
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
                PortImplementation::ModelInference(Arc::new(Model)),
            ),
            (
                binding(PortSlot::Capability, "apxm.capability-invocation"),
                PortImplementation::Capability(Arc::new(Capability)),
            ),
            (
                binding(PortSlot::ExternalAgentCapability, "apxm.external-agent"),
                PortImplementation::ExternalAgentCapability(Arc::new(External)),
            ),
        ],
    )
    .expect("test ports satisfy the admitted bundle");
    let bundle = ExecutionPortBundle::construct(
        Arc::new(kernel_bundle),
        contract("apxm.durable-event"),
        binding(PortSlot::DurableEvent, "apxm.durable-event"),
        Arc::new(Events),
        contract("apxm.program-composition"),
        binding(PortSlot::ProgramComposition, "apxm.program-composition"),
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
            operational_usage: CommittedNativeModelUsageOutcome::NotConfigured,
        } if continuation_id == "node.await" && event_ref.as_str() == "evt-atomic"
    ));
    assert_eq!(*commit.version.lock().unwrap(), 1);
    assert_eq!(
        *commit.invocation_refs.lock().unwrap(),
        vec!["invocation.instance.atomic"]
    );
    let tuples = commit.tuples.lock().unwrap();
    assert_eq!(tuples.len(), 1);
    let tuple = &tuples[0];
    assert_eq!(tuple.context, json!({"iteration": 1}));
    assert!(tuple.continuation.is_some());
    let continuation: Continuation =
        serde_json::from_value(tuple.continuation.clone().unwrap()).expect("typed continuation");
    assert_eq!(
        continuation.program_instance_ref,
        ProgramInstanceRef::new("instance.atomic")
    );
    assert_eq!(
        continuation.program_invocation_ref,
        ProgramInvocationRef::new("invocation.instance.atomic")
    );
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
        &ProgramInstanceRef::new("instance.replay"),
        EventRef::new("evt-atomic").expect("non-empty event ref"),
        json!({"iteration": 2}),
    )
    .await
    .expect("resume from committed state");
    let RunOutcome::Completed(report) = resumed else {
        panic!("event resume must complete");
    };
    assert_eq!(report.final_context, json!({"iteration": 1}));
    assert_eq!(*commit.version.lock().unwrap(), 2);
    assert_eq!(
        *commit.invocation_refs.lock().unwrap(),
        vec!["invocation.instance.replay", "invocation.instance.replay"]
    );
    assert_eq!(
        *commit.continuation_load_refs.lock().unwrap(),
        vec!["instance.replay"]
    );
    assert_eq!(
        *commit.instance_refs.lock().unwrap(),
        vec![
            "instance.replay",
            "instance.replay",
            "instance.replay",
            "instance.replay",
        ]
    );
    let tuples = commit.tuples.lock().unwrap();
    let completions = tuples
        .iter()
        .flat_map(|tuple| tuple.evidence.iter())
        .filter(|fact| fact.loop_iteration_completed().is_some())
        .count();
    assert_eq!(completions, 1);
}

#[tokio::test]
async fn structural_yield_binds_delivered_input_without_overwriting_context() {
    let commit = Arc::new(Commit::default());
    let mut yielded = request("instance.yield-input");
    yielded.air = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "semantic_operations": [{
            "node_id": "node.after-yield",
            "op": "model.call",
            "parent_region_id": "region.root",
            "execution_order": 1,
            "operands": [
                {"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"},
                {"slot": "request", "value_id": "value.resume.input", "type_ref": "ConversationInput"}
            ],
            "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}
        }],
        "structural_ir": [
            {"region_id": "region.root", "kind": "function", "execution_order": 0},
            {"region_id": "yield.input", "kind": "yield", "parent_region_id": "region.root", "execution_order": 0, "block_arguments": [{"value_id": "value.resume.input", "type_ref": "ConversationInput"}]},
            {"region_id": "return.done", "kind": "return", "parent_region_id": "region.root", "execution_order": 2}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("yield-resume AIR");
    yielded.initial_values.clear();
    assert!(yielded.air.verify().is_accepted());

    execute_resumable(
        &ports(commit.clone()),
        yielded,
        json!({"persistent": "context"}),
    )
    .await
    .expect("structural yield parks");
    let continuation: Continuation = serde_json::from_value(
        commit
            .continuation
            .lock()
            .unwrap()
            .clone()
            .expect("continuation"),
    )
    .expect("typed continuation");
    assert_eq!(
        continuation.resume_value_id.as_deref(),
        Some("value.resume.input")
    );

    let resumed = resume(
        &ports(commit),
        &ProgramInstanceRef::new("instance.yield-input"),
        json!({"message": "next"}),
    )
    .await
    .expect("yield delivery resumes");
    let RunOutcome::Completed(report) = resumed else {
        panic!("yield resume must complete");
    };
    assert_eq!(report.final_context, json!({"persistent": "context"}));
    assert!(matches!(
        &report.node_outcomes[0],
        NodeOutcome::Model { result, .. } if result == &json!({"message": "next"})
    ));
}

#[tokio::test]
async fn branch_decision_survives_an_await_inside_the_selected_arm() {
    let commit = Arc::new(Commit::default());
    let mut branched = request("instance.branch-await");
    branched.air = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "semantic_operations": [{
            "node_id": "node.branch.await",
            "op": "await.event",
            "parent_region_id": "branch.then",
            "execution_order": 0,
            "operands": [{"slot": "event_ref", "value_id": "evt-atomic", "type_ref": "EventRef"}],
            "result": {"value_id": "value.await.result", "type_ref": "EventOutput"}
        }],
        "structural_ir": [
            {"region_id": "region.root", "kind": "function", "execution_order": 0, "block_arguments": [{"value_id": "value.condition", "type_ref": "Boolean"}]},
            {"region_id": "branch.main", "kind": "branch", "parent_region_id": "region.root", "execution_order": 0, "predicate": {"root_value_id": "value.condition", "property_path": [], "comparator": "truthy"}},
            {"region_id": "branch.then", "kind": "region", "parent_region_id": "branch.main", "execution_order": 0},
            {"region_id": "branch.else", "kind": "region", "parent_region_id": "branch.main", "execution_order": 1},
            {"region_id": "throw.unselected", "kind": "throw", "parent_region_id": "branch.else", "execution_order": 0},
            {"region_id": "return.done", "kind": "return", "parent_region_id": "region.root", "execution_order": 1}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("branch-await AIR");
    branched.initial_values = BTreeMap::from([("value.condition".into(), json!(true))]);
    assert!(branched.air.verify().is_accepted());

    execute_resumable(&ports(commit.clone()), branched, json!({"context": 1}))
        .await
        .expect("selected branch awaits");
    let continuation: Continuation = serde_json::from_value(
        commit
            .continuation
            .lock()
            .unwrap()
            .clone()
            .expect("continuation"),
    )
    .expect("typed continuation");
    assert_eq!(continuation.branch_decisions.get("branch.main"), Some(&0));

    let resumed = resume_event(
        &ports(commit),
        &ProgramInstanceRef::new("instance.branch-await"),
        EventRef::new("evt-atomic").unwrap(),
        json!({"approved": true}),
    )
    .await
    .expect("resume retains selected branch");
    assert!(matches!(resumed, RunOutcome::Completed(_)));
}

#[tokio::test]
async fn resumed_input_cannot_become_a_capability_argument() {
    let commit = Arc::new(Commit::default());
    let mut yielded = request("instance.resume-capability-input");
    yielded.air = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "semantic_operations": [{
            "node_id": "node.capability",
            "op": "capability.invoke",
            "parent_region_id": "region.root",
            "execution_order": 1,
            "operands": [
                {"slot": "capability_ref", "value_id": "cap.finish", "type_ref": "CapabilityRef"},
                {"slot": "arguments", "value_id": "value.resume.input", "type_ref": "CapabilityArguments"}
            ],
            "result": {"value_id": "value.capability.output", "type_ref": "CapabilityOutput"}
        }],
        "structural_ir": [
            {"region_id": "region.root", "kind": "function", "execution_order": 0},
            {"region_id": "yield.input", "kind": "yield", "parent_region_id": "region.root", "execution_order": 0, "block_arguments": [{"value_id": "value.resume.input", "type_ref": "ConversationInput"}]},
            {"region_id": "return.done", "kind": "return", "parent_region_id": "region.root", "execution_order": 2}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("resume capability AIR");
    yielded.initial_values.clear();
    let verdict = yielded.air.verify();
    assert!(!verdict.is_accepted(), "{verdict:?}");

    let error = execute_resumable(
        &ports(commit.clone()),
        yielded,
        json!({"persistent": "context"}),
    )
    .await
    .expect_err("resume input must be rejected before any park or dispatch");
    assert!(matches!(
        error,
        apxm_execution::ExecutionError::InvalidAir { .. }
    ));
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
        &ProgramInstanceRef::new("instance.final-await"),
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
        "schema_version": "apxm.air",
        "semantic_operations": [
            {"node_id": "node.outer.before", "op": "model.call", "parent_region_id": "loop.outer", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.outer.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.outer.output", "type_ref": "ModelOutput"}},
            {"node_id": "node.inner.await", "op": "await.event", "parent_region_id": "loop.inner", "execution_order": 0, "operands": [{"slot": "event_ref", "value_id": "evt-atomic", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}},
            {"node_id": "node.outer.after", "op": "capability.invoke", "parent_region_id": "loop.outer", "execution_order": 2, "operands": [{"slot": "capability_ref", "value_id": "cap.finish", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.capability.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.capability.output", "type_ref": "CapabilityOutput"}}
        ],
        "structural_ir": [
            {"region_id": "region.root", "kind": "function", "execution_order": 0},
            {"region_id": "loop.outer", "kind": "ais.loop", "parent_region_id": "region.root", "execution_order": 0},
            {"region_id": "loop.inner", "kind": "ais.loop", "parent_region_id": "loop.outer", "execution_order": 1}
        ],
        "value_assemblies": [{"value_id": "value.capability.arguments", "expression": {"kind": "object", "fields": [{"name": "status", "value": {"kind": "string", "value": "completed"}}]}}],
        "context_flow": [],
        "source_map": {
            "schema_version": "apxm.source-map",
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
    nested.initial_values =
        BTreeMap::from([("value.outer.request".into(), json!({"prompt": "nested"}))]);
    nested.capability_invocations = BTreeMap::from([(
        "node.outer.after".to_string(),
        CapabilityInvocationAdmission {
            capability_ref: "cap.finish".into(),
            authority: CapabilityInvocationAuthority::new(
                "principal.test",
                "agent.test",
                "grant.finish",
                Vec::new(),
            )
            .expect("valid test authority"),
            permission: None,
        },
    )]);

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
        &ProgramInstanceRef::new("instance.nested"),
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
