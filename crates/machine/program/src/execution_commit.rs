//! `apxm.execution-commit.v1` — closed consumer types and verification.
//!
//! One atomic idempotent compare-and-commit publishes the exact five-member
//! write set — state/continuation, checkpoint/effect outcomes, canonical
//! evidence, usage, and immutable session-output refs — or publishes none of it.
//! There is no split or partial commit path: the write set must equal the
//! canonical five members in order, any per-member split field is an unknown
//! field, and the commit result is a closed set.

use serde::{Deserialize, Serialize};

use crate::common::{IdempotencyKey, TypedRef};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::grammar::is_digest;

/// The single accepted `schema_version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionCommitVersion {
    #[serde(rename = "apxm.execution-commit.v1")]
    V1,
}

/// The closed members of the atomic write set, in canonical order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomicWriteSetMember {
    StateContinuation,
    CheckpointEffectOutcomes,
    RuntimeEvidence,
    UsageFacts,
    SessionOutputRefs,
}

/// The exact ordered atomic write set. The commit publishes all five or none.
pub const CANONICAL_ATOMIC_WRITE_SET: [AtomicWriteSetMember; 5] = [
    AtomicWriteSetMember::StateContinuation,
    AtomicWriteSetMember::CheckpointEffectOutcomes,
    AtomicWriteSetMember::RuntimeEvidence,
    AtomicWriteSetMember::UsageFacts,
    AtomicWriteSetMember::SessionOutputRefs,
];

/// The closed commit-result set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitResult {
    Committed,
    CompareConflict,
    OutcomeUnknown,
}

/// A decoded atomic execution commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCommit {
    pub schema_version: ExecutionCommitVersion,
    pub commit_id: String,
    pub invocation_ref: TypedRef,
    pub idempotency_key: IdempotencyKey,
    pub expected_program_state_version: u64,
    pub atomic_write_set: Vec<AtomicWriteSetMember>,
    pub next_program_state_digest: String,
    pub continuation_digest: String,
    pub checkpoint_effect_outcomes_digest: String,
    pub runtime_evidence_batch_digest: String,
    pub usage_facts_digest: String,
    pub session_output_refs_digest: String,
    pub commit_result: CommitResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_program_state_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_position_ref: Option<TypedRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciliation_ref: Option<TypedRef>,
}

impl ExecutionCommit {
    /// Verify a decoded commit: the write set is exactly the canonical five in
    /// order (no split/partial path), and every digest is well-formed.
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();

        if self.atomic_write_set != CANONICAL_ATOMIC_WRITE_SET {
            verdict.push(Diagnostic::new(
                DiagnosticCode::NonAtomicWriteSet,
                "atomic_write_set",
                "the atomic write set must be exactly the canonical five members in order",
            ));
        }

        for (location, digest) in [
            ("next_program_state_digest", &self.next_program_state_digest),
            ("continuation_digest", &self.continuation_digest),
            (
                "checkpoint_effect_outcomes_digest",
                &self.checkpoint_effect_outcomes_digest,
            ),
            (
                "runtime_evidence_batch_digest",
                &self.runtime_evidence_batch_digest,
            ),
            ("usage_facts_digest", &self.usage_facts_digest),
            ("session_output_refs_digest", &self.session_output_refs_digest),
        ] {
            if !is_digest(digest) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::InvalidDigest,
                    location,
                    "digest is not a lowercase sha256 value",
                ));
            }
        }

        verdict.finish()
    }
}

/// Verify an execution commit presented as JSON, failing closed on decode
/// errors. A split-commit per-member field is an unknown field and is rejected
/// here; an unknown commit result discriminant is rejected at decode.
#[must_use]
pub fn verify_execution_commit_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<ExecutionCommit>(value.clone()) {
        Ok(commit) => commit.verify(),
        Err(error) => schema_violation("execution_commit", &error),
    }
}
