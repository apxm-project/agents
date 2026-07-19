//! Conformance: the runtime-evidence and execution-commit verifiers accept and
//! reject exactly the checked-in vectors.

mod common;

use apxm_program::{verify_execution_commit_json, verify_runtime_evidence_json};
use common::{load_vectors, Vector};
use serde_json::Value;

fn check(file: &str, verify: impl Fn(&Value) -> bool) {
    for Vector {
        name,
        input,
        expected_valid,
    } in load_vectors(file)
    {
        let accepted = verify(&input);
        assert_eq!(
            accepted, expected_valid,
            "{file}: vector '{name}' expected valid={expected_valid} but verifier returned {accepted}",
        );
    }
}

#[test]
fn runtime_evidence_vectors_match_verifier() {
    check("apxm.runtime-evidence.v1.json", |v| {
        verify_runtime_evidence_json(v).is_accepted()
    });
}

#[test]
fn execution_commit_vectors_match_verifier() {
    check("apxm.execution-commit.v1.json", |v| {
        verify_execution_commit_json(v).is_accepted()
    });
}
