//! The runtime's atomic commit projects a valid `apxm.execution-commit`
//! object — the exact five-member write set, no split field — accepted by the
//! owned contract verifier.

use apxm_kernel::{
    AtomicWriteSet, ExecutionCommitRequest, ExecutionCommitResult, ExecutionCommitTuple,
    PrecommitEvidenceRef, ProgramInstanceRef, ProgramInvocationRef,
};
use apxm_program::runtime_evidence::{Fact, FactKind, RuntimeFact};
use apxm_program::verify_execution_commit_json;
use serde_json::json;

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn request() -> ExecutionCommitRequest {
    ExecutionCommitRequest {
        commit_id: "commit.1".into(),
        program_instance_ref: ProgramInstanceRef::new("instance.1"),
        program_invocation_ref: ProgramInvocationRef::new("invoke.1"),
        idempotency_key: "idem.commit.1".into(),
        expected_program_state_version: 7,
        write_set: AtomicWriteSet {
            next_program_state_digest: digest('1'),
            continuation_digest: digest('2'),
            checkpoint_effect_outcomes_digest: digest('3'),
            runtime_evidence_batch_digest: apxm_kernel::runtime_evidence_and_observation_digest(
                &[],
                &[],
            ),
            usage_facts_digest: digest('5'),
            session_output_refs_digest: apxm_kernel::session_output_refs_digest(&[]),
        },
        tuple: ExecutionCommitTuple::empty(vec![]),
        evidence_batch: vec![],
    }
}

#[test]
fn committed_projection_is_a_valid_contract() {
    let json = request().to_contract_json(&ExecutionCommitResult::Committed {
        new_program_state_version: 8,
        evidence_position_ref: "evidence:invoke.1:8".into(),
    });
    assert!(verify_execution_commit_json(&json).is_accepted());
    assert!(
        json.get("state_commit_ref").is_none(),
        "no per-member split field"
    );
    assert_eq!(
        json["program_instance_ref"]["ref_type"],
        "ProgramInstanceRef"
    );
    assert_eq!(json["invocation_ref"]["ref_type"], "ProgramInvocationRef");
    let mut collapsed = json;
    collapsed["invocation_ref"]["ref_type"] = "ProgramInstanceRef".into();
    assert!(
        !verify_execution_commit_json(&collapsed).is_accepted(),
        "Program Invocation evidence identity cannot collapse into the Program Instance CAS key"
    );
}

#[test]
fn conflict_and_unknown_projections_are_valid_contracts() {
    let conflict = request().to_contract_json(&ExecutionCommitResult::CompareConflict {
        current_program_state_version: 5,
    });
    assert!(verify_execution_commit_json(&conflict).is_accepted());

    let unknown = request().to_contract_json(&ExecutionCommitResult::OutcomeUnknown {
        reconciliation_ref: "reconcile:commit.1".into(),
    });
    assert!(verify_execution_commit_json(&unknown).is_accepted());
}

#[test]
fn atomic_request_rejects_split_evidence_and_malformed_write_members() {
    let mut split = request();
    let fact = serde_json::from_value::<RuntimeFact>(serde_json::json!({
        "fact_id": "fact.1",
        "event_sequence": 1
    }))
    .expect("minimal runtime fact");
    split.evidence_batch = vec![Fact::from_runtime(FactKind::InstanceCreated, fact)];
    assert!(matches!(
        split.validate(),
        Err(apxm_kernel::CommitRequestError::EvidenceBatchMismatch)
    ));

    let mut malformed = request();
    malformed.write_set.usage_facts_digest = "not-a-digest".into();
    assert!(matches!(
        malformed.validate(),
        Err(apxm_kernel::CommitRequestError::InvalidDigest(
            "usage_facts_digest"
        ))
    ));
}

#[test]
fn atomic_request_binds_driver_observations_to_the_evidence_digest() {
    let mut observation_request = request();
    observation_request.tuple.observations = vec![json!({
        "contract": "apxm.execution-observation.v1",
        "observation_id": "observation.invoke.1.1",
        "program_invocation_id": "invoke.1",
        "sequence": 1,
        "cursor": {"position": 1, "token": "cursor.1"},
        "timing": {"observed_at_unix_ms": 1},
        "observation_kind": "invocation_started",
        "commitment": "provisional"
    })];
    observation_request.write_set.runtime_evidence_batch_digest =
        apxm_kernel::runtime_evidence_and_observation_digest(
            &observation_request.tuple.evidence,
            &observation_request.tuple.observations,
        );
    assert!(observation_request.validate().is_ok());

    observation_request.tuple.observations[0]["sequence"] = json!(2);
    assert!(matches!(
        observation_request.validate(),
        Err(apxm_kernel::CommitRequestError::EvidenceObservationDigestMismatch { .. })
    ));

    let mut output_tampered = request();
    assert!(output_tampered.validate().is_ok());
    output_tampered
        .tuple
        .output_refs
        .push(json!({"ref": "out.1"}));
    assert!(matches!(
        output_tampered.validate(),
        Err(apxm_kernel::CommitRequestError::SessionOutputRefsDigestMismatch { .. })
    ));
}

#[test]
fn atomic_request_rejects_malformed_typed_observations_before_adapter() {
    let mut malformed = request();
    malformed.tuple.observations = vec![json!({
        "contract": "apxm.execution-observation.v1",
        "observation_id": "observation.invoke.1.1",
        "program_invocation_id": "invoke.1",
        "sequence": 1,
        "cursor": {"position": 1, "token": "cursor.1"},
        "timing": {"observed_at_unix_ms": 1},
        "observation_kind": "not_a_kind",
        "commitment": "provisional"
    })];
    malformed.write_set.runtime_evidence_batch_digest =
        apxm_kernel::runtime_evidence_and_observation_digest(
            &malformed.tuple.evidence,
            &malformed.tuple.observations,
        );
    assert!(matches!(
        malformed.validate(),
        Err(apxm_kernel::CommitRequestError::InvalidObservation(_))
    ));
}

#[test]
fn precommit_evidence_refs_are_deterministic_and_tamper_evident() {
    let first = PrecommitEvidenceRef::new("commit.1", "invoke.1", 0).expect("first ref");
    assert_eq!(first.as_str(), "evidence.invoke.1.commit.1.1");
    first.validate().expect("valid first ref");
    let second = PrecommitEvidenceRef::new("commit.1", "invoke.1", 1).expect("second ref");
    assert_eq!(second.as_str(), "evidence.invoke.1.commit.1.2");
    assert_ne!(first, second);

    let mut forged = first;
    forged.evidence_ref = "evidence.invoke.1.commit.other.1".into();
    assert!(forged.validate().is_err());
}
