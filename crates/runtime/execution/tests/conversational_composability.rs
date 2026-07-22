//! Owner-local composability conformance: two successive invocations preserve
//! committed Context across a conversational loop yield boundary.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use apxm_inference::{
    AttemptDisposition, ExactPortBindingRef, ModelBindingAdmission, ModelCallRequest,
    ModelDeploymentRef, ModelInferencePort, ModelTargetRef, ResolvedModelBinding, Usage,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AtomicWriteSet, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExternalAgentCapabilityPort, PromptEffectState,
};
use apxm_program::ExecutableArtifact;
use apxm_program::air::{AirModule, SemanticOpKind};
use apxm_program::runtime_evidence::{Fact, FactKind};

use apxm_execution::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionPorts, ExecutionRequest,
    NoopStaticHookHandler, RunOutcome, execute_resumable, resume,
};

fn agents_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

fn load_gao_air() -> AirModule {
    let root = agents_root();
    let python = Command::new("python")
        .current_dir(&root)
        .env(
            "PYTHONPATH",
            root.join("crates/compiler/frontend/python"),
        )
        .args([
            "-c",
            "import json; import apxm_program; print(json.dumps(apxm_program.compile_artifact(apxm_program.build_gao().build_graph()), separators=(',', ':')))",
        ])
        .output()
        .expect("run public Python ConversationalAgent authoring");
    assert!(
        python.status.success(),
        "public Python authoring failed: {}",
        String::from_utf8_lossy(&python.stderr)
    );
    let typescript = Command::new("node")
        .current_dir(&root)
        .args([
            "--experimental-strip-types",
            "crates/compiler/frontend/native/typescript/js/emit_air.ts",
            "gao-artifact",
        ])
        .output()
        .expect("run public TypeScript Gao authoring");
    assert!(
        typescript.status.success(),
        "public TypeScript authoring failed: {}",
        String::from_utf8_lossy(&typescript.stderr)
    );
    let python_artifact = String::from_utf8(python.stdout).expect("Python artifact is UTF-8");
    let typescript_artifact =
        String::from_utf8(typescript.stdout).expect("TypeScript artifact is UTF-8");
    let artifact = ExecutableArtifact::decode(python_artifact.trim().as_bytes())
        .expect("decode public-authored Gao artifact");
    let typescript_artifact =
        ExecutableArtifact::decode(typescript_artifact.trim().as_bytes())
            .expect("decode public-authored TypeScript Gao artifact");
    assert_eq!(
        artifact.air.semantic_operations,
        typescript_artifact.air.semantic_operations,
        "public frontends compile identical Gao semantic operations"
    );
    assert_eq!(
        artifact.air.structural_ir,
        typescript_artifact.air.structural_ir,
        "public frontends compile identical Gao structure"
    );
    assert_eq!(
        artifact.source_map.region_annotations,
        typescript_artifact.source_map.region_annotations,
        "public frontends preserve equivalent conversational regions"
    );
    assert_eq!(artifact.hook_bindings, typescript_artifact.hook_bindings);
    assert_eq!(artifact.entrypoints, typescript_artifact.entrypoints);
    assert_eq!(
        artifact.artifact_semantic_requirements,
        typescript_artifact.artifact_semantic_requirements
    );
    assert!(
        artifact.validate().is_accepted(),
        "public-authored Gao artifact passes canonical validation"
    );
    artifact.air
}

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
    ModelBindingAdmission::new(ResolvedModelBinding {
        model_target_ref: ModelTargetRef("model.default".into()),
        model_deployment_ref: ModelDeploymentRef("deploy.default".into()),
        exact_port_binding: ExactPortBindingRef {
            binding_digest: digest('a'),
        },
    })
}

fn conversational_request(version_scope: &str, commit_id: &str) -> ExecutionRequest {
    let mut air = load_gao_air();
    let await_event = air
        .semantic_operations
        .iter_mut()
        .find(|operation| operation.op == SemanticOpKind::AwaitEvent)
        .expect("conversational fixture has an await.event node");
    await_event
        .operands
        .get_or_insert_default()
        .insert("event_ref".into(), json!("event.turn.input"));
    ExecutionRequest {
        air,
        hook_bindings: Vec::new(),
        model_admission: admission(),
        version_scope: version_scope.into(),
        commit_id: commit_id.into(),
        write_set: write_set(),
    }
}

struct FakeModel;
impl ModelInferencePort for FakeModel {
    fn attempt(&self, _request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        AttemptDisposition::Success(Usage {
            input_tokens: 5,
            output_tokens: 9,
        })
    }
}

struct FakeCapability;
#[async_trait]
impl CapabilityPort for FakeCapability {
    async fn invoke(&self, _request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::Completed {
            result: "tool-ok".into(),
        }
    }
}

struct FakeAcpPeer;
#[async_trait]
impl ExternalAgentCapabilityPort for FakeAcpPeer {
    async fn prompt(&self, request: AcpPromptRequest) -> AcpPromptOutcome {
        AcpPromptOutcome {
            session_ref: request.session_ref,
            state: PromptEffectState::Completed { stop_reason: None },
            nested_events: vec![],
            peer_usage: vec![],
        }
    }
}

struct FakeComposition;
#[async_trait]
impl CompositionPort for FakeComposition {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Created {
            child_instance_ref: request.program_ref,
        }
    }
    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Invoked {
            child_instance_ref: request.program_ref,
        }
    }
}

struct FulfilledEvents;
#[async_trait]
impl EventPort for FulfilledEvents {
    async fn await_event(&self, request: EventAwait) -> EventOutcome {
        EventOutcome::Fulfilled {
            payload: format!("event:{}", request.event_ref),
            event_ref: request.event_ref,
        }
    }
}

struct FakeCommit {
    version: Mutex<u64>,
    continuation: Mutex<Option<Value>>,
    evidence: Mutex<Vec<Fact>>,
}
impl FakeCommit {
    fn new() -> Self {
        Self {
            version: Mutex::new(0),
            continuation: Mutex::new(None),
            evidence: Mutex::new(Vec::new()),
        }
    }
}
#[async_trait]
impl ExecutionCommitPort for FakeCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut version = self.version.lock().unwrap();
        *version = request.expected_program_state_version + 1;
        *self.continuation.lock().unwrap() = request.tuple.continuation;
        self.evidence.lock().unwrap().extend(request.evidence_batch);
        ExecutionCommitResult::Committed {
            new_program_state_version: *version,
            evidence_position_ref: "evidence:conv".into(),
        }
    }
    async fn current_version(&self, _invocation_ref: &str) -> u64 {
        *self.version.lock().unwrap()
    }
    async fn load_continuation(&self, _invocation_ref: &str) -> Option<Value> {
        self.continuation.lock().unwrap().clone()
    }
}

fn ports(commit: Arc<FakeCommit>) -> ExecutionPorts {
    ExecutionPorts {
        model_inference: Arc::new(FakeModel),
        capability: Arc::new(FakeCapability),
        external_agent: Arc::new(FakeAcpPeer),
        events: Arc::new(FulfilledEvents),
        composition: Arc::new(FakeComposition),
        execution_commit: commit,
        hook_handlers: Arc::new(NoopStaticHookHandler),
    }
}

#[tokio::test]
async fn conversational_two_invokes_yield_and_resume_with_context() {
    let commit = Arc::new(FakeCommit::new());
    let turn_one_context = json!({"turn": 1, "messages": ["hello"]});

    let turn_one = execute_resumable(
        &ports(commit.clone()),
        conversational_request("instance.gao", "commit.turn1"),
        turn_one_context.clone(),
    )
    .await
    .expect("turn one drives to loop yield");

    match turn_one {
        RunOutcome::Suspended { continuation_id, .. } => {
            assert_eq!(continuation_id, "region.region.loop.turn.yield")
        }
        RunOutcome::Completed(_) => panic!("turn one must yield at the loop boundary"),
    }

    assert_eq!(
        *commit.version.lock().unwrap(),
        1,
        "the yielded context and continuation commit together before a crash can occur"
    );
    let parked: apxm_execution::Continuation = serde_json::from_value(
        commit
            .continuation
            .lock()
            .unwrap()
            .clone()
            .expect("continuation is part of the committed tuple"),
    )
    .expect("committed continuation decodes");
    assert_eq!(parked.context, turn_one_context);
    assert_eq!(parked.next_op_index, 0, "turn two restarts the loop body");

    let turn_two = resume(
        &ports(commit.clone()),
        "instance.gao",
        json!({"turn": 2, "resume": true}),
    )
    .await
    .expect("turn two resumes");

    match turn_two {
        RunOutcome::Completed(report) => {
            assert!(
                report.node_outcomes.len() >= 5,
                "turn two re-executes the authored semantic spine"
            );
            assert!(matches!(
                report.commit,
                ExecutionCommitResult::Committed { .. }
            ));
        }
        RunOutcome::Suspended { continuation_id, .. } => {
            panic!(
                "turn two should complete after one loop iteration, got suspend on {continuation_id}"
            )
        }
    }

    let facts = commit.evidence.lock().unwrap();
    let occurrences: std::collections::BTreeSet<_> = facts
        .iter()
        .filter(|fact| fact.fact_kind == FactKind::RegionOccurrenceStarted)
        .filter_map(|fact| fact.region_occurrence_id.as_deref())
        .collect();
    assert_eq!(occurrences.len(), 2, "each loop visit has a distinct dynamic occurrence");
    let capability = facts
        .iter()
        .find(|fact| {
            fact.fact_kind == FactKind::NodeExecutionRecorded
                && fact.air_node_id.as_deref() == Some("node.turn.tool")
        })
        .expect("capability execution has an authoritative join");
    assert!(
        capability.parent_node_execution_id.is_some(),
        "an authored capability nested under the model result retains its parent NodeExecution"
    );
    let specialist_new = facts
        .iter()
        .find(|fact| {
            fact.fact_kind == FactKind::NodeExecutionRecorded
                && fact.air_node_id.as_deref() == Some("node.specialist.new")
        })
        .expect("specialist creation has an authoritative NodeExecution");
    let specialist_invoke = facts
        .iter()
        .find(|fact| {
            fact.fact_kind == FactKind::NodeExecutionRecorded
                && fact.air_node_id.as_deref() == Some("node.specialist.invoke")
        })
        .expect("specialist invocation has an authoritative NodeExecution");
    assert_eq!(
        specialist_invoke.parent_node_execution_id.as_deref(),
        specialist_new.node_execution_id.as_deref(),
        "specialist invocation retains parent/child Program Instance lineage"
    );
}

#[tokio::test]
async fn public_authored_gao_uses_only_five_semantic_operations() {
    let air = load_gao_air();
    assert_eq!(air.semantic_operations.len(), 5);
    for op in &air.semantic_operations {
        match op.op {
            SemanticOpKind::ModelCall
            | SemanticOpKind::CapabilityInvoke
            | SemanticOpKind::ProgramNew
            | SemanticOpKind::ProgramInvoke
            | SemanticOpKind::AwaitEvent => {}
        }
    }
    assert!(air.structural_ir.iter().any(|node| node.kind == apxm_program::air::StructuralKind::Loop));
    assert!(air.structural_ir.iter().any(|node| node.kind == apxm_program::air::StructuralKind::Yield));
}
