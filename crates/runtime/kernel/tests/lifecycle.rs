//! Program Instance lifecycle vectors: atomic commit, conflict/outcome-unknown
//! publish nothing, crash/reconcile, replay, cancel, idempotency, single-flight
//! fail-busy, and multi-instance isolation. Driven by a deterministic in-crate
//! fixture commit port (a non-admissible test double implementing the port).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use apxm_program::runtime_evidence::{
    InvocationState, ProgramIdentity, RuntimeEvidence, RuntimeEvidenceVersion,
};
use apxm_program::runtime_evidence::Fact;
use apxm_program::artifact::SchemaDigestRef;

use apxm_kernel::{
    reconstruct, AtomicWriteSet, ExactPortBinding, ExecutionCommitPort, ExecutionCommitRequest,
    ExecutionCommitResult, Invocation, InvocationReport, InstanceError, PortBundle, PortBundleSpec,
    PortImplementation, PortSlot, ProgramInstance,
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

fn identity(id: &str) -> ProgramIdentity {
    ProgramIdentity {
        artifact_digest: digest('a'),
        entrypoint: "run".into(),
        agent_identity_binding: "agent.1".into(),
        program_instance_id: Some(id.into()),
    }
}

fn contract() -> SchemaDigestRef {
    SchemaDigestRef {
        schema_id: "apxm.execution-commit.v1".into(),
        digest: digest('e'),
    }
}

fn bundle(port: Arc<dyn ExecutionCommitPort>) -> PortBundle {
    let spec = PortBundleSpec::new(vec![(PortSlot::ExecutionCommit, contract())]);
    let binding = ExactPortBinding {
        slot: PortSlot::ExecutionCommit,
        port_contract: contract(),
        binding_digest: digest('b'),
        proof_digest: digest('c'),
    };
    PortBundle::construct(&spec, vec![(binding, PortImplementation::ExecutionCommit(port))])
        .expect("valid bundle")
}

fn instance_on(port: Arc<dyn ExecutionCommitPort>, id: &str) -> ProgramInstance {
    ProgramInstance::new(identity(id), id, bundle(port))
}

// ── Deterministic fixture commit port ──────────────────────────────────────

#[derive(Default)]
struct FixtureState {
    versions: HashMap<String, u64>,
    evidence: HashMap<String, Vec<Fact>>,
    by_id: HashMap<String, ExecutionCommitResult>,
    force_unknown: HashSet<String>,
}

struct FixtureCommit {
    state: Mutex<FixtureState>,
}

impl FixtureCommit {
    fn new() -> Self {
        Self {
            state: Mutex::new(FixtureState::default()),
        }
    }

    fn inject_outcome_unknown(&self, commit_id: &str) {
        self.state
            .lock()
            .unwrap()
            .force_unknown
            .insert(commit_id.to_string());
    }

    fn committed_evidence(&self, key: &str) -> Vec<Fact> {
        self.state
            .lock()
            .unwrap()
            .evidence
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    fn evidence_for(&self, id: &str) -> RuntimeEvidence {
        RuntimeEvidence {
            schema_version: RuntimeEvidenceVersion::V1,
            program_identity: identity(id),
            facts: self.committed_evidence(id),
        }
    }

    fn version(&self, key: &str) -> u64 {
        *self.state.lock().unwrap().versions.get(key).unwrap_or(&0)
    }
}

#[async_trait]
impl ExecutionCommitPort for FixtureCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut state = self.state.lock().unwrap();
        if let Some(prior) = state.by_id.get(&request.commit_id) {
            return prior.clone();
        }
        if state.force_unknown.contains(&request.commit_id) {
            return ExecutionCommitResult::OutcomeUnknown {
                reconciliation_ref: format!("reconcile:{}", request.commit_id),
            };
        }
        let current = *state.versions.get(&request.invocation_ref).unwrap_or(&0);
        if request.expected_program_state_version != current {
            return ExecutionCommitResult::CompareConflict {
                current_program_state_version: current,
            };
        }
        let new_version = current + 1;
        state
            .versions
            .insert(request.invocation_ref.clone(), new_version);
        state
            .evidence
            .entry(request.invocation_ref.clone())
            .or_default()
            .extend(request.evidence_batch.clone());
        let result = ExecutionCommitResult::Committed {
            new_program_state_version: new_version,
            evidence_position_ref: format!("evidence:{}:{new_version}", request.invocation_ref),
        };
        state
            .by_id
            .insert(request.commit_id.clone(), result.clone());
        result
    }

    async fn current_version(&self, invocation_ref: &str) -> u64 {
        self.version(invocation_ref)
    }
}

// ── Vectors ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn atomic_commit_publishes_full_write_set_and_all_facts() {
    let port = Arc::new(FixtureCommit::new());
    let instance = instance_on(port.clone(), "instance.1");

    let report = instance
        .invoke(Invocation {
            commit_id: "c1".into(),
            write_set: write_set(),
        })
        .await
        .expect("invocation runs");
    assert_eq!(report, InvocationReport::Committed { new_program_state_version: 1 });

    assert_eq!(port.version("instance.1"), 1);
    let evidence = port.evidence_for("instance.1");
    // instance.created + invocation.admitted + attempt.recorded + invocation.committed
    assert_eq!(evidence.facts.len(), 4);
    assert!(evidence.verify().is_accepted(), "durable evidence must be valid");

    let view = reconstruct(&evidence);
    assert!(view.committed);
    assert_eq!(view.invocation_state, Some(InvocationState::CommittedReturn));
}

#[tokio::test]
async fn compare_conflict_publishes_nothing() {
    // A stale expected version conflicts and the atomic commit writes nothing.
    let port = Arc::new(FixtureCommit::new());
    let request = ExecutionCommitRequest {
        commit_id: "c1".into(),
        invocation_ref: "instance.1".into(),
        idempotency_key: "idem.c1".into(),
        expected_program_state_version: 5,
        write_set: write_set(),
        evidence_batch: vec![],
    };
    let result = port.commit(request).await;
    assert_eq!(
        result,
        ExecutionCommitResult::CompareConflict { current_program_state_version: 0 }
    );
    assert_eq!(port.version("instance.1"), 0);
    assert!(port.committed_evidence("instance.1").is_empty());
}

#[tokio::test]
async fn outcome_unknown_publishes_nothing_and_is_not_success() {
    let port = Arc::new(FixtureCommit::new());
    port.inject_outcome_unknown("c1");
    let instance = instance_on(port.clone(), "instance.1");

    let report = instance
        .invoke(Invocation {
            commit_id: "c1".into(),
            write_set: write_set(),
        })
        .await
        .expect("invocation runs");
    assert!(matches!(report, InvocationReport::OutcomeUnknown { .. }));

    assert_eq!(port.version("instance.1"), 0);
    assert!(port.committed_evidence("instance.1").is_empty());
    let view = reconstruct(&port.evidence_for("instance.1"));
    assert!(!view.committed, "an uncertain outcome never becomes success");
    assert_eq!(view.invocation_state, None);
}

#[tokio::test]
async fn crash_after_commit_reconciles_to_committed() {
    let port = Arc::new(FixtureCommit::new());
    {
        let instance = instance_on(port.clone(), "instance.1");
        instance
            .invoke(Invocation { commit_id: "c1".into(), write_set: write_set() })
            .await
            .expect("commit");
        // Instance dropped: a crash loses the live instance, not the durable
        // evidence held by the atomic commit.
    }
    let view = reconstruct(&port.evidence_for("instance.1"));
    assert!(view.committed);
}

#[tokio::test]
async fn crash_before_commit_reconciles_to_uncommitted() {
    let port = Arc::new(FixtureCommit::new());
    port.inject_outcome_unknown("c1");
    let instance = instance_on(port.clone(), "instance.1");
    let _ = instance
        .invoke(Invocation { commit_id: "c1".into(), write_set: write_set() })
        .await;
    let view = reconstruct(&port.evidence_for("instance.1"));
    assert!(!view.committed);
    assert_eq!(view.last_event_sequence, 0);
}

#[tokio::test]
async fn idempotent_recommit_applies_once() {
    let port = Arc::new(FixtureCommit::new());
    let instance = instance_on(port.clone(), "instance.1");
    let first = instance
        .invoke(Invocation { commit_id: "c1".into(), write_set: write_set() })
        .await
        .unwrap();
    let second = instance
        .invoke(Invocation { commit_id: "c1".into(), write_set: write_set() })
        .await
        .unwrap();
    assert_eq!(first, InvocationReport::Committed { new_program_state_version: 1 });
    assert_eq!(second, InvocationReport::Committed { new_program_state_version: 1 });
    // The atomic boundary applied exactly once: version and durable evidence
    // reflect a single commit.
    assert_eq!(port.version("instance.1"), 1);
    assert_eq!(port.committed_evidence("instance.1").len(), 4);
}

#[tokio::test]
async fn replay_from_durable_evidence_is_monotonic() {
    let port = Arc::new(FixtureCommit::new());
    let instance = instance_on(port.clone(), "instance.1");
    instance
        .invoke(Invocation { commit_id: "c1".into(), write_set: write_set() })
        .await
        .unwrap();
    let evidence = port.evidence_for("instance.1");
    assert!(evidence.verify().is_accepted(), "evidence is strictly monotonic");

    let prefix = RuntimeEvidence {
        schema_version: RuntimeEvidenceVersion::V1,
        program_identity: identity("instance.1"),
        facts: evidence.facts[..2].to_vec(),
    };
    let replayed = reconstruct(&prefix);
    let live = reconstruct(&evidence);
    assert!(!replayed.committed, "prefix before the commit fact is uncommitted");
    assert!(live.committed);
    assert!(live.last_event_sequence >= replayed.last_event_sequence);
}

#[tokio::test]
async fn cancel_commits_cancellation() {
    let port = Arc::new(FixtureCommit::new());
    let instance = instance_on(port.clone(), "instance.1");
    let report = instance.cancel("cx", write_set()).await.expect("cancel");
    assert_eq!(report, InvocationReport::Cancelled);
    let view = reconstruct(&port.evidence_for("instance.1"));
    assert_eq!(view.invocation_state, Some(InvocationState::Cancelled));
}

#[tokio::test]
async fn instances_are_isolated() {
    let port_a = Arc::new(FixtureCommit::new());
    let port_b = Arc::new(FixtureCommit::new());
    let a = instance_on(port_a.clone(), "instance.a");
    let b = instance_on(port_b.clone(), "instance.b");

    a.invoke(Invocation { commit_id: "c1".into(), write_set: write_set() })
        .await
        .unwrap();

    // B shares no state with A.
    assert_eq!(port_b.version("instance.b"), 0);
    assert!(port_b.committed_evidence("instance.b").is_empty());

    b.invoke(Invocation { commit_id: "c1".into(), write_set: write_set() })
        .await
        .unwrap();
    assert_eq!(port_a.version("instance.a"), 1);
    assert_eq!(port_b.version("instance.b"), 1);
}

// ── Single-flight fail-busy ────────────────────────────────────────────────

struct GatedCommit {
    released: Arc<AtomicBool>,
    inner: Arc<FixtureCommit>,
}

#[async_trait]
impl ExecutionCommitPort for GatedCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        while !self.released.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        self.inner.commit(request).await
    }

    async fn current_version(&self, invocation_ref: &str) -> u64 {
        self.inner.current_version(invocation_ref).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_flight_rejects_concurrent_invocation() {
    let released = Arc::new(AtomicBool::new(false));
    let inner = Arc::new(FixtureCommit::new());
    let port: Arc<dyn ExecutionCommitPort> = Arc::new(GatedCommit {
        released: released.clone(),
        inner: inner.clone(),
    });
    let instance = Arc::new(instance_on(port, "instance.1"));

    let driver = instance.clone();
    let handle = tokio::spawn(async move {
        driver
            .invoke(Invocation { commit_id: "c1".into(), write_set: write_set() })
            .await
    });

    // Wait until the first invocation holds the single-flight guard.
    while !instance.is_busy() {
        tokio::task::yield_now().await;
    }

    let busy = instance
        .invoke(Invocation { commit_id: "c2".into(), write_set: write_set() })
        .await;
    assert_eq!(busy, Err(InstanceError::Busy));

    released.store(true, Ordering::SeqCst);
    let first = handle.await.expect("join").expect("first invocation");
    assert_eq!(first, InvocationReport::Committed { new_program_state_version: 1 });
    assert_eq!(inner.version("instance.1"), 1);
}
