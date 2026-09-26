//! Owner-local in-memory/filesystem ExecutionCommitPort conformance.
//!
//! Proves the mandatory local persistence bounds for the G3 no-build decision
//! on hosted durable checkpoint/output. Does not introduce a hosted service.

use apxm_commit_local::{
    AllowReadAccess, CommitLocalStore, CommitRequestIdentity, DenyReadAccess,
    FilesystemExecutionCommit, InMemoryExecutionCommit, MAX_COMMIT_RESULTS, MAX_READ_RECORDS,
    MAX_STORE_BYTES, ReadAccessHook, ReadAudit, ReadAuthorization, ReadTarget,
    SessionOutputPreparation,
};
use apxm_kernel::{
    AtomicWriteSet, ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    ExecutionCommitTuple, ProgramInstanceRef, ProgramInvocationRef, continuation_digest,
};
use apxm_program::runtime_evidence::{
    Fact, FactKind, LoopIterationCompletedFact, ModelAttemptRecordedFact,
    NodeExecutionRecordedFact, NodeExecutionScope, RuntimeFact,
};
use apxm_program::{common::ErrorCategory, common::TypedErrorEnvelope};
use apxm_runtime_protocol::{
    ContentRef, ExecutionReadRequest, ExecutionReadResult, GrantRef, PrincipalRef, ReadContext,
    ReadPurpose, RequestId, ScopeRef,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn request(
    commit_id: &str,
    instance: &str,
    expected: u64,
    continuation: Option<serde_json::Value>,
) -> ExecutionCommitRequest {
    let continuation_digest = continuation_digest(continuation.as_ref());
    ExecutionCommitRequest {
        commit_id: commit_id.into(),
        program_instance_ref: ProgramInstanceRef::new(instance),
        program_invocation_ref: ProgramInvocationRef::new("invoke.1"),
        idempotency_key: format!("idem.{commit_id}"),
        expected_program_state_version: expected,
        write_set: AtomicWriteSet {
            next_program_state_digest: digest('1'),
            continuation_digest,
            checkpoint_effect_outcomes_digest: digest('3'),
            runtime_evidence_batch_digest: apxm_kernel::runtime_evidence_and_observation_digest(
                &[],
                &[],
            ),
            usage_facts_digest: digest('5'),
            session_output_refs_digest: apxm_kernel::session_output_refs_digest(&[
                json!({"ref": "out.1"}),
            ]),
        },
        tuple: ExecutionCommitTuple {
            context: json!({"k": "v"}),
            continuation,
            event_wait: None,
            effect_outcomes: vec![],
            evidence: vec![],
            usage: json!({"tokens": 1}),
            output_refs: vec![json!({"ref": "out.1"})],
            observations: vec![],
        },
        evidence_batch: vec![],
    }
}

fn bind_observation_digest(request: &mut ExecutionCommitRequest) {
    request.write_set.runtime_evidence_batch_digest =
        apxm_kernel::runtime_evidence_and_observation_digest(
            &request.tuple.evidence,
            &request.tuple.observations,
        );
    request.write_set.session_output_refs_digest =
        apxm_kernel::session_output_refs_digest(&request.tuple.output_refs);
}

fn observation(
    kind: &str,
    commitment: &str,
    sequence: u64,
    content_ref: Option<&str>,
    output_ref: Option<&str>,
    event_ref: Option<&str>,
) -> Value {
    let mut value = json!({
        "contract": "apxm.execution-observation.v1",
        "observation_id": format!("observation.invoke.1.{sequence}"),
        "program_invocation_id": "invoke.1",
        "sequence": sequence,
        "cursor": {
            "position": sequence,
            "token": format!("cursor.{sequence}")
        },
        "timing": {"observed_at_unix_ms": sequence},
        "observation_kind": kind,
        "commitment": commitment
    });
    if let Some(content_ref) = content_ref {
        value["content_ref"] = json!(content_ref);
    }
    if let Some(output_ref) = output_ref {
        value["output_ref"] = json!(output_ref);
    }
    if let Some(event_ref) = event_ref {
        value["event_ref"] = json!({
            "event_ref": event_ref,
            "generation": 1
        });
    }
    value
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
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    let first = port
        .commit(request(
            "commit.scoped",
            "instance.1",
            0,
            Some(json!({"pc": 1})),
        ))
        .await;
    assert!(matches!(first, ExecutionCommitResult::Committed { .. }));

    let mut other_instance_request =
        request("commit.scoped", "instance.2", 0, Some(json!({"pc": 2})));
    other_instance_request.program_invocation_ref = ProgramInvocationRef::new("invoke.2");
    let other_instance = port.commit(other_instance_request).await;
    assert!(matches!(
        other_instance,
        ExecutionCommitResult::Committed { .. }
    ));

    let mut other_invocation = request("commit.scoped", "instance.1", 1, Some(json!({"pc": 3})));
    other_invocation.program_invocation_ref = ProgramInvocationRef::new("invoke.3");
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
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
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
async fn compact_replay_identity_reopens_and_old_reader_refuses_new_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = FilesystemExecutionCommit::open(dir.path()).expect("open");
    let first_request = request(
        "commit.compact-replay",
        "instance.compact-replay",
        0,
        Some(json!({"state": "x".repeat(32 * 1024)})),
    );
    let full_identity = CommitRequestIdentity::from(&first_request);
    let first = port.commit(first_request.clone()).await;
    assert!(matches!(first, ExecutionCommitResult::Committed { .. }));
    drop(port);

    let persisted = std::fs::read(dir.path().join("execution-commit-local.v2.json"))
        .expect("read persisted compact store");
    let store: Value = serde_json::from_slice(&persisted).expect("store JSON");
    let identity = store["by_commit_scope"]
        .as_object()
        .and_then(|scopes| scopes.values().next())
        .and_then(|row| row.get("request_identity"))
        .expect("retained replay identity");
    let fingerprint = identity.as_str().expect("compact fingerprint");
    assert!(fingerprint.starts_with("apxm.commit-request-identity.v1.sha256:"));
    assert!(
        serde_json::from_value::<CommitRequestIdentity>(identity.clone()).is_err(),
        "older full-identity readers must refuse a fingerprint row"
    );
    assert!(
        serde_json::to_vec(identity).unwrap().len()
            < serde_json::to_vec(&full_identity).unwrap().len() / 100,
        "retained replay identity should not duplicate the large continuation"
    );

    let reopened = FilesystemExecutionCommit::open(dir.path()).expect("reopen compact store");
    assert_eq!(reopened.commit(first_request.clone()).await, first);
    let mut changed_context = first_request;
    changed_context.tuple.context = json!({"k": "different"});
    assert!(matches!(
        reopened.commit(changed_context).await,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        reopened
            .current_version(&ProgramInstanceRef::new("instance.compact-replay"))
            .await,
        1
    );
}

fn output_preparation(
    commit_id: &str,
    instance: &str,
    invocation: &str,
) -> SessionOutputPreparation {
    SessionOutputPreparation {
        commit_id: commit_id.into(),
        program_instance_ref: ProgramInstanceRef::new(instance),
        program_invocation_ref: ProgramInvocationRef::new(invocation),
        content: b"durable-session-output".to_vec(),
        media_type: "text/plain".into(),
        node_execution_id: None,
        occurrence_id: None,
        access_scope_ref: if instance == "instance.scope-target" {
            "scope.allowed".into()
        } else {
            instance.into()
        },
        disclosure_ref: None,
    }
}

#[tokio::test]
async fn prepared_output_becomes_visible_only_with_atomic_commit_and_replays() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    let prepared = port
        .prepare_output(output_preparation(
            "commit.output",
            "instance.output",
            "invoke.output",
        ))
        .expect("prepare output");
    assert!(matches!(
        port.read_committed_output(
            read_context(ReadPurpose::Content, "instance.output"),
            ContentRef::new(prepared.output_ref.clone()).expect("content ref"),
        ),
        Err(apxm_commit_local::CommitLocalError::ProvisionalOutput)
    ));

    let mut commit = request("commit.output", "instance.output", 0, None);
    commit.program_invocation_ref = ProgramInvocationRef::new("invoke.output");
    commit.tuple.output_refs = vec![prepared.to_json()];
    bind_observation_digest(&mut commit);
    let first = port.commit(commit.clone()).await;
    assert!(matches!(first, ExecutionCommitResult::Committed { .. }));
    assert_eq!(
        port.read_committed_output(
            read_context(ReadPurpose::Content, "instance.output"),
            ContentRef::new(prepared.output_ref.clone()).expect("content ref"),
        )
        .expect("committed output")
        .bytes,
        b"durable-session-output".to_vec()
    );
    assert_eq!(
        port.read_output(
            read_context(ReadPurpose::Output, "instance.output"),
            apxm_runtime_protocol::OutputRef::new(prepared.output_ref.clone()).expect("output ref"),
        )
        .expect("typed output read")
        .output_ref
        .expect("output binding")
        .reference
        .as_str(),
        prepared.output_ref
    );

    // Replaying the exact commit returns the stored result and never creates a
    // second visible output.
    assert_eq!(port.commit(commit).await, first);
    assert_eq!(
        port.prepare_output(output_preparation(
            "commit.output",
            "instance.output",
            "invoke.output",
        ))
        .expect("replay output preparation"),
        prepared
    );
}

#[tokio::test]
async fn observation_references_must_bind_to_the_atomic_commit_tuple() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));

    let mut content_request = request("commit.observation-content", "instance.obs", 0, None);
    content_request.tuple.observations.push(observation(
        "content_published",
        "provisional",
        1,
        Some("content.not-in-tuple"),
        None,
        None,
    ));
    bind_observation_digest(&mut content_request);
    assert!(matches!(
        port.commit(content_request).await,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.obs"))
            .await,
        0
    );

    let mut event_request = request("commit.observation-event", "instance.event", 0, None);
    event_request.tuple.event_wait = Some(json!({
        "continuation_id": "continuation.1",
        "event_ref": "event.expected"
    }));
    event_request.tuple.observations.push(observation(
        "event_waiting",
        "provisional",
        1,
        None,
        None,
        Some("event.forged"),
    ));
    bind_observation_digest(&mut event_request);
    assert!(matches!(
        port.commit(event_request).await,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.event"))
            .await,
        0
    );
}

#[test]
fn observation_content_coordinates_cannot_cross_occurrences_or_mutate_the_store() {
    let mut store = CommitLocalStore::new();
    let mut preparation =
        output_preparation("commit.observation-occurrence", "instance.obs", "invoke.1");
    preparation.node_execution_id =
        Some(apxm_runtime_protocol::NodeExecutionId::new("node.first").expect("node"));
    preparation.occurrence_id = Some(
        apxm_runtime_protocol::execution_contracts::OccurrenceId::new("occurrence.first")
            .expect("occurrence"),
    );
    let prepared = store.prepare_output(preparation).expect("prepare output");

    let mut request = request("commit.observation-occurrence", "instance.obs", 0, None);
    request.tuple.output_refs = vec![prepared.to_json()];
    let mut observation = observation(
        "content_committed",
        "committed",
        1,
        Some(&prepared.output_ref),
        Some(&prepared.output_ref),
        None,
    );
    observation["node_execution_id"] = json!("node.second");
    observation["occurrence_id"] = json!("occurrence.second");
    request.tuple.observations = vec![observation];
    bind_observation_digest(&mut request);

    let before = store.clone();
    assert!(matches!(
        store.commit(&request),
        Err(apxm_commit_local::CommitLocalError::InvalidRequest(message))
            if message.contains("output coordinates")
    ));
    assert_eq!(
        store, before,
        "coordinate rejection must publish no durable mutation"
    );
    assert_eq!(
        store.current_version(&ProgramInstanceRef::new("instance.obs")),
        0
    );
}

#[test]
fn observation_event_ref_cannot_cross_commits_or_mutate_the_store() {
    let mut store = CommitLocalStore::new();
    let mut first = request("commit.observation-event-first", "instance.event", 0, None);
    first.tuple.event_wait = Some(json!({
        "continuation_id": "continuation.first",
        "event_ref": "event.first",
        "generation": 7,
        "occurrence_id": "occurrence.first"
    }));
    bind_observation_digest(&mut first);
    assert!(matches!(
        store.commit(&first),
        Ok(ExecutionCommitResult::Committed {
            new_program_state_version: 1,
            ..
        })
    ));

    let mut second = request("commit.observation-event-second", "instance.event", 1, None);
    second.tuple.event_wait = Some(json!({
        "continuation_id": "continuation.second",
        "event_ref": "event.second",
        "generation": 1
    }));
    second.tuple.observations = vec![observation(
        "event_waiting",
        "provisional",
        1,
        None,
        None,
        Some("event.first"),
    )];
    bind_observation_digest(&mut second);

    let before = store.clone();
    assert!(matches!(
        store.commit(&second),
        Err(apxm_commit_local::CommitLocalError::InvalidRequest(message))
            if message.contains("current commit event_wait")
    ));
    assert_eq!(
        store, before,
        "cross-commit event rejection must publish no durable mutation"
    );
    assert_eq!(
        store.current_version(&ProgramInstanceRef::new("instance.event")),
        1
    );

    let mut resumed = request("commit.observation-event-resume", "instance.event", 1, None);
    let mut resumed_observation = observation(
        "event_resumed",
        "provisional",
        1,
        None,
        None,
        Some("event.first"),
    );
    resumed_observation["event_ref"] = json!({
        "event_ref": "event.first",
        "generation": 7,
        "occurrence_id": "occurrence.first"
    });
    resumed.tuple.observations = vec![resumed_observation];
    bind_observation_digest(&mut resumed);
    assert!(matches!(
        store.commit(&resumed),
        Ok(ExecutionCommitResult::Committed {
            new_program_state_version: 2,
            ..
        })
    ));
}

#[test]
fn event_resumed_cannot_reuse_a_prior_wait_from_another_invocation() {
    let mut store = CommitLocalStore::new();
    let mut first = request("commit.event-owner", "instance.event-owner", 0, None);
    first.program_invocation_ref = ProgramInvocationRef::new("invoke.first");
    first.tuple.event_wait = Some(json!({
        "continuation_id": "continuation.first",
        "event_ref": "event.first",
        "generation": 7,
        "occurrence_id": "occurrence.first"
    }));
    bind_observation_digest(&mut first);
    assert!(matches!(
        store.commit(&first),
        Ok(ExecutionCommitResult::Committed {
            new_program_state_version: 1,
            ..
        })
    ));

    let mut second = request("commit.event-reuse", "instance.event-owner", 1, None);
    second.program_invocation_ref = ProgramInvocationRef::new("invoke.second");
    let mut resumed = observation_value("invoke.second", 1);
    resumed["observation_kind"] = json!("event_resumed");
    resumed["event_ref"] = json!({
        "event_ref": "event.first",
        "generation": 7,
        "occurrence_id": "occurrence.first"
    });
    second.tuple.observations = vec![resumed];
    bind_observation_digest(&mut second);

    let before = store.clone();
    assert!(matches!(
        store.commit(&second),
        Err(apxm_commit_local::CommitLocalError::InvalidRequest(message))
            if message.contains("prior committed event_wait")
    ));
    assert_eq!(
        store, before,
        "cross-invocation resume rejection must publish no durable mutation"
    );
    assert_eq!(
        store.current_version(&ProgramInstanceRef::new("instance.event-owner")),
        1
    );
}

#[test]
fn fulfilled_event_wait_and_resume_may_commit_together() {
    let mut store = CommitLocalStore::new();
    let mut request = request("commit.event-fulfilled", "instance.event", 0, None);
    request.tuple.observations = vec![
        observation(
            "event_waiting",
            "provisional",
            1,
            None,
            None,
            Some("event.fulfilled"),
        ),
        observation(
            "event_resumed",
            "provisional",
            2,
            None,
            None,
            Some("event.fulfilled"),
        ),
    ];
    bind_observation_digest(&mut request);

    assert!(matches!(
        store.commit(&request),
        Ok(ExecutionCommitResult::Committed {
            new_program_state_version: 1,
            ..
        })
    ));
}

#[test]
fn non_local_session_output_ref_bounds_fail_before_identity_persistence() {
    for (field, invalid_value) in [
        ("access_scope_ref", json!("s".repeat(257))),
        ("disclosure_ref", json!("d".repeat(257))),
    ] {
        let mut store = CommitLocalStore::new();
        store.inject_outcome_unknown("commit.invalid-output");
        let mut output = json!({
            "contract": "apxm.session-output-ref.v1",
            "ref_type": "SessionOutputRef",
            "ref": "remote.output",
            "program_instance_id": "instance.remote",
            "program_invocation_id": "invoke.1",
            "content_digest": digest('a'),
            "byte_length": 0,
            "media_type": "text/plain",
            "commitment": "committed",
            "access_scope_ref": "scope.remote"
        });
        output[field] = invalid_value;
        let mut commit = request("commit.invalid-output", "instance.remote", 0, None);
        commit.tuple.output_refs = vec![output];
        bind_observation_digest(&mut commit);

        let before = store.clone();
        let result = store.commit(&commit);
        assert!(matches!(
            result,
            Err(apxm_commit_local::CommitLocalError::InvalidOutput(_))
        ));
        assert_eq!(
            store, before,
            "invalid non-local Session Output metadata must not persist identity"
        );
        assert_eq!(
            store.current_version(&ProgramInstanceRef::new("instance.remote")),
            0
        );
    }
}

#[test]
fn valid_non_local_session_output_ref_remains_allowed() {
    let mut store = CommitLocalStore::new();
    let output = json!({
        "contract": "apxm.session-output-ref.v1",
        "ref_type": "SessionOutputRef",
        "ref": "remote.output",
        "program_instance_id": "instance.remote",
        "program_invocation_id": "invoke.1",
        "content_digest": digest('a'),
        "byte_length": 0,
        "media_type": "text/plain",
        "commitment": "committed",
        "access_scope_ref": "scope.remote"
    });
    let mut commit = request("commit.remote-output", "instance.remote", 0, None);
    commit.tuple.output_refs = vec![output];
    bind_observation_digest(&mut commit);

    assert!(matches!(
        store.commit(&commit),
        Ok(ExecutionCommitResult::Committed {
            new_program_state_version: 1,
            ..
        })
    ));
}

#[tokio::test]
async fn kernel_output_staging_api_returns_a_validated_opaque_ref() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    let prepared = ExecutionCommitPort::prepare_output(
        &port,
        apxm_kernel::SessionOutputPreparation {
            contract: apxm_kernel::SESSION_OUTPUT_REF_CONTRACT.into(),
            commit_id: "commit.kernel-output".into(),
            program_instance_ref: "instance.kernel-output".into(),
            program_invocation_ref: "invoke.kernel-output".into(),
            content: b"kernel-staged".to_vec(),
            media_type: "text/plain".into(),
            visibility: apxm_kernel::SessionOutputVisibility::Provisional,
            node_execution_id: Some("node.kernel-output".into()),
            occurrence_id: Some("occurrence.kernel-output".into()),
            access_scope_ref: "scope.kernel-output".into(),
            disclosure_ref: None,
        },
    )
    .await
    .expect("kernel staging");
    prepared.validate().expect("validated output ref");
    assert_eq!(prepared.program_instance_id, "instance.kernel-output");
    assert_eq!(prepared.program_invocation_id, "invoke.kernel-output");
    assert_eq!(
        prepared.visibility,
        apxm_kernel::SessionOutputVisibility::Committed
    );
}

#[tokio::test]
async fn output_prepare_and_commit_fail_closed_on_scope_escape_and_conflict() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    let prepared = port
        .prepare_output(output_preparation(
            "commit.scope",
            "instance.scope",
            "invoke.scope",
        ))
        .expect("prepare scoped output");
    let mut escaped = request("commit.scope", "instance.other", 0, None);
    escaped.program_invocation_ref = ProgramInvocationRef::new("invoke.other");
    escaped.tuple.output_refs = vec![prepared.to_json()];
    assert!(matches!(
        port.commit(escaped).await,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert_eq!(
        port.current_version(&ProgramInstanceRef::new("instance.other"))
            .await,
        0
    );
    assert!(matches!(
        port.read_committed_output(
            read_context(ReadPurpose::Content, "instance.scope"),
            ContentRef::new(prepared.output_ref.clone()).expect("content ref"),
        ),
        Err(apxm_commit_local::CommitLocalError::ProvisionalOutput)
    ));
    let mut malformed = prepared.to_json();
    malformed["content_digest"] = json!(digest('f'));
    let mut malformed_request = request("commit.scope", "instance.scope", 0, None);
    malformed_request.program_invocation_ref = ProgramInvocationRef::new("invoke.scope");
    malformed_request.tuple.output_refs = vec![malformed];
    assert!(matches!(
        port.commit(malformed_request).await,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    let mut forged_instance = prepared.to_json();
    forged_instance["program_instance_id"] = json!("instance.foraged");
    let mut forged_request = request("commit.scope.forged", "instance.scope", 0, None);
    forged_request.program_invocation_ref = ProgramInvocationRef::new("invoke.scope");
    forged_request.tuple.output_refs = vec![forged_instance];
    assert!(matches!(
        port.commit(forged_request).await,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    let mut provisional = prepared.to_json();
    provisional["visibility"] = json!("provisional");
    let mut provisional_request = request("commit.scope.provisional", "instance.scope", 0, None);
    provisional_request.program_invocation_ref = ProgramInvocationRef::new("invoke.scope");
    provisional_request.tuple.output_refs = vec![provisional];
    assert!(matches!(
        port.commit(provisional_request).await,
        ExecutionCommitResult::OutcomeUnknown { .. }
    ));
    assert!(
        port.reclaim_prepared_output(&prepared.output_ref)
            .expect("reclaim orphan")
    );

    assert!(matches!(
        port.commit(request("commit.base", "instance.output", 0, None))
            .await,
        ExecutionCommitResult::Committed { .. }
    ));
    let conflict = port
        .prepare_output(output_preparation(
            "commit.conflict-output",
            "instance.output",
            "invoke.output",
        ))
        .expect("prepare conflict output");
    let mut conflicting = request("commit.conflict-output", "instance.output", 0, None);
    conflicting.program_invocation_ref = ProgramInvocationRef::new("invoke.output");
    conflicting.tuple.output_refs = vec![conflict.to_json()];
    bind_observation_digest(&mut conflicting);
    assert!(matches!(
        port.commit(conflicting).await,
        ExecutionCommitResult::CompareConflict {
            current_program_state_version: 1
        }
    ));
    assert!(matches!(
        port.read_committed_output(
            read_context(ReadPurpose::Content, "instance.output"),
            ContentRef::new(conflict.output_ref.clone()).expect("content ref"),
        ),
        Err(apxm_commit_local::CommitLocalError::ProvisionalOutput)
    ));
}

#[test]
fn output_refs_are_owner_local_and_cannot_escape_by_path() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    assert!(port.reclaim_prepared_output("../../outside").is_err());
    let too_large = output_preparation("commit.large-output", "instance.large", "invoke.large");
    let mut too_large = too_large;
    too_large.content = vec![b'x'; apxm_commit_local::MAX_OUTPUT_BYTES + 1];
    assert!(matches!(
        port.prepare_output(too_large),
        Err(apxm_commit_local::CommitLocalError::OutputTooLarge { .. })
    ));
}

#[tokio::test]
async fn in_memory_owner_local_commit_resume_and_conflict() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    assert_happy_path(&port).await;

    port.inject_outcome_unknown("commit.unknown");
    let mut unknown_request = request("commit.unknown", "instance.2", 0, None);
    unknown_request.program_invocation_ref = ProgramInvocationRef::new("invoke.unknown");
    let unknown = port.commit(unknown_request).await;
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
    let inspection = port
        .read_execution(ExecutionReadRequest::ProgramInvocationInspect {
            context: read_context(ReadPurpose::Inspection, "scope.read"),
            program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(
                "invoke.unknown",
            )
            .expect("invocation"),
            node_execution_id: None,
        })
        .expect("forced unknown remains inspectable");
    assert!(matches!(
        inspection,
        ExecutionReadResult::ProgramInvocationInspection { inspection }
            if inspection.status == apxm_runtime_protocol::ProgramInvocationStatus::OutcomeUnknown
    ));
}

#[tokio::test]
async fn loop_completion_fact_cannot_cross_invocation_scope() {
    let mut store = CommitLocalStore::new();
    let fact = Fact::LoopIterationCompleted(LoopIterationCompletedFact::new(
        "fact.invoke.1.1".into(),
        1,
        "loop.1".into(),
        "occurrence.1".into(),
        0,
        "invoke.other".into(),
        vec!["node.1".into()],
    ));
    let mut request = request("commit.cross-invocation-loop", "instance.1", 0, None);
    request.tuple.evidence = vec![fact.clone()];
    request.evidence_batch = vec![fact];
    bind_observation_digest(&mut request);
    assert!(matches!(
        store.commit(&request),
        Err(apxm_commit_local::CommitLocalError::InvalidRequest(message))
            if message.contains("evidence fact invocation")
    ));
}

#[tokio::test]
async fn malformed_node_coordinate_is_rejected_before_commit_or_indexing() {
    let mut store = CommitLocalStore::new();
    let fact = Fact::NodeExecutionRecorded(NodeExecutionRecordedFact {
        fact_id: "fact.invoke.1.1".into(),
        event_sequence: 1,
        program_invocation_id: "invoke.1".into(),
        node_execution_id: "node execution with spaces".into(),
        air_node_id: "air.node.1".into(),
        parent_node_execution_id: None,
        execution_scope: NodeExecutionScope::NonLoop,
    });
    let mut request = request("commit.malformed-node", "instance.1", 0, None);
    request.tuple.evidence = vec![fact.clone()];
    request.evidence_batch = vec![fact];
    bind_observation_digest(&mut request);

    assert!(matches!(
        store.commit(&request),
        Err(apxm_commit_local::CommitLocalError::InvalidRequest(message))
            if message.contains("node_execution_id")
    ));
    assert_eq!(
        store.current_version(&ProgramInstanceRef::new("instance.1")),
        0,
        "malformed coordinates must not advance the commit"
    );
}

#[tokio::test]
async fn malformed_region_occurrence_is_rejected_before_commit_or_indexing() {
    let mut store = CommitLocalStore::new();
    let fact = Fact::NodeExecutionRecorded(NodeExecutionRecordedFact {
        fact_id: "fact.invoke.1.1".into(),
        event_sequence: 1,
        program_invocation_id: "invoke.1".into(),
        node_execution_id: "node.1".into(),
        air_node_id: "air.node.1".into(),
        parent_node_execution_id: None,
        execution_scope: NodeExecutionScope::Loop {
            region_occurrence_id: "region occurrence with spaces".into(),
            static_region_id: "region.1".into(),
            loop_memberships: vec![apxm_program::runtime_evidence::LoopMembership {
                static_loop_id: "loop.1".into(),
                loop_occurrence_id: "loop-occurrence.1".into(),
            }],
        },
    });
    let mut request = request("commit.malformed-region", "instance.1", 0, None);
    request.tuple.evidence = vec![fact.clone()];
    request.evidence_batch = vec![fact];
    bind_observation_digest(&mut request);

    assert!(matches!(
        store.commit(&request),
        Err(apxm_commit_local::CommitLocalError::InvalidRequest(message))
            if message.contains("region_occurrence_id")
    ));
    assert_eq!(
        store.current_version(&ProgramInstanceRef::new("instance.1")),
        0,
    );
}

#[tokio::test]
async fn malformed_observation_coordinate_is_rejected_before_commit_or_indexing() {
    let mut store = CommitLocalStore::new();
    let mut request = request("commit.malformed-observation", "instance.1", 0, None);
    request.tuple.observations = vec![json!({
        "contract": "apxm.execution-observation.v1",
        "observation_id": "observation.invoke.1.1",
        "program_invocation_id": "invoke.1",
        "node_execution_id": "node execution with spaces",
        "occurrence_id": "occurrence.1",
        "sequence": 1,
        "cursor": {"position": 1, "token": "cursor.1"},
        "timing": {"observed_at_unix_ms": 1},
        "observation_kind": "node_started",
        "commitment": "provisional"
    })];
    bind_observation_digest(&mut request);

    assert!(matches!(
        store.commit(&request),
        Err(apxm_commit_local::CommitLocalError::InvalidRequest(message))
            if message.contains("invalid execution observation")
    ));
    assert_eq!(
        store.current_version(&ProgramInstanceRef::new("instance.1")),
        0,
    );
}

#[tokio::test]
async fn filesystem_owner_local_survives_reopen_and_is_portable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = FilesystemExecutionCommit::open_with_read_access_hook(
        dir.path(),
        Arc::new(AllowReadAccess),
    )
    .expect("open");
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

#[tokio::test]
async fn terminal_typed_error_projection_survives_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = FilesystemExecutionCommit::open_with_read_access_hook(
        dir.path(),
        Arc::new(AllowReadAccess),
    )
    .expect("open");
    let invocation = "invoke.typed-error";
    let mut runtime = serde_json::from_value::<RuntimeFact>(json!({
        "fact_id": format!("fact.{invocation}.1"),
        "event_sequence": 1,
        "invocation_state": "failed",
        "node_execution_id": "node.typed-error"
    }))
    .expect("runtime fact");
    runtime.typed_error = Some(TypedErrorEnvelope {
        error_id: "model-failure.node.typed-error".into(),
        category: ErrorCategory::Unavailable,
        code_ref: "model_reported_timeout".into(),
        message: "the provider reported a terminal timeout".into(),
        details_digest: None,
    });
    let fact = Fact::from_runtime(FactKind::InvocationFailed, runtime);
    let mut commit = request("commit.typed-error", "instance.typed-error", 0, None);
    commit.program_invocation_ref = ProgramInvocationRef::new(invocation);
    commit.tuple.output_refs.clear();
    commit.tuple.evidence = vec![fact.clone()];
    commit.evidence_batch = vec![fact];
    bind_observation_digest(&mut commit);
    assert!(matches!(
        port.commit(commit).await,
        ExecutionCommitResult::Committed { .. }
    ));
    let root = port.root().to_path_buf();
    drop(port);

    let reopened =
        FilesystemExecutionCommit::open_with_read_access_hook(&root, Arc::new(AllowReadAccess))
            .expect("reopen");
    let page = match reopened
        .read_execution(ExecutionReadRequest::EvidenceRead {
            context: read_context(ReadPurpose::Evidence, "scope.typed-error"),
            program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(invocation)
                .expect("invocation"),
            after_cursor: None,
            limit: 10,
        })
        .expect("evidence read")
    {
        ExecutionReadResult::EvidencePage { page } => page,
        _ => panic!("wrong evidence page kind"),
    };
    let error = page.items[0].typed_error.as_ref().expect("typed error");
    assert_eq!(error.code_ref, "model_reported_timeout");
    assert_eq!(
        error.category,
        apxm_runtime_protocol::EvidenceErrorCategory::Unavailable
    );
    assert!(page.items[0].validate().is_ok());
}

#[tokio::test]
async fn filesystem_store_tampering_fails_authentication_before_resume() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = FilesystemExecutionCommit::open(dir.path()).expect("open");
    assert!(matches!(
        port.commit(request(
            "commit.tamper",
            "instance.tamper",
            0,
            Some(json!({"pc": 7}))
        ))
        .await,
        ExecutionCommitResult::Committed { .. }
    ));
    let root = port.root().to_path_buf();
    drop(port);

    let path = root.join("execution-commit-local.v2.json");
    let mut persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read store")).expect("store JSON");
    persisted["instances"]["instance.tamper"]["continuation"]["pc"] = json!(99);
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&persisted).expect("tampered JSON"),
    )
    .expect("write tampered store");

    assert!(matches!(
        FilesystemExecutionCommit::open(root),
        Err(apxm_commit_local::CommitLocalError::AuthenticationFailed(_))
    ));
}

#[tokio::test]
async fn filesystem_prepared_output_survives_reopen_after_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = FilesystemExecutionCommit::open(dir.path()).expect("open");
    let prepared = port
        .prepare_output(output_preparation(
            "commit.filesystem-output",
            "instance.filesystem-output",
            "invoke.filesystem-output",
        ))
        .expect("prepare filesystem output");
    let mut commit = request(
        "commit.filesystem-output",
        "instance.filesystem-output",
        0,
        None,
    );
    commit.program_invocation_ref = ProgramInvocationRef::new("invoke.filesystem-output");
    commit.tuple.output_refs = vec![prepared.to_json()];
    bind_observation_digest(&mut commit);
    assert!(matches!(
        port.commit(commit).await,
        ExecutionCommitResult::Committed { .. }
    ));
    let root = port.root().to_path_buf();
    drop(port);

    let reopened =
        FilesystemExecutionCommit::open_with_read_access_hook(root, Arc::new(AllowReadAccess))
            .expect("reopen");
    assert_eq!(
        reopened
            .read_committed_output(
                read_context(ReadPurpose::Content, "instance.filesystem-output"),
                ContentRef::new(prepared.output_ref.clone()).expect("content ref"),
            )
            .expect("committed output")
            .bytes,
        b"durable-session-output".to_vec()
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

#[test]
fn filesystem_store_size_is_bounded_before_json_decode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("execution-commit-local.v2.json");
    let file = std::fs::File::create(path).expect("create store");
    file.set_len((MAX_STORE_BYTES as u64) + 1)
        .expect("create sparse oversized store");

    let error = match FilesystemExecutionCommit::open(dir.path()) {
        Ok(_) => panic!("an oversized persisted store must fail closed"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        apxm_commit_local::CommitLocalError::StoreTooLarge { .. }
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

fn read_context(purpose: ReadPurpose, scope: &str) -> ReadContext {
    ReadContext {
        request_id: RequestId::new("request.read").expect("request"),
        scope_ref: ScopeRef::new(scope).expect("scope"),
        principal_ref: PrincipalRef::new("principal.read").expect("principal"),
        grant_ref: GrantRef::new("grant.read").expect("grant"),
        correlation_id: None,
        purpose,
    }
}

fn observation_value(invocation: &str, sequence: u64) -> serde_json::Value {
    json!({
        "contract": "apxm.execution-observation.v1",
        "observation_id": format!("observation.{invocation}.{sequence}"),
        "program_invocation_id": invocation,
        "sequence": sequence,
        "cursor": {"position": sequence, "token": format!("cursor.{sequence}")},
        "timing": {"observed_at_unix_ms": sequence},
        "observation_kind": "invocation_started",
        "commitment": "provisional"
    })
}

struct AuditFailureHook;

impl ReadAccessHook for AuditFailureHook {
    fn reauthorize(&self, _request: &ReadAuthorization) -> Result<(), String> {
        Ok(())
    }

    fn audit(&self, _event: ReadAudit) -> Result<(), String> {
        Err("audit sink unavailable".into())
    }
}

struct ScopeTargetHook {
    seen: Arc<Mutex<Vec<ReadTarget>>>,
}

impl ReadAccessHook for ScopeTargetHook {
    fn reauthorize(&self, request: &ReadAuthorization) -> Result<(), String> {
        if request.context.scope_ref.as_str() == "scope.allowed" {
            self.seen
                .lock()
                .expect("target capture lock")
                .push(request.target.clone());
            Ok(())
        } else {
            Err("scope is not authorized".into())
        }
    }

    fn audit(&self, _event: ReadAudit) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn reads_fail_closed_without_explicit_authorization_binding() {
    let port = InMemoryExecutionCommit::new();
    let result = port.read_execution(ExecutionReadRequest::ProgramInvocationInspect {
        context: read_context(ReadPurpose::Inspection, "scope.denied"),
        program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new("invoke.denied")
            .expect("invocation"),
        node_execution_id: None,
    });
    assert!(matches!(
        result,
        Err(apxm_commit_local::CommitLocalError::Unauthorized(_))
    ));
}

#[test]
fn reads_fail_closed_when_audit_hook_fails() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AuditFailureHook));
    let result = port.read_execution(ExecutionReadRequest::ProgramInvocationInspect {
        context: read_context(ReadPurpose::Inspection, "scope.audit"),
        program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new("invoke.audit")
            .expect("invocation"),
        node_execution_id: None,
    });
    assert!(matches!(
        result,
        Err(apxm_commit_local::CommitLocalError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn read_authorization_is_target_aware_and_scope_isolated() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(ScopeTargetHook {
        seen: seen.clone(),
    }));
    let prepared = port
        .prepare_output(output_preparation(
            "commit.scope-target",
            "instance.scope-target",
            "invoke.1",
        ))
        .expect("prepare output");
    let mut commit = request("commit.scope-target", "instance.scope-target", 0, None);
    commit.tuple.output_refs = vec![prepared.to_json()];
    bind_observation_digest(&mut commit);
    assert!(matches!(
        port.commit(commit).await,
        ExecutionCommitResult::Committed { .. }
    ));
    let content_ref = ContentRef::new(prepared.output_ref).expect("content ref");
    assert!(
        port.read_committed_output(
            read_context(ReadPurpose::Content, "scope.allowed"),
            content_ref.clone(),
        )
        .is_ok()
    );
    assert!(matches!(
        port.read_committed_output(
            read_context(ReadPurpose::Content, "scope.other"),
            content_ref,
        ),
        Err(apxm_commit_local::CommitLocalError::Unauthorized(_))
    ));
    assert!(
        seen.lock()
            .expect("target capture lock")
            .iter()
            .any(|target| matches!(target, ReadTarget::Content { .. }))
    );
}

#[tokio::test]
async fn replacing_in_memory_read_hook_preserves_durable_records() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    assert!(matches!(
        port.commit(request("commit.hook", "instance.hook", 0, None))
            .await,
        ExecutionCommitResult::Committed { .. }
    ));

    port.set_read_access_hook(Arc::new(DenyReadAccess));
    let denied = port.read_execution(ExecutionReadRequest::ProgramInvocationInspect {
        context: read_context(ReadPurpose::Inspection, "scope.hook"),
        program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new("invoke.1")
            .expect("invocation"),
        node_execution_id: None,
    });
    assert!(matches!(
        denied,
        Err(apxm_commit_local::CommitLocalError::Unauthorized(_))
    ));

    port.set_read_access_hook(Arc::new(AllowReadAccess));
    assert!(matches!(
        port.read_execution(ExecutionReadRequest::ProgramInvocationInspect {
            context: read_context(ReadPurpose::Inspection, "scope.hook"),
            program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new("invoke.1")
                .expect("invocation"),
            node_execution_id: None,
        }),
        Ok(ExecutionReadResult::ProgramInvocationInspection { .. })
    ));
}

#[tokio::test]
async fn typed_reads_are_scope_bound_and_only_committed_output_is_visible() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    let mut preparation = output_preparation("commit.typed", "instance.typed", "invoke.typed");
    preparation.node_execution_id = Some(
        apxm_runtime_protocol::execution_contracts::NodeExecutionId::new("node.typed")
            .expect("node"),
    );
    preparation.occurrence_id = Some(
        apxm_runtime_protocol::execution_contracts::OccurrenceId::new("occurrence.typed")
            .expect("occurrence"),
    );
    let prepared = port.prepare_output(preparation).expect("prepare");
    let content_ref = ContentRef::new(prepared.output_ref.clone()).expect("content ref");
    let provisional = port.read_committed_output(
        read_context(ReadPurpose::Content, "scope.typed"),
        content_ref.clone(),
    );
    assert!(provisional.is_err());

    let mut commit = request("commit.typed", "instance.typed", 0, None);
    commit.program_invocation_ref = ProgramInvocationRef::new("invoke.typed");
    commit.tuple.output_refs = vec![prepared.to_json()];
    commit.tuple.observations = vec![json!({
        "contract": "apxm.execution-observation.v1",
        "observation_id": "observation.invoke.typed.1",
        "program_invocation_id": "invoke.typed",
        "node_execution_id": "node.typed",
        "occurrence_id": "occurrence.typed",
        "sequence": 1,
        "cursor": {"position": 1, "token": "cursor.1"},
        "timing": {"observed_at_unix_ms": 1},
        "observation_kind": "node_started",
        "commitment": "provisional"
    })];
    bind_observation_digest(&mut commit);
    assert!(matches!(
        port.commit(commit).await,
        ExecutionCommitResult::Committed { .. }
    ));
    let content = port
        .read_committed_output(
            read_context(ReadPurpose::Content, "instance.typed"),
            content_ref,
        )
        .expect("typed content read");
    assert_eq!(
        content.visibility,
        apxm_runtime_protocol::OutputVisibility::Committed
    );
    assert_eq!(
        content
            .output_ref
            .expect("output")
            .node_execution_id
            .expect("node")
            .as_str(),
        "node.typed"
    );

    let page = match port
        .read_execution(ExecutionReadRequest::ObservationSubscribe {
            context: read_context(ReadPurpose::Observation, "scope.typed"),
            program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new("invoke.typed")
                .expect("invocation"),
            after_cursor: None,
            limit: 1,
        })
        .expect("observation read")
    {
        ExecutionReadResult::ObservationPage { page } => page,
        _ => panic!("wrong page kind"),
    };
    let cursor = page.item_cursors[0].clone();
    let mut wrong_scope = read_context(ReadPurpose::Observation, "scope.other");
    let wrong = port.read_execution(ExecutionReadRequest::ObservationSubscribe {
        context: wrong_scope.clone(),
        program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new("invoke.typed")
            .expect("invocation"),
        after_cursor: Some(cursor.clone()),
        limit: 1,
    });
    assert!(wrong.is_err());
    wrong_scope.scope_ref = ScopeRef::new("scope.typed").expect("scope");
    let resumed = port.read_execution(ExecutionReadRequest::ObservationSubscribe {
        context: wrong_scope,
        program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new("invoke.typed")
            .expect("invocation"),
        after_cursor: Some(cursor),
        limit: 1,
    });
    assert!(resumed.is_ok());
}

#[tokio::test]
async fn observation_watermark_survives_retention_and_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = FilesystemExecutionCommit::open_with_read_access_hook(
        dir.path(),
        Arc::new(AllowReadAccess),
    )
    .expect("open");
    let mut first = request("commit.observations.1", "instance.observations", 0, None);
    first.program_invocation_ref = ProgramInvocationRef::new("invoke.observations");
    first.tuple.output_refs.clear();
    first.tuple.observations = (1..=(apxm_commit_local::MAX_READ_RECORDS as u64 + 1))
        .map(|sequence| observation_value("invoke.observations", sequence))
        .collect();
    bind_observation_digest(&mut first);
    assert!(matches!(
        port.commit(first).await,
        ExecutionCommitResult::Committed { .. }
    ));
    let root = port.root().to_path_buf();
    drop(port);

    let reopened =
        FilesystemExecutionCommit::open_with_read_access_hook(&root, Arc::new(AllowReadAccess))
            .expect("reopen");
    let page = match reopened
        .read_execution(ExecutionReadRequest::ObservationSubscribe {
            context: read_context(ReadPurpose::Observation, "scope.observations"),
            program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(
                "invoke.observations",
            )
            .expect("invocation"),
            after_cursor: None,
            limit: 1,
        })
        .expect("read observations")
    {
        ExecutionReadResult::ObservationPage { page } => page,
        _ => panic!("wrong page kind"),
    };
    assert_eq!(page.retention_floor.position, 2);
    assert_eq!(page.high_watermark.position, MAX_READ_RECORDS as u64 + 1);
    assert_eq!(page.items[0].sequence, 2);

    let mut second = request("commit.observations.2", "instance.observations", 1, None);
    second.program_invocation_ref = ProgramInvocationRef::new("invoke.observations");
    second.tuple.output_refs.clear();
    second.tuple.observations = vec![observation_value(
        "invoke.observations",
        MAX_READ_RECORDS as u64 + 2,
    )];
    bind_observation_digest(&mut second);
    assert!(matches!(
        reopened.commit(second).await,
        ExecutionCommitResult::Committed { .. }
    ));
    let page = match reopened
        .read_execution(ExecutionReadRequest::ObservationSubscribe {
            context: read_context(ReadPurpose::Observation, "scope.observations"),
            program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(
                "invoke.observations",
            )
            .expect("invocation"),
            after_cursor: None,
            limit: 1,
        })
        .expect("read observations")
    {
        ExecutionReadResult::ObservationPage { page } => page,
        _ => panic!("wrong page kind"),
    };
    assert_eq!(page.retention_floor.position, 3);
    assert_eq!(page.high_watermark.position, MAX_READ_RECORDS as u64 + 2);
}

#[tokio::test]
async fn observation_and_evidence_pages_resume_without_skipping_or_repeating() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    let invocation = "invoke.pages";
    let facts = (1..=3)
        .map(|sequence| {
            Fact::from_runtime(
                FactKind::InstanceCreated,
                serde_json::from_value::<RuntimeFact>(json!({
                    "fact_id": format!("fact.{invocation}.{sequence}"),
                    "event_sequence": sequence
                }))
                .expect("runtime fact"),
            )
        })
        .collect::<Vec<_>>();
    let mut commit = request("commit.pages", "instance.pages", 0, None);
    commit.program_invocation_ref = ProgramInvocationRef::new(invocation);
    commit.tuple.output_refs.clear();
    commit.tuple.evidence = facts.clone();
    commit.evidence_batch = facts;
    commit.tuple.observations = (1..=3)
        .map(|sequence| observation_value(invocation, sequence))
        .collect();
    bind_observation_digest(&mut commit);
    assert!(matches!(
        port.commit(commit).await,
        ExecutionCommitResult::Committed { .. }
    ));

    let context = read_context(ReadPurpose::Observation, "scope.pages");
    let mut after_cursor = None;
    let mut observed = Vec::new();
    loop {
        let page = match port
            .read_execution(ExecutionReadRequest::ObservationSubscribe {
                context: context.clone(),
                program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(invocation)
                    .expect("invocation"),
                after_cursor,
                limit: 1,
            })
            .expect("observation page")
        {
            ExecutionReadResult::ObservationPage { page } => page,
            _ => panic!("wrong observation page kind"),
        };
        observed.extend(page.items.iter().map(|item| item.sequence));
        after_cursor = page.next_cursor;
        if !page.has_more {
            break;
        }
    }
    assert_eq!(observed, [1, 2, 3]);

    let context = read_context(ReadPurpose::Evidence, "scope.pages");
    let mut after_cursor = None;
    let mut evidence = Vec::new();
    let mut evidence_refs = Vec::new();
    loop {
        let page = match port
            .read_execution(ExecutionReadRequest::EvidenceRead {
                context: context.clone(),
                program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(invocation)
                    .expect("invocation"),
                after_cursor,
                limit: 1,
            })
            .expect("evidence page")
        {
            ExecutionReadResult::EvidencePage { page } => page,
            _ => panic!("wrong evidence page kind"),
        };
        evidence.extend(page.items.iter().map(|item| item.sequence));
        evidence_refs.extend(
            page.items
                .iter()
                .map(|item| item.evidence_ref.as_str().to_owned()),
        );
        after_cursor = page.next_cursor;
        if !page.has_more {
            break;
        }
    }
    assert_eq!(evidence, [1, 2, 3]);
    assert_eq!(
        evidence_refs,
        [
            "evidence.invoke.pages.commit.pages.1",
            "evidence.invoke.pages.commit.pages.2",
            "evidence.invoke.pages.commit.pages.3",
        ]
    );
}

#[tokio::test]
async fn evidence_read_projects_committed_model_attempt_without_private_fields() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    let invocation = "invoke.model";
    let attempt_digest = digest('a');
    let attempt = Fact::AttemptRecorded(ModelAttemptRecordedFact {
        fact_id: "fact.invoke.model.1".into(),
        event_sequence: 1,
        program_invocation_id: invocation.into(),
        node_execution_id: "node.execution".into(),
        air_node_id: "node.model".into(),
        attempt_id: "model-attempt.node.execution.0".into(),
        attempt_index: 0,
        model_effect_id: "effect.model.1".into(),
        request_digest: attempt_digest.clone(),
        model_target_ref: "model-target.1".into(),
        model_target_digest: attempt_digest.clone(),
        model_deployment_ref: "deployment.1".into(),
        exact_port_binding_digest: attempt_digest.clone(),
        target_commitment_digest: attempt_digest.clone(),
        generation_cohort_digest: attempt_digest.clone(),
        target_generation: 1,
        target_port_contract_digest: attempt_digest.clone(),
        target_composition_digest: attempt_digest.clone(),
        native_input_tokens: 4,
        native_output_tokens: 5,
    });
    let mut commit = request("commit.model", "instance.model", 0, None);
    commit.program_invocation_ref = ProgramInvocationRef::new(invocation);
    commit.tuple.evidence = vec![attempt.clone()];
    commit.evidence_batch = vec![attempt];
    commit.tuple.output_refs.clear();
    bind_observation_digest(&mut commit);
    assert!(matches!(
        port.commit(commit).await,
        ExecutionCommitResult::Committed { .. }
    ));

    let result = port
        .read_execution(ExecutionReadRequest::EvidenceRead {
            context: read_context(ReadPurpose::Evidence, "scope.model"),
            program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(invocation)
                .expect("invocation"),
            after_cursor: None,
            limit: 10,
        })
        .expect("evidence read");
    let ExecutionReadResult::EvidencePage { page } = result else {
        panic!("wrong evidence page kind");
    };
    let record = &page.items[0];
    let model = record.model_attempt.as_ref().expect("typed model attempt");
    assert_eq!(
        record.fact_kind,
        apxm_runtime_protocol::EvidenceFactKind::AttemptRecorded
    );
    assert_eq!(model.model_target_ref, "model-target.1");
    assert_eq!(model.model_target_digest, digest('a'));
    assert_eq!(model.program_invocation_id, invocation);
    assert!(
        !serde_json::to_string(record)
            .expect("record JSON")
            .contains("owner_claim")
    );
    assert!(record.validate().is_ok());
}

#[tokio::test]
async fn live_observation_overlay_converges_without_duplicates_or_cursor_lies() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    let invocation = "invoke.live";
    let mut commit = request("commit.live", "instance.live", 0, None);
    commit.program_invocation_ref = ProgramInvocationRef::new(invocation);
    commit.tuple.output_refs.clear();
    commit.tuple.observations = (1..=2)
        .map(|sequence| observation_value(invocation, sequence))
        .collect();
    bind_observation_digest(&mut commit);
    assert!(matches!(
        port.commit(commit).await,
        ExecutionCommitResult::Committed { .. }
    ));

    // Sequence 2 is already durable. Sequence 3 is process-local and must be
    // pageable while execution runs, without a second copy of sequence 2.
    let live = [
        serde_json::from_value(observation_value(invocation, 2)).expect("live duplicate"),
        serde_json::from_value(observation_value(invocation, 3)).expect("live item"),
    ];
    let page = match port
        .read_execution_with_live(
            ExecutionReadRequest::ObservationSubscribe {
                context: read_context(ReadPurpose::Observation, "scope.live"),
                program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(invocation)
                    .expect("invocation"),
                after_cursor: None,
                limit: 10,
            },
            &live,
        )
        .expect("live observation page")
    {
        ExecutionReadResult::ObservationPage { page } => page,
        _ => panic!("wrong page kind"),
    };
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.sequence)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(page.high_watermark.position, 3);
    assert_eq!(page.items.last().expect("last item").cursor.position, 3);

    let resumed = match port
        .read_execution_with_live(
            ExecutionReadRequest::ObservationSubscribe {
                context: read_context(ReadPurpose::Observation, "scope.live"),
                program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(invocation)
                    .expect("invocation"),
                after_cursor: Some(page.items.last().expect("last item").cursor.clone()),
                limit: 10,
            },
            &live,
        )
        .expect("resumed live observation page")
    {
        ExecutionReadResult::ObservationPage { page } => page,
        _ => panic!("wrong resumed page kind"),
    };
    assert!(resumed.items.is_empty());
    assert_eq!(resumed.high_watermark.position, 3);
}

#[tokio::test]
async fn invocation_inspection_keeps_each_historical_invocation() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    for (commit_id, invocation, node, expected) in [
        ("commit.history.a", "invoke.history.a", "node.history.a", 0),
        ("commit.history.b", "invoke.history.b", "node.history.b", 1),
    ] {
        let mut commit = request(commit_id, "instance.history", expected, None);
        commit.program_invocation_ref = ProgramInvocationRef::new(invocation);
        commit.tuple.output_refs.clear();
        let mut observation = observation_value(invocation, 1);
        observation["node_execution_id"] = json!(node);
        commit.tuple.observations = vec![observation];
        bind_observation_digest(&mut commit);
        assert!(matches!(
            port.commit(commit).await,
            ExecutionCommitResult::Committed { .. }
        ));
    }
    for (invocation, node) in [
        ("invoke.history.a", "node.history.a"),
        ("invoke.history.b", "node.history.b"),
    ] {
        let result = port
            .read_execution(ExecutionReadRequest::ProgramInvocationInspect {
                context: read_context(ReadPurpose::Inspection, "scope.history"),
                program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(invocation)
                    .expect("invocation"),
                node_execution_id: None,
            })
            .expect("inspection");
        let ExecutionReadResult::ProgramInvocationInspection { inspection } = result else {
            panic!("wrong inspection result");
        };
        assert_eq!(inspection.node_execution_refs[0].as_str(), node);
    }
}

#[tokio::test]
async fn node_inspection_is_invocation_bound_and_has_no_synthetic_air_node() {
    let port = InMemoryExecutionCommit::with_read_access_hook(Arc::new(AllowReadAccess));
    let mut output = output_preparation("commit.node-inspection", "instance.node", "invoke.node");
    output.node_execution_id =
        Some(apxm_runtime_protocol::NodeExecutionId::new("node.execution").expect("node"));
    let prepared = port.prepare_output(output).expect("prepare node output");
    let mut commit = request("commit.node-inspection", "instance.node", 0, None);
    commit.program_invocation_ref = ProgramInvocationRef::new("invoke.node");
    commit.tuple.output_refs = vec![prepared.to_json()];
    let fact = Fact::NodeExecutionRecorded(NodeExecutionRecordedFact {
        fact_id: "fact.node".into(),
        event_sequence: 1,
        program_invocation_id: "invoke.node".into(),
        node_execution_id: "node.execution".into(),
        air_node_id: "air.model".into(),
        parent_node_execution_id: None,
        execution_scope: NodeExecutionScope::NonLoop,
    });
    commit.tuple.evidence = vec![fact.clone()];
    commit.evidence_batch = vec![fact];
    let mut started = observation_value("invoke.node", 1);
    started["node_execution_id"] = json!("node.execution");
    let mut waiting = observation_value("invoke.node", 2);
    waiting["node_execution_id"] = json!("node.execution");
    waiting["observation_kind"] = json!("event_waiting");
    waiting["event_ref"] = json!({
        "event_ref": "event.node",
        "generation": 1
    });
    let mut committed = observation_value("invoke.node", 4);
    committed["node_execution_id"] = json!("node.execution");
    committed["observation_kind"] = json!("content_committed");
    committed["commitment"] = json!("committed");
    committed["output_ref"] = json!(prepared.output_ref);
    commit.tuple.event_wait = Some(json!({
        "event_ref": "event.node",
        "generation": 1
    }));
    commit.tuple.observations = vec![started, waiting, committed];
    bind_observation_digest(&mut commit);
    assert!(matches!(
        port.commit(commit).await,
        ExecutionCommitResult::Committed { .. }
    ));
    let result = port
        .read_execution(ExecutionReadRequest::ProgramInvocationInspect {
            context: read_context(ReadPurpose::Inspection, "scope.node"),
            program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new("invoke.node")
                .expect("invocation"),
            node_execution_id: Some(
                apxm_runtime_protocol::NodeExecutionId::new("node.execution").expect("node"),
            ),
        })
        .expect("node inspection");
    let ExecutionReadResult::NodeExecutionInspection { inspection } = result else {
        panic!("wrong inspection result");
    };
    assert_eq!(inspection.air_node_id, "air.model");
    assert_eq!(
        inspection.status,
        apxm_runtime_protocol::NodeExecutionStatus::Succeeded
    );
    assert_eq!(
        inspection.commitment,
        apxm_runtime_protocol::Commitment::Committed
    );
    assert_eq!(
        inspection
            .output_ref
            .as_ref()
            .map(|reference| reference.as_str()),
        Some(prepared.output_ref.as_str())
    );
    assert_eq!(
        inspection
            .final_response_ref
            .as_ref()
            .map(|reference| reference.as_str()),
        Some(prepared.output_ref.as_str())
    );
}

fn host_capability_projection_commits(
    outcome: &str,
) -> (ExecutionCommitRequest, ExecutionCommitRequest) {
    let invocation = "invoke.host-projection";
    let mut requested = observation_value(invocation, 1);
    requested["observation_kind"] = json!("capability_requested");
    requested["node_execution_id"] = json!("node.host");
    requested["host_capability"] = json!({
        "capability_request_id": "request.host", "capability_ref": "host:notes.read",
        "input": "{}", "authored_permission": "allow"
    });
    let mut settled = observation_value(invocation, 2);
    settled["observation_kind"] = json!("capability_settled");
    settled["node_execution_id"] = json!("node.host");
    settled["host_capability"] = json!({
        "capability_request_id": "request.host", "capability_ref": "host:notes.read",
        "outcome": outcome, "receipt_ref": "receipt.host"
    });
    let mut later = observation_value(invocation, 3);
    later["observation_kind"] = json!("node_started");
    later["node_execution_id"] = json!("node.later");
    let mut commits = Vec::new();
    for (index, node, observations) in [
        (0, "host", vec![requested]),
        (1, "later", vec![settled, later]),
    ] {
        let mut commit = request(
            &format!("commit.host.{index}"),
            "instance.host",
            index,
            Some(json!({"pc": index + 1})),
        );
        commit.program_invocation_ref = ProgramInvocationRef::new(invocation);
        commit.tuple.output_refs.clear();
        let fact = Fact::NodeExecutionRecorded(NodeExecutionRecordedFact {
            fact_id: format!("fact.{invocation}.{index}"),
            event_sequence: index + 1,
            program_invocation_id: invocation.into(),
            node_execution_id: format!("node.{node}"),
            air_node_id: format!("air.{node}"),
            parent_node_execution_id: None,
            execution_scope: NodeExecutionScope::NonLoop,
        });
        commit.tuple.evidence = vec![fact.clone()];
        commit.evidence_batch = vec![fact];
        commit.tuple.observations = observations;
        if index == 0 {
            commit.tuple.event_wait = Some(json!({"event_ref":"request.host", "generation":1}));
        }
        bind_observation_digest(&mut commit);
        commits.push(commit);
    }
    (commits.remove(0), commits.remove(0))
}

fn host_node_read(node: &str) -> ExecutionReadRequest {
    ExecutionReadRequest::ProgramInvocationInspect {
        context: read_context(ReadPurpose::Inspection, "scope.host"),
        program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(
            "invoke.host-projection",
        )
        .unwrap(),
        node_execution_id: Some(apxm_runtime_protocol::NodeExecutionId::new(node).unwrap()),
    }
}

fn inspected_status(result: ExecutionReadResult) -> apxm_runtime_protocol::NodeExecutionStatus {
    let ExecutionReadResult::NodeExecutionInspection { inspection } = result else {
        panic!("node inspection expected")
    };
    assert_eq!(
        inspection.commitment,
        apxm_runtime_protocol::Commitment::Committed
    );
    inspection.status
}

#[tokio::test]
async fn host_capability_node_projection_is_durable_exact_and_survives_reopen() {
    use apxm_runtime_protocol::NodeExecutionStatus::{
        Failed, Running, Succeeded, Unknown, Waiting,
    };
    for (outcome, expected) in [
        ("ok", Succeeded),
        ("denied", Failed),
        ("failed", Failed),
        ("unknown", Unknown),
        ("cancelled", Unknown),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let port = FilesystemExecutionCommit::open_with_read_access_hook(
            directory.path(),
            Arc::new(AllowReadAccess),
        )
        .unwrap();
        let (park, settle) = host_capability_projection_commits(outcome);
        assert!(matches!(
            port.commit(park).await,
            ExecutionCommitResult::Committed { .. }
        ));
        assert_eq!(
            inspected_status(port.read_execution(host_node_read("node.host")).unwrap()),
            Waiting
        );
        let live = vec![serde_json::from_value(settle.tuple.observations[0].clone()).unwrap()];
        assert_eq!(
            inspected_status(
                port.read_execution_with_live(host_node_read("node.host"), &live)
                    .unwrap()
            ),
            Waiting,
            "a live provisional settlement must not change committed inspection"
        );
        assert!(matches!(
            port.commit(settle).await,
            ExecutionCommitResult::Committed { .. }
        ));
        assert_eq!(
            inspected_status(port.read_execution(host_node_read("node.host")).unwrap()),
            expected
        );
        assert_eq!(
            inspected_status(port.read_execution(host_node_read("node.later")).unwrap()),
            Running
        );
        drop(port);
        let reopened = FilesystemExecutionCommit::open_with_read_access_hook(
            directory.path(),
            Arc::new(AllowReadAccess),
        )
        .unwrap();
        assert_eq!(
            inspected_status(
                reopened
                    .read_execution(host_node_read("node.host"))
                    .unwrap()
            ),
            expected
        );
        assert_eq!(
            inspected_status(
                reopened
                    .read_execution(host_node_read("node.later"))
                    .unwrap()
            ),
            Running
        );
    }
}

#[test]
fn retained_settlement_repairs_a_legacy_running_node_index_without_crossing_identity() {
    use apxm_runtime_protocol::NodeExecutionStatus::{Running, Succeeded, Waiting};
    let (park, settle) = host_capability_projection_commits("ok");
    let mut store = CommitLocalStore::new();
    store.commit(&park).unwrap();
    store.commit(&settle).unwrap();
    for node in store.node_executions.values_mut() {
        node.status = Running;
    }
    let restored: CommitLocalStore =
        serde_json::from_slice(&serde_json::to_vec(&store).unwrap()).unwrap();
    assert_eq!(
        inspected_status(
            restored
                .read_execution(host_node_read("node.host"), &[7; 32], &AllowReadAccess)
                .unwrap()
        ),
        Succeeded
    );
    assert_eq!(
        inspected_status(
            restored
                .read_execution(host_node_read("node.later"), &[7; 32], &AllowReadAccess)
                .unwrap()
        ),
        Running
    );
    let mut terminal = restored.clone();
    terminal
        .node_executions
        .get_mut("invoke.host-projection:node.host")
        .unwrap()
        .status = apxm_runtime_protocol::NodeExecutionStatus::Failed;
    assert_eq!(
        inspected_status(
            terminal
                .read_execution(host_node_read("node.host"), &[7; 32], &AllowReadAccess)
                .unwrap()
        ),
        apxm_runtime_protocol::NodeExecutionStatus::Failed,
        "retained settlement must not overwrite an existing terminal error"
    );
    let mut missing = restored.clone();
    missing
        .observations
        .get_mut("invoke.host-projection")
        .unwrap()
        .retain(|observation| {
            observation.observation_kind
                != apxm_runtime_protocol::ObservationKind::CapabilitySettled
        });
    assert_eq!(
        inspected_status(
            missing
                .read_execution(host_node_read("node.host"), &[7; 32], &AllowReadAccess)
                .unwrap()
        ),
        Waiting,
        "without retained settlement a legacy index cannot fabricate success"
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
