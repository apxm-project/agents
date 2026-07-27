//! The runtime's atomic commit projects a valid `apxm.execution-commit.v1`
//! object — the exact five-member write set, no split field — accepted by the
//! owned contract verifier.

use apxm_kernel::{
    AtomicWriteSet, ExecutionCommitRequest, ExecutionCommitResult, ExecutionCommitTuple,
    ProgramInstanceRef, ProgramInvocationRef,
};
use apxm_program::verify_execution_commit_json;

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
            runtime_evidence_batch_digest: digest('4'),
            usage_facts_digest: digest('5'),
            session_output_refs_digest: digest('6'),
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
