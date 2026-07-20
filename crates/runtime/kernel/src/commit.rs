//! The atomic Execution Commit port — the only commit boundary.
//!
//! One compare-and-commit publishes the exact five-member write set —
//! state/continuation, checkpoint/effect outcomes, canonical evidence, usage,
//! and immutable session-output refs — atomically or publishes none of it.
//! There is no separate state, checkpoint, effect-journal, evidence, usage, or
//! output-ref commit path. A version mismatch is `compare_conflict` and writes
//! nothing; an ambiguous backend failure is `outcome_unknown` and writes
//! nothing. This port is defined here and implemented by injected adapters; the
//! shape is kept consistent with the runtime-side adapter crate.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use apxm_program::runtime_evidence::Fact;

/// The exact ordered atomic write-set members. This is the only legal write set.
pub const ATOMIC_WRITE_SET: [&str; 5] = [
    "state_continuation",
    "checkpoint_effect_outcomes",
    "runtime_evidence",
    "usage_facts",
    "session_output_refs",
];

/// The six digest members published by one atomic commit. Every member is
/// required at the type level, so a partial/split write set cannot be built.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomicWriteSet {
    pub next_program_state_digest: String,
    pub continuation_digest: String,
    pub checkpoint_effect_outcomes_digest: String,
    pub runtime_evidence_batch_digest: String,
    pub usage_facts_digest: String,
    pub session_output_refs_digest: String,
}

/// One atomic Execution Commit request. Carries the full write set, the
/// canonical evidence batch published by that write set, and the expected
/// program-state version for the compare-and-commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionCommitRequest {
    pub commit_id: String,
    pub invocation_ref: String,
    pub idempotency_key: String,
    pub expected_program_state_version: u64,
    pub write_set: AtomicWriteSet,
    /// The runtime-evidence facts published atomically with this commit; their
    /// content is summarized by `write_set.runtime_evidence_batch_digest`.
    pub evidence_batch: Vec<Fact>,
}

/// The typed result of an atomic commit. Mirrors the closed
/// `apxm.execution-commit.v1` `commit_result` set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionCommitResult {
    Committed {
        new_program_state_version: u64,
        evidence_position_ref: String,
    },
    CompareConflict {
        current_program_state_version: u64,
    },
    OutcomeUnknown {
        reconciliation_ref: String,
    },
}

impl ExecutionCommitResult {
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Committed { .. } => "committed",
            Self::CompareConflict { .. } => "compare_conflict",
            Self::OutcomeUnknown { .. } => "outcome_unknown",
        }
    }
}

impl ExecutionCommitRequest {
    /// Project this request and its result as an exact `apxm.execution-commit.v1`
    /// object, including the constant atomic write set. No per-member split field
    /// is ever emitted.
    #[must_use]
    pub fn to_contract_json(&self, result: &ExecutionCommitResult) -> Value {
        let mut object = json!({
            "schema_version": "apxm.execution-commit.v1",
            "commit_id": self.commit_id,
            "invocation_ref": { "ref_type": "ProgramInvocationRef", "ref": self.invocation_ref },
            "idempotency_key": {
                "key_id": self.idempotency_key,
                "scope_ref": self.invocation_ref,
                "request_digest": self.write_set.next_program_state_digest,
            },
            "expected_program_state_version": self.expected_program_state_version,
            "atomic_write_set": ATOMIC_WRITE_SET,
            "next_program_state_digest": self.write_set.next_program_state_digest,
            "continuation_digest": self.write_set.continuation_digest,
            "checkpoint_effect_outcomes_digest": self.write_set.checkpoint_effect_outcomes_digest,
            "runtime_evidence_batch_digest": self.write_set.runtime_evidence_batch_digest,
            "usage_facts_digest": self.write_set.usage_facts_digest,
            "session_output_refs_digest": self.write_set.session_output_refs_digest,
            "commit_result": result.label(),
        });
        let map = object.as_object_mut().expect("contract object");
        match result {
            ExecutionCommitResult::Committed {
                new_program_state_version,
                evidence_position_ref,
            } => {
                map.insert(
                    "new_program_state_version".into(),
                    json!(new_program_state_version),
                );
                map.insert(
                    "evidence_position_ref".into(),
                    json!({ "ref_type": "EvidencePositionRef", "ref": evidence_position_ref }),
                );
            }
            ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => {
                map.insert(
                    "reconciliation_ref".into(),
                    json!({ "ref_type": "ReconciliationRef", "ref": reconciliation_ref }),
                );
            }
            ExecutionCommitResult::CompareConflict { .. } => {}
        }
        object
    }
}

/// The atomic Execution Commit port. Exactly one implementation commits Program
/// state; there is no split commit surface.
#[async_trait]
pub trait ExecutionCommitPort: Send + Sync {
    /// Compare `expected_program_state_version` against the invocation's current
    /// version and, only on an exact match, atomically publish the whole write
    /// set and its evidence batch. Idempotent on `commit_id`.
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult;

    /// The current committed version for an invocation (0 before any commit).
    async fn current_version(&self, invocation_ref: &str) -> u64;
}
