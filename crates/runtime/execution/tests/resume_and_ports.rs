//! Atomic structural continuation conformance for the canonical execution driver.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use apxm_execution::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionPorts, ExecutionRequest,
    EventRef, NoopStaticHookHandler, RunOutcome, execute_resumable, resume_event,
};
use apxm_inference::{
    AttemptDisposition, ExactPortBindingRef, ModelBindingAdmission, ModelCallRequest,
    ModelDeploymentRef, ModelInferencePort, ModelTargetRef, ResolvedModelBinding, Usage,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AtomicWriteSet, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExternalAgentCapabilityPort, PromptEffectState,
};
use apxm_program::air::AirModule;

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn request(scope: &str) -> ExecutionRequest {
    let air = serde_json::from_value::<AirModule>(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "node.model", "op": "model.call", "operands": {"model_target_ref": "model.default"}},
            {"node_id": "node.await", "op": "await.event", "operands": {"event_ref": "evt-atomic"}},
            {"node_id": "node.capability", "op": "capability.invoke", "operands": {"capability_ref": "cap.finish"}}
        ],
        "structural_ir": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("canonical AIR");
    ExecutionRequest {
        air,
        hook_bindings: Vec::new(),
        model_admission: ModelBindingAdmission::new(ResolvedModelBinding {
            model_target_ref: ModelTargetRef("model.default".into()),
            model_deployment_ref: ModelDeploymentRef("deployment.default".into()),
            exact_port_binding: ExactPortBindingRef {
                binding_digest: digest('a'),
            },
        }),
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
            child_instance_ref: request.program_ref,
        }
    }

    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Invoked {
            child_instance_ref: request.program_ref,
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
    ExecutionPorts {
        model_inference: Arc::new(Model),
        capability: Arc::new(Capability),
        external_agent: Arc::new(External),
        events: Arc::new(Events),
        composition: Arc::new(Composition),
        execution_commit: commit,
        hook_handlers: Arc::new(NoopStaticHookHandler),
    }
}

#[tokio::test]
async fn park_commits_context_continuation_wait_effects_evidence_usage_and_outputs_together() {
    let commit = Arc::new(Commit::default());
    let outcome = execute_resumable(
        &ports(commit.clone()),
        request("instance.atomic"),
        json!({"turn": 1}),
    )
    .await
    .expect("park commits atomically");

    assert!(matches!(
        outcome,
        RunOutcome::Suspended {
            continuation_id,
            event_ref: Some(event_ref),
        } if continuation_id == "node.await" && event_ref.as_str() == "evt-atomic"
    ));
    assert_eq!(*commit.version.lock().unwrap(), 1);
    let tuples = commit.tuples.lock().unwrap();
    assert_eq!(tuples.len(), 1);
    let tuple = &tuples[0];
    assert_eq!(tuple.context, json!({"turn": 1}));
    assert!(tuple.continuation.is_some());
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
        json!({"turn": 1}),
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
        json!({"turn": 1}),
    )
    .await
    .expect("park");

    let resumed = resume_event(
        &ports(commit.clone()),
        "instance.replay",
        EventRef::new("evt-atomic").expect("non-empty event ref"),
        json!({"turn": 2}),
    )
    .await
    .expect("resume from committed state");
    assert!(matches!(resumed, RunOutcome::Completed(_)));
    assert_eq!(*commit.version.lock().unwrap(), 2);
}
