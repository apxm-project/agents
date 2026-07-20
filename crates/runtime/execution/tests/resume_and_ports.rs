//! Conformance for the session-family canonical ports: durable park/resume over
//! the driver, the pure session ledger, and scoped memory. Deterministic
//! in-crate fakes stand in for the durable implementations (test doubles,
//! non-admissible) — they are the real consumers that make these ports concrete
//! rather than speculative.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use apxm_inference::{
    AttemptDisposition, ExactPortBindingRef, ModelBindingAdmission, ModelCallRequest,
    ModelDeploymentRef, ModelInferencePort, ModelTargetRef, ResolvedModelBinding, Usage,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AtomicWriteSet, ExecutionCommitPort, ExecutionCommitRequest,
    ExecutionCommitResult, ExternalAgentCapabilityPort, PromptEffectState,
};
use apxm_program::air::AirModule;
use apxm_program::runtime_evidence::Fact;

use apxm_execution::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, Continuation, ContinuationError, ContinuationPort, EventAwait, EventOutcome,
    EventPort, ExecutionPorts, ExecutionRequest, LedgerState, MemoryError, MemorySpace, NoResume,
    RunOutcome, ScopedMemoryPort, SessionLedger, SessionLedgerError, SessionLedgerStore,
    WakeOutcome, execute_resumable, resume,
};

const WAIT_KEY: &str = "session-input:s1";

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

/// A three-op session turn: answer with the model, park for the next user turn,
/// then run one capability after the turn is delivered.
fn session_air() -> AirModule {
    serde_json::from_value(json!({
        "schema_version": "apxm.air.v1",
        "semantic_operations": [
            {"node_id": "n.model", "op": "model.call", "operands": {"model_target_ref": "model.default"}},
            {"node_id": "n.await", "op": "await.event", "operands": {"event_selector": WAIT_KEY}},
            {"node_id": "n.cap", "op": "capability.invoke", "operands": {"capability_ref": "cap.finish"}}
        ],
        "structural_ir": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    }))
    .expect("valid session AIR")
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

fn request(version_scope: &str, commit_id: &str) -> ExecutionRequest {
    ExecutionRequest {
        air: session_air(),
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
            input_tokens: 7,
            output_tokens: 11,
        })
    }
}

struct FakeCapability;
#[async_trait]
impl CapabilityPort for FakeCapability {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::Completed {
            result: format!("cap:{}", request.capability_ref),
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

/// An event port whose `await.event` parks or fulfills based on configuration.
struct ConfigurableEvents {
    park: bool,
}
#[async_trait]
impl EventPort for ConfigurableEvents {
    async fn await_event(&self, request: EventAwait) -> EventOutcome {
        if self.park {
            EventOutcome::Parked
        } else {
            EventOutcome::Fulfilled {
                payload: format!("event:{}", request.selector),
            }
        }
    }
}

struct FakeCommit {
    version: Mutex<u64>,
    committed_facts: Mutex<Vec<Fact>>,
}
impl FakeCommit {
    fn new() -> Self {
        Self {
            version: Mutex::new(0),
            committed_facts: Mutex::new(Vec::new()),
        }
    }
}
#[async_trait]
impl ExecutionCommitPort for FakeCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut version = self.version.lock().unwrap();
        *version = request.expected_program_state_version + 1;
        self.committed_facts
            .lock()
            .unwrap()
            .extend(request.evidence_batch);
        ExecutionCommitResult::Committed {
            new_program_state_version: *version,
            evidence_position_ref: "evidence:1".into(),
        }
    }
    async fn current_version(&self, _invocation_ref: &str) -> u64 {
        *self.version.lock().unwrap()
    }
}

fn ports(park: bool, commit: Arc<FakeCommit>) -> ExecutionPorts {
    ExecutionPorts {
        model_inference: Arc::new(FakeModel),
        capability: Arc::new(FakeCapability),
        external_agent: Arc::new(FakeAcpPeer),
        events: Arc::new(ConfigurableEvents { park }),
        composition: Arc::new(FakeComposition),
        execution_commit: commit,
    }
}

/// In-memory durable-continuation double: parked continuations by wait key plus a
/// FIFO of delivered values, so wake-before-register is observable.
#[derive(Default)]
struct InMemoryContinuations {
    parked: Mutex<HashMap<String, Continuation>>,
    pending: Mutex<HashMap<String, VecDeque<Value>>>,
}
#[async_trait]
impl ContinuationPort for InMemoryContinuations {
    async fn persist(&self, continuation: Continuation) -> Result<(), ContinuationError> {
        self.parked
            .lock()
            .unwrap()
            .insert(continuation.wait_key.clone(), continuation);
        Ok(())
    }
    async fn take(&self, wait_key: &str) -> Result<Option<Continuation>, ContinuationError> {
        Ok(self.parked.lock().unwrap().remove(wait_key))
    }
    async fn wake(&self, wait_key: &str, value: Value) -> Result<WakeOutcome, ContinuationError> {
        let woken = self.parked.lock().unwrap().contains_key(wait_key);
        self.pending
            .lock()
            .unwrap()
            .entry(wait_key.to_string())
            .or_default()
            .push_back(value);
        Ok(if woken {
            WakeOutcome::Woken
        } else {
            WakeOutcome::Queued
        })
    }
    async fn take_pending(&self, wait_key: &str) -> Result<Option<Value>, ContinuationError> {
        Ok(self
            .pending
            .lock()
            .unwrap()
            .get_mut(wait_key)
            .and_then(VecDeque::pop_front))
    }
}

// ── Resumable execution: park then resume ───────────────────────────────────

#[tokio::test]
async fn resumable_run_without_park_completes_and_commits() {
    let commit = Arc::new(FakeCommit::new());
    let continuations = InMemoryContinuations::default();
    let outcome = execute_resumable(
        &ports(false, commit.clone()),
        &continuations,
        request("inv.1", "c1"),
        Value::Null,
        &[],
    )
    .await
    .expect("run drives to completion");

    match outcome {
        RunOutcome::Completed(report) => {
            assert_eq!(report.node_outcomes.len(), 3, "all three ops ran");
            assert_eq!(report.native_usage.input_tokens, 7);
            assert!(matches!(
                report.commit,
                ExecutionCommitResult::Committed { .. }
            ));
        }
        RunOutcome::Suspended { .. } => panic!("a fulfilled await must not suspend"),
    }
    assert!(continuations.parked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resumable_run_suspends_at_park_and_persists_continuation() {
    let commit = Arc::new(FakeCommit::new());
    let continuations = InMemoryContinuations::default();
    let outcome = execute_resumable(
        &ports(true, commit.clone()),
        &continuations,
        request("inv.2", "c2"),
        Value::Null,
        &[],
    )
    .await
    .expect("run parks");

    match outcome {
        RunOutcome::Suspended { wait_key } => assert_eq!(wait_key, WAIT_KEY),
        RunOutcome::Completed(_) => panic!("a parked await must suspend"),
    }
    // Nothing committed on park; the continuation carries the pre-park model
    // usage and points at the op after the await.
    assert!(commit.committed_facts.lock().unwrap().is_empty());
    let parked = continuations.parked.lock().unwrap();
    let cont = parked.get(WAIT_KEY).expect("continuation persisted");
    assert_eq!(cont.next_op_index, 2);
    assert_eq!(cont.native_usage.input_tokens, 7);
}

#[tokio::test]
async fn continuation_serializes_for_durable_storage() {
    let commit = Arc::new(FakeCommit::new());
    let continuations = InMemoryContinuations::default();
    execute_resumable(
        &ports(true, commit),
        &continuations,
        request("inv.serde", "c.serde"),
        Value::Null,
        &[],
    )
    .await
    .expect("parks");

    let cont = continuations
        .parked
        .lock()
        .unwrap()
        .get(WAIT_KEY)
        .cloned()
        .expect("continuation persisted");
    // A durable ContinuationPort round-trips the continuation through serde; the
    // resume-critical fields must survive intact.
    let encoded = serde_json::to_string(&cont).expect("continuation serializes");
    let decoded: Continuation = serde_json::from_str(&encoded).expect("continuation deserializes");
    assert_eq!(decoded.wait_key, WAIT_KEY);
    assert_eq!(decoded.next_op_index, cont.next_op_index);
    assert_eq!(decoded.native_usage, cont.native_usage);
    assert_eq!(decoded.write_set, cont.write_set);
}

#[tokio::test]
async fn wake_then_resume_finishes_the_parked_run() {
    let commit = Arc::new(FakeCommit::new());
    let continuations = InMemoryContinuations::default();

    // Turn 1 parks.
    let suspended = execute_resumable(
        &ports(true, commit.clone()),
        &continuations,
        request("inv.3", "c3"),
        Value::Null,
        &[],
    )
    .await
    .expect("parks");
    assert!(matches!(suspended, RunOutcome::Suspended { .. }));

    // Delivering input to a parked wait key wakes it.
    let wake = continuations
        .wake(WAIT_KEY, json!({"message": "next turn"}))
        .await
        .expect("wake");
    assert_eq!(wake, WakeOutcome::Woken);
    let delivered = continuations
        .take_pending(WAIT_KEY)
        .await
        .expect("pending")
        .expect("a queued value");

    // Resume continues from the op after the await and commits.
    let completed = resume(
        &ports(true, commit.clone()),
        &continuations,
        WAIT_KEY,
        delivered,
        &[],
    )
    .await
    .expect("resumes");
    match completed {
        RunOutcome::Completed(report) => {
            // The delivered await plus the post-await capability node.
            assert_eq!(report.node_outcomes.len(), 2);
            assert!(matches!(
                report.commit,
                ExecutionCommitResult::Committed { .. }
            ));
        }
        RunOutcome::Suspended { .. } => panic!("only one park in this AIR"),
    }
    assert!(
        continuations.parked.lock().unwrap().is_empty(),
        "resume takes the continuation"
    );
}

#[tokio::test]
async fn resume_without_a_parked_continuation_fails_closed() {
    let commit = Arc::new(FakeCommit::new());
    let continuations = InMemoryContinuations::default();
    let err = resume(
        &ports(true, commit),
        &continuations,
        "no-such-key",
        Value::Null,
        &[],
    )
    .await
    .expect_err("no continuation is parked");
    assert!(matches!(
        err,
        apxm_execution::ExecutionError::Continuation(ContinuationError::NotParked { .. })
    ));
}

#[tokio::test]
async fn wake_before_park_is_queued_not_lost() {
    let continuations = InMemoryContinuations::default();
    // No continuation parked yet: the delivery is queued, not woken.
    let outcome = continuations
        .wake(WAIT_KEY, json!("early"))
        .await
        .expect("wake");
    assert_eq!(outcome, WakeOutcome::Queued);
    // The value survives until a waiter consumes it.
    let pending = continuations
        .take_pending(WAIT_KEY)
        .await
        .expect("pending")
        .expect("queued value retained");
    assert_eq!(pending, json!("early"));
}

#[tokio::test]
async fn no_resume_port_supports_a_run_that_never_parks() {
    let commit = Arc::new(FakeCommit::new());
    let outcome = execute_resumable(
        &ports(false, commit),
        &NoResume,
        request("inv.4", "c4"),
        Value::Null,
        &[],
    )
    .await
    .expect("a non-parking run needs no durable store");
    assert!(matches!(outcome, RunOutcome::Completed(_)));
}

#[tokio::test]
async fn no_resume_port_fails_closed_when_a_run_parks() {
    let commit = Arc::new(FakeCommit::new());
    let err = execute_resumable(
        &ports(true, commit),
        &NoResume,
        request("inv.5", "c5"),
        Value::Null,
        &[],
    )
    .await
    .expect_err("NoResume cannot persist a continuation");
    assert!(matches!(
        err,
        apxm_execution::ExecutionError::Continuation(ContinuationError::ResumeUnsupported)
    ));
}

// ── Pure session ledger ─────────────────────────────────────────────────────

#[test]
fn session_ledger_enforces_the_turn_cap_fail_closed() {
    let ledger = SessionLedger::new(Some(2), HashMap::new());
    assert_eq!(ledger.charge_turn().expect("turn 1"), 1);
    assert_eq!(ledger.charge_turn().expect("turn 2"), 2);
    assert!(ledger.would_exceed_turn_cap());
    let err = ledger.charge_turn().expect_err("cap reached");
    assert_eq!(err, SessionLedgerError::TurnCapReached { cap: 2 });
    assert_eq!(ledger.turns_used(), 2, "a refused turn does not advance");
    assert_eq!(ledger.turns_remaining(), Some(0));
}

#[test]
fn session_ledger_enforces_per_tool_budget_fail_closed() {
    let ledger = SessionLedger::new(None, HashMap::from([("search".to_string(), 1)]));
    assert_eq!(ledger.charge_tool("search").expect("first search"), 1);
    let err = ledger.charge_tool("search").expect_err("budget exhausted");
    assert_eq!(
        err,
        SessionLedgerError::ToolBudgetExhausted {
            tool: "search".to_string(),
            budget: 1,
        }
    );
    assert_eq!(ledger.tool_budgets_remaining().get("search").copied(), Some(0));
    // An unbudgeted tool is unbounded.
    assert_eq!(ledger.charge_tool("read").expect("unbudgeted"), 1);
}

/// In-memory durable ledger double.
#[derive(Default)]
struct InMemoryLedgerStore {
    rows: Mutex<HashMap<String, LedgerState>>,
}
impl SessionLedgerStore for InMemoryLedgerStore {
    fn load(&self, session_id: &str) -> Result<Option<LedgerState>, SessionLedgerError> {
        Ok(self.rows.lock().unwrap().get(session_id).cloned())
    }
    fn persist(&self, session_id: &str, state: &LedgerState) -> Result<(), SessionLedgerError> {
        self.rows
            .lock()
            .unwrap()
            .insert(session_id.to_string(), state.clone());
        Ok(())
    }
}

#[test]
fn session_ledger_snapshot_round_trips_through_a_store() {
    let store = InMemoryLedgerStore::default();
    let ledger = SessionLedger::new(Some(5), HashMap::new());
    ledger.charge_turn().expect("turn");
    ledger.charge_turn().expect("turn");
    store.persist("s1", &ledger.snapshot()).expect("persist");

    let loaded = store.load("s1").expect("load").expect("row exists");
    let rehydrated = SessionLedger::with_state(Some(5), HashMap::new(), loaded);
    assert_eq!(rehydrated.turns_used(), 2, "counters survive a store round-trip");
    assert_eq!(rehydrated.charge_turn().expect("turn 3"), 3);
}

// ── Scoped memory ───────────────────────────────────────────────────────────

#[derive(Default)]
struct InMemoryScopedMemory {
    store: Mutex<HashMap<(String, String, String), Value>>,
}
impl InMemoryScopedMemory {
    fn key(space: MemorySpace, scope: &str, key: &str) -> (String, String, String) {
        let space = match space {
            MemorySpace::Stm => "stm",
            MemorySpace::Ltm => "ltm",
        };
        (space.to_string(), scope.to_string(), key.to_string())
    }
}
#[async_trait]
impl ScopedMemoryPort for InMemoryScopedMemory {
    async fn read_scoped(
        &self,
        space: MemorySpace,
        scope: &str,
        key: &str,
    ) -> Result<Option<Value>, MemoryError> {
        Ok(self
            .store
            .lock()
            .unwrap()
            .get(&Self::key(space, scope, key))
            .cloned())
    }
    async fn write_scoped(
        &self,
        space: MemorySpace,
        scope: &str,
        key: &str,
        value: Value,
    ) -> Result<(), MemoryError> {
        self.store
            .lock()
            .unwrap()
            .insert(Self::key(space, scope, key), value);
        Ok(())
    }
    async fn delete_scoped(
        &self,
        space: MemorySpace,
        scope: &str,
        key: &str,
    ) -> Result<(), MemoryError> {
        self.store.lock().unwrap().remove(&Self::key(space, scope, key));
        Ok(())
    }
}

#[tokio::test]
async fn scoped_memory_reads_writes_and_deletes_by_scope() {
    let mem = InMemoryScopedMemory::default();
    assert_eq!(
        mem.read_scoped(MemorySpace::Stm, "s1", "conversation:user:1")
            .await
            .expect("read"),
        None
    );
    mem.write_scoped(MemorySpace::Stm, "s1", "conversation:user:1", json!("hello"))
        .await
        .expect("write");
    // Scope isolates: the same key under a different scope is absent.
    assert_eq!(
        mem.read_scoped(MemorySpace::Stm, "s2", "conversation:user:1")
            .await
            .expect("read"),
        None
    );
    assert_eq!(
        mem.read_scoped(MemorySpace::Stm, "s1", "conversation:user:1")
            .await
            .expect("read"),
        Some(json!("hello"))
    );
    mem.delete_scoped(MemorySpace::Stm, "s1", "conversation:user:1")
        .await
        .expect("delete");
    assert_eq!(
        mem.read_scoped(MemorySpace::Stm, "s1", "conversation:user:1")
            .await
            .expect("read"),
        None
    );
}
