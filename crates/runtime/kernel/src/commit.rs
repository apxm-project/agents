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

use apxm_program::grammar::is_digest;
use apxm_program::runtime_evidence::Fact;

/// The durable Program Instance identity that scopes compare-and-commit state
/// and continuation reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProgramInstanceRef(String);

impl ProgramInstanceRef {
    /// Construct one exact Program Instance reference.
    #[must_use]
    pub fn new(reference: impl Into<String>) -> Self {
        Self(reference.into())
    }

    /// Borrow the exact reference value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ProgramInstanceRef {
    fn from(reference: String) -> Self {
        Self::new(reference)
    }
}

impl From<&str> for ProgramInstanceRef {
    fn from(reference: &str) -> Self {
        Self::new(reference)
    }
}

/// The admitted Program Invocation identity that scopes one committed evidence
/// and idempotency record.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProgramInvocationRef(String);

impl ProgramInvocationRef {
    /// Construct one exact Program Invocation reference.
    #[must_use]
    pub fn new(reference: impl Into<String>) -> Self {
        Self(reference.into())
    }

    /// Borrow the exact reference value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ProgramInvocationRef {
    fn from(reference: String) -> Self {
        Self::new(reference)
    }
}

impl From<&str> for ProgramInvocationRef {
    fn from(reference: &str) -> Self {
        Self::new(reference)
    }
}

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

/// The exact runtime values represented by one atomic write set.
///
/// The commit adapter publishes this tuple as one compare-and-commit unit. The
/// opaque continuation payload is runtime-owned serialized state; it is read
/// back only through this same port on a later invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionCommitTuple {
    pub context: Value,
    pub continuation: Option<Value>,
    pub event_wait: Option<Value>,
    pub effect_outcomes: Vec<Value>,
    pub evidence: Vec<Fact>,
    pub usage: Value,
    pub output_refs: Vec<Value>,
}

impl ExecutionCommitTuple {
    /// Build an empty execution tuple with the supplied authoritative evidence.
    #[must_use]
    pub fn empty(evidence: Vec<Fact>) -> Self {
        Self {
            context: Value::Null,
            continuation: None,
            event_wait: None,
            effect_outcomes: Vec::new(),
            evidence,
            usage: Value::Null,
            output_refs: Vec::new(),
        }
    }
}

/// One atomic Execution Commit request. Carries the full write set, the
/// canonical evidence batch published by that write set, and the expected
/// program-state version for the compare-and-commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionCommitRequest {
    pub commit_id: String,
    pub program_instance_ref: ProgramInstanceRef,
    pub program_invocation_ref: ProgramInvocationRef,
    pub idempotency_key: String,
    pub expected_program_state_version: u64,
    pub write_set: AtomicWriteSet,
    /// The full state/effect/evidence tuple whose digests appear in `write_set`.
    pub tuple: ExecutionCommitTuple,
    /// The runtime-evidence facts published atomically with this commit; their
    /// content is summarized by `write_set.runtime_evidence_batch_digest`.
    pub evidence_batch: Vec<Fact>,
}

/// Why a commit request cannot cross the single atomic boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitRequestError {
    EmptyField(&'static str),
    InvalidDigest(&'static str),
    EvidenceBatchMismatch,
}

impl std::fmt::Display for CommitRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "commit field {field} must be non-empty"),
            Self::InvalidDigest(field) => write!(f, "commit field {field} is not a sha256 digest"),
            Self::EvidenceBatchMismatch => {
                f.write_str("tuple evidence and evidence_batch must be identical")
            }
        }
    }
}

impl std::error::Error for CommitRequestError {}

impl ExecutionCommitRequest {
    /// Validate the complete request before an adapter is called. The tuple and
    /// evidence batch are one atomic member; accepting divergent copies would
    /// create a second, split evidence path.
    pub fn validate(&self) -> Result<(), CommitRequestError> {
        for (field, value) in [
            ("commit_id", self.commit_id.as_str()),
            ("program_instance_ref", self.program_instance_ref.as_str()),
            (
                "program_invocation_ref",
                self.program_invocation_ref.as_str(),
            ),
            ("idempotency_key", self.idempotency_key.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(CommitRequestError::EmptyField(field));
            }
        }
        for (field, value) in [
            (
                "next_program_state_digest",
                self.write_set.next_program_state_digest.as_str(),
            ),
            (
                "continuation_digest",
                self.write_set.continuation_digest.as_str(),
            ),
            (
                "checkpoint_effect_outcomes_digest",
                self.write_set.checkpoint_effect_outcomes_digest.as_str(),
            ),
            (
                "runtime_evidence_batch_digest",
                self.write_set.runtime_evidence_batch_digest.as_str(),
            ),
            (
                "usage_facts_digest",
                self.write_set.usage_facts_digest.as_str(),
            ),
            (
                "session_output_refs_digest",
                self.write_set.session_output_refs_digest.as_str(),
            ),
        ] {
            if !is_digest(value) {
                return Err(CommitRequestError::InvalidDigest(field));
            }
        }
        if self.tuple.evidence != self.evidence_batch {
            return Err(CommitRequestError::EvidenceBatchMismatch);
        }
        Ok(())
    }
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
            "program_instance_ref": {
                "ref_type": "ProgramInstanceRef",
                "ref": self.program_instance_ref.as_str(),
            },
            "invocation_ref": {
                "ref_type": "ProgramInvocationRef",
                "ref": self.program_invocation_ref.as_str(),
            },
            "idempotency_key": {
                "key_id": self.idempotency_key,
                "scope_ref": self.program_invocation_ref.as_str(),
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
    /// Compare `expected_program_state_version` against the Program Instance's
    /// current version and, only on an exact match, atomically publish the
    /// whole write set and its evidence batch. Idempotency remains scoped to
    /// the admitted Program Invocation carried by the request.
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult;

    /// The current committed version for one Program Instance (0 before any
    /// commit).
    async fn current_version(&self, program_instance_ref: &ProgramInstanceRef) -> u64;

    /// Read the current continuation payload from the same authoritative commit
    /// record, or `None` when this Program Instance has no committed
    /// continuation.
    ///
    /// This is a required method rather than a defaulted one: an implementation
    /// that silently answered `None` would make a resumable Program look
    /// permanently uncommitted while its continuation sat durable behind a
    /// second persistence authority. An implementation that genuinely holds no
    /// continuation states that explicitly.
    async fn load_continuation(&self, program_instance_ref: &ProgramInstanceRef) -> Option<Value>;
}
