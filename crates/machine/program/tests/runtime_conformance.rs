//! Conformance: the runtime-evidence and execution-commit verifiers accept and
//! reject exactly the checked-in vectors.

mod common;

use apxm_program::{
    Fact, verify_execution_commit_json, verify_external_agent_evidence_json,
    verify_external_agent_session_json, verify_runtime_evidence_json,
};
use common::{Vector, load_vectors};
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
    check("apxm.runtime-evidence.json", |v| {
        verify_runtime_evidence_json(v).is_accepted()
    });
}

#[test]
fn execution_commit_vectors_match_verifier() {
    check("apxm.execution-commit.json", |v| {
        verify_execution_commit_json(v).is_accepted()
    });
}

#[test]
fn external_agent_session_vectors_match_verifier() {
    check("apxm.external-agent-session.json", |v| {
        verify_external_agent_session_json(v).is_accepted()
    });
}

#[test]
fn external_agent_evidence_vectors_match_verifier() {
    check("apxm.external-agent-evidence.json", |v| {
        verify_external_agent_evidence_json(v).is_accepted()
    });
}

#[test]
fn loop_iteration_completion_deserializes_as_required_closed_variant() {
    let fact: Fact = serde_json::from_value(serde_json::json!({
        "fact_id": "loop-iteration.1",
        "event_sequence": 2,
        "fact_kind": "LoopIterationCompleted",
        "static_loop_id": "loop.1",
        "loop_occurrence_id": "loop-occurrence.1",
        "iteration_index": 0,
        "program_invocation_id": "invocation.1",
        "causal_node_execution_ids": ["node-execution.1"]
    }))
    .expect("typed completion");
    let Fact::LoopIterationCompleted(completed) = fact else {
        panic!("completion decoded as a broad runtime fact");
    };
    assert_eq!(completed.iteration_index, 0);
    assert_eq!(completed.causal_node_execution_ids, ["node-execution.1"]);
}
