//! Owner-local in-memory/filesystem ExecutionCommitPort conformance.
//!
//! Proves the mandatory local persistence bounds for the G3 no-build decision
//! on hosted durable checkpoint/output. Does not introduce a hosted service.

use apxm_commit_local::{FilesystemExecutionCommit, InMemoryExecutionCommit, MAX_COMMIT_RESULTS};
use apxm_kernel::{
    AtomicWriteSet, ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    ExecutionCommitTuple, ProgramInstanceRef, ProgramInvocationRef,
};
use serde_json::json;

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn request(
    commit_id: &str,
    instance: &str,
    expected: u64,
    continuation: Option<serde_json::Value>,
) -> ExecutionCommitRequest {
    ExecutionCommitRequest {
        commit_id: commit_id.into(),
        program_instance_ref: ProgramInstanceRef::new(instance),
        program_invocation_ref: ProgramInvocationRef::new("invoke.1"),
        idempotency_key: format!("idem.{commit_id}"),
        expected_program_state_version: expected,
        write_set: AtomicWriteSet {
            next_program_state_digest: digest('1'),
            continuation_digest: digest('2'),
            checkpoint_effect_outcomes_digest: digest('3'),
            runtime_evidence_batch_digest: digest('4'),
            usage_facts_digest: digest('5'),
            session_output_refs_digest: digest('6'),
        },
        tuple: ExecutionCommitTuple {
            context: json!({"k": "v"}),
            continuation,
            event_wait: None,
            effect_outcomes: vec![],
            evidence: vec![],
            usage: json!({"tokens": 1}),
            output_refs: vec![json!({"ref": "out.1"})],
        },
        evidence_batch: vec![],
    }
}

async fn assert_happy_path(port: &dyn ExecutionCommitPort) {
    let cont = json!({"pc": 7, "stack": ["a"]});
    let first = port
        .commit(request("commit.1", "instance.1", 0, Some(cont.clone())))
        .await;
    assert!(matches!(
        first,
        ExecutionCommitResult::Committed {
            new_program_state_version: 1,
            ..
        }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.1"))
            .await,
        1
    );
    assert_eq!(
        port.load_continuation(&ProgramInstanceRef::new("instance.1"))
            .await,
        Some(cont.clone())
    );

    // Idempotent replay — one advancer, no duplicate external effect surface.
    let replay = port
        .commit(request("commit.1", "instance.1", 0, Some(cont.clone())))
        .await;
    assert_eq!(replay, first);
    assert_eq!(
        port.load_continuation(&ProgramInstanceRef::new("instance.1"))
            .await
            .unwrap()["pc"],
        7
    );

    let conflict = port
        .commit(request("commit.2", "instance.1", 0, None))
        .await;
    assert!(matches!(
        conflict,
        ExecutionCommitResult::CompareConflict {
            current_program_state_version: 1
        }
    ));
}

#[tokio::test]
async fn replay_identity_is_scoped_and_conflicts_fail_closed() {
    let port = InMemoryExecutionCommit::new();
    let first = port
        .commit(request(
            "commit.scoped",
            "instance.1",
            0,
            Some(json!({"pc": 1})),
        ))
        .await;
    assert!(matches!(first, ExecutionCommitResult::Committed { .. }));

    let other_instance = port
        .commit(request(
            "commit.scoped",
            "instance.2",
            0,
            Some(json!({"pc": 2})),
        ))
        .await;
    assert!(matches!(
        other_instance,
        ExecutionCommitResult::Committed { .. }
    ));

    let mut other_invocation = request("commit.scoped", "instance.1", 1, Some(json!({"pc": 3})));
    other_invocation.program_invocation_ref = ProgramInvocationRef::new("invoke.2");
    let separate_invocation = port.commit(other_invocation).await;
    assert!(matches!(
        separate_invocation,
        ExecutionCommitResult::Committed { .. }
    ));

    let mut conflicting_request =
        request("commit.scoped", "instance.1", 0, Some(json!({"pc": 99})));
    conflicting_request.idempotency_key = "idem.conflict".into();
    let conflict = port.commit(conflicting_request).await;
    assert!(matches!(
        conflict,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.1"))
            .await,
        2
    );
}

#[tokio::test]
async fn exact_replay_requires_the_complete_request_identity() {
    let port = InMemoryExecutionCommit::new();
    let first_request = request(
        "commit.identity",
        "instance.identity",
        0,
        Some(json!({"pc": 1})),
    );
    let first = port.commit(first_request.clone()).await;
    assert!(matches!(first, ExecutionCommitResult::Committed { .. }));

    let mut changed_write_set = first_request;
    changed_write_set.write_set.next_program_state_digest = digest('9');
    let conflict = port.commit(changed_write_set).await;
    assert!(matches!(
        conflict,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.identity"))
            .await,
        1
    );
}

#[tokio::test]
async fn in_memory_owner_local_commit_resume_and_conflict() {
    let port = InMemoryExecutionCommit::new();
    assert_happy_path(&port).await;

    port.inject_outcome_unknown("commit.unknown");
    let unknown = port
        .commit(request("commit.unknown", "instance.2", 0, None))
        .await;
    assert!(matches!(
        unknown,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.2"))
            .await,
        0,
        "outcome_unknown must write none of the atomic set"
    );
}

#[tokio::test]
async fn filesystem_owner_local_survives_reopen_and_is_portable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = FilesystemExecutionCommit::open(dir.path()).expect("open");
    let mut first = request("commit.1", "instance.1", 0, Some(json!({"pc": 7})));
    first.tuple.event_wait = Some(json!({"event": "resume.1"}));
    assert!(matches!(
        port.commit(first).await,
        ExecutionCommitResult::Committed { .. }
    ));

    let root = port.root().to_path_buf();
    drop(port);

    // Restart / portability: reopen the same directory bytes.
    let reopened = FilesystemExecutionCommit::open(&root).expect("reopen");
    assert_eq!(
        reopened
            .current_version(&ProgramInstanceRef::new("instance.1"))
            .await,
        1
    );
    assert_eq!(
        reopened
            .load_continuation(&ProgramInstanceRef::new("instance.1"))
            .await
            .unwrap()["pc"],
        7
    );
    let persisted = std::fs::read_to_string(root.join("execution-commit-local.v2.json"))
        .expect("read persisted store");
    assert!(persisted.contains("\"event_wait\""));

    // Directory is the whole portable unit — copy path equals reopen path.
    let copy = tempfile::tempdir().expect("copy dir");
    for entry in std::fs::read_dir(&root).expect("read") {
        let entry = entry.expect("entry");
        std::fs::copy(entry.path(), copy.path().join(entry.file_name())).expect("copy");
    }
    let copied = FilesystemExecutionCommit::open(copy.path()).expect("open copy");
    assert_eq!(
        copied
            .current_version(&ProgramInstanceRef::new("instance.1"))
            .await,
        1
    );
}

#[test]
fn filesystem_directory_ownership_is_exclusive_and_recoverable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let owner = FilesystemExecutionCommit::open(dir.path()).expect("open owner");
    let contention = FilesystemExecutionCommit::open(dir.path());
    assert!(matches!(
        contention,
        Err(apxm_commit_local::CommitLocalError::OwnershipContended)
    ));

    drop(owner);
    FilesystemExecutionCommit::open(dir.path()).expect("ownership recovers after owner closes");
}

#[test]
fn filesystem_legacy_store_is_not_silently_reinitialized() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("execution-commit-local.v1.json"),
        r#"{"schema_version":"apxm.execution-commit-local.v1"}"#,
    )
    .expect("write legacy store marker");

    let error = match FilesystemExecutionCommit::open(dir.path()) {
        Ok(_) => panic!("legacy state must fail closed"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        apxm_commit_local::CommitLocalError::SchemaMismatch { .. }
    ));
}

#[tokio::test]
async fn filesystem_outcome_unknown_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = FilesystemExecutionCommit::open(dir.path()).expect("open");
    port.inject_outcome_unknown("commit.u").expect("inject");
    let unknown = port
        .commit(request("commit.u", "instance.x", 0, Some(json!({"pc": 1}))))
        .await;
    assert!(matches!(
        unknown,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.x"))
            .await,
        0
    );
    assert!(
        port.load_continuation(&ProgramInstanceRef::new("instance.x"))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn tuple_size_bound_fails_closed() {
    let port = InMemoryExecutionCommit::new();
    let huge = "x".repeat(9 * 1024 * 1024);
    let mut req = request("commit.huge", "instance.huge", 0, Some(json!(huge)));
    req.tuple.context = json!({"pad": "y".repeat(1024)});
    let result = port.commit(req).await;
    assert!(matches!(
        result,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.huge"))
            .await,
        0
    );
}

#[tokio::test]
async fn retention_overflow_is_atomic() {
    let port = InMemoryExecutionCommit::new();
    for version in 0..MAX_COMMIT_RESULTS {
        let id = format!("commit.retained.{version}");
        let result = port
            .commit(request(
                &id,
                "instance.retained",
                version as u64,
                Some(json!(version)),
            ))
            .await;
        assert!(matches!(result, ExecutionCommitResult::Committed { .. }));
    }

    let overflow = port
        .commit(request(
            "commit.retention-overflow",
            "instance.retained",
            MAX_COMMIT_RESULTS as u64,
            Some(json!({"must_not_persist": true})),
        ))
        .await;
    assert!(matches!(
        overflow,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.retained"))
            .await,
        MAX_COMMIT_RESULTS as u64
    );
    assert_eq!(
        port.load_continuation(&ProgramInstanceRef::new("instance.retained"))
            .await
            .unwrap(),
        json!(MAX_COMMIT_RESULTS - 1)
    );
}
