//! Shared durable record shape for owner-local Execution Commit adapters.
//!
//! One writer mutates one store. Compare-and-commit publishes the whole five-
//! member write set or none of it. Idempotency is keyed by `commit_id`.

use std::collections::{HashMap, HashSet};

use apxm_kernel::{
    ExecutionCommitRequest, ExecutionCommitResult, ExecutionCommitTuple, ProgramInstanceRef,
};
use apxm_program::runtime_evidence::Fact;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// On-disk / in-memory schema identity for owner-local commit records.
pub const COMMIT_LOCAL_SCHEMA: &str = "apxm.execution-commit-local.v1";

/// Maximum serialized tuple bytes accepted by owner-local adapters (8 MiB).
/// Larger payloads fail closed; hosted multi-tenant retention is out of scope.
pub const MAX_TUPLE_BYTES: usize = 8 * 1024 * 1024;

/// Maximum retained idempotent commit results per store (restart / retention bound).
pub const MAX_COMMIT_RESULTS: usize = 10_000;

#[derive(Debug, Error)]
pub enum CommitLocalError {
    #[error("commit-local schema mismatch: expected {COMMIT_LOCAL_SCHEMA}, found {found}")]
    SchemaMismatch { found: String },
    #[error("checkpoint/output tuple exceeds owner-local bound ({MAX_TUPLE_BYTES} bytes)")]
    TupleTooLarge { bytes: usize },
    #[error("commit-local retention bound exceeded ({MAX_COMMIT_RESULTS} commit results)")]
    RetentionExceeded,
    #[error("commit-local I/O error: {0}")]
    Io(String),
    #[error("commit-local encode/decode error: {0}")]
    Codec(String),
}

/// One authoritative instance record after a successful atomic commit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitLocalRecord {
    pub program_state_version: u64,
    pub write_set: apxm_kernel::AtomicWriteSet,
    pub continuation: Option<Value>,
    pub event_wait: Option<Value>,
    pub context: Value,
    pub evidence: Vec<Fact>,
    pub evidence_batch: Vec<Fact>,
    pub usage: Value,
    pub output_refs: Vec<Value>,
    pub effect_outcomes: Vec<Value>,
    pub last_commit_id: String,
    pub last_evidence_position_ref: String,
}

/// Idempotent commit result retained for replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredCommit {
    pub result: StoredCommitResult,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoredCommitResult {
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

impl From<&ExecutionCommitResult> for StoredCommitResult {
    fn from(result: &ExecutionCommitResult) -> Self {
        match result {
            ExecutionCommitResult::Committed {
                new_program_state_version,
                evidence_position_ref,
            } => Self::Committed {
                new_program_state_version: *new_program_state_version,
                evidence_position_ref: evidence_position_ref.clone(),
            },
            ExecutionCommitResult::CompareConflict {
                current_program_state_version,
            } => Self::CompareConflict {
                current_program_state_version: *current_program_state_version,
            },
            ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => Self::OutcomeUnknown {
                reconciliation_ref: reconciliation_ref.clone(),
            },
        }
    }
}

impl From<&StoredCommitResult> for ExecutionCommitResult {
    fn from(result: &StoredCommitResult) -> Self {
        match result {
            StoredCommitResult::Committed {
                new_program_state_version,
                evidence_position_ref,
            } => Self::Committed {
                new_program_state_version: *new_program_state_version,
                evidence_position_ref: evidence_position_ref.clone(),
            },
            StoredCommitResult::CompareConflict {
                current_program_state_version,
            } => Self::CompareConflict {
                current_program_state_version: *current_program_state_version,
            },
            StoredCommitResult::OutcomeUnknown { reconciliation_ref } => Self::OutcomeUnknown {
                reconciliation_ref: reconciliation_ref.clone(),
            },
        }
    }
}

/// Portable owner-local store body (memory and filesystem share this shape).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommitLocalStore {
    pub schema_version: String,
    pub instances: HashMap<String, CommitLocalRecord>,
    pub by_commit_id: HashMap<String, StoredCommit>,
    /// Commit ids forced to `outcome_unknown` without writing (failure injection).
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub force_unknown: HashSet<String>,
}

impl CommitLocalStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema_version: COMMIT_LOCAL_SCHEMA.to_string(),
            instances: HashMap::new(),
            by_commit_id: HashMap::new(),
            force_unknown: HashSet::new(),
        }
    }

    pub fn validate_schema(&self) -> Result<(), CommitLocalError> {
        if self.schema_version != COMMIT_LOCAL_SCHEMA {
            return Err(CommitLocalError::SchemaMismatch {
                found: self.schema_version.clone(),
            });
        }
        Ok(())
    }

    pub fn inject_outcome_unknown(&mut self, commit_id: impl Into<String>) {
        self.force_unknown.insert(commit_id.into());
    }

    pub fn current_version(&self, program_instance_ref: &ProgramInstanceRef) -> u64 {
        self.instances
            .get(program_instance_ref.as_str())
            .map(|r| r.program_state_version)
            .unwrap_or(0)
    }

    pub fn load_continuation(&self, program_instance_ref: &ProgramInstanceRef) -> Option<Value> {
        self.instances
            .get(program_instance_ref.as_str())
            .and_then(|r| r.continuation.clone())
    }

    /// Apply one compare-and-commit against this store. Writes all members or none.
    pub fn commit(
        &mut self,
        request: &ExecutionCommitRequest,
    ) -> Result<ExecutionCommitResult, CommitLocalError> {
        self.validate_schema()?;
        enforce_tuple_bound(&request.tuple)?;

        if let Some(prior) = self.by_commit_id.get(&request.commit_id) {
            return Ok(ExecutionCommitResult::from(&prior.result));
        }

        if self.force_unknown.contains(&request.commit_id) {
            let result = ExecutionCommitResult::OutcomeUnknown {
                reconciliation_ref: format!("reconcile:{}", request.commit_id),
            };
            self.retain_result(&request.commit_id, &result)?;
            return Ok(result);
        }

        let current = self.current_version(&request.program_instance_ref);
        if request.expected_program_state_version != current {
            let result = ExecutionCommitResult::CompareConflict {
                current_program_state_version: current,
            };
            self.retain_result(&request.commit_id, &result)?;
            return Ok(result);
        }

        let new_version = current + 1;
        let evidence_position_ref = format!(
            "evidence:{}:{new_version}",
            request.program_invocation_ref.as_str()
        );
        let record = CommitLocalRecord {
            program_state_version: new_version,
            write_set: request.write_set.clone(),
            continuation: request.tuple.continuation.clone(),
            event_wait: request.tuple.event_wait.clone(),
            context: request.tuple.context.clone(),
            evidence: request.tuple.evidence.clone(),
            evidence_batch: request.evidence_batch.clone(),
            usage: request.tuple.usage.clone(),
            output_refs: request.tuple.output_refs.clone(),
            effect_outcomes: request.tuple.effect_outcomes.clone(),
            last_commit_id: request.commit_id.clone(),
            last_evidence_position_ref: evidence_position_ref.clone(),
        };

        // Stage then publish: no partial instance update without the idempotency row.
        let result = ExecutionCommitResult::Committed {
            new_program_state_version: new_version,
            evidence_position_ref,
        };
        self.ensure_retention_capacity(&request.commit_id)?;
        self.instances
            .insert(request.program_instance_ref.as_str().to_string(), record);
        self.retain_result(&request.commit_id, &result)?;
        Ok(result)
    }

    fn ensure_retention_capacity(&self, commit_id: &str) -> Result<(), CommitLocalError> {
        if self.by_commit_id.len() >= MAX_COMMIT_RESULTS
            && !self.by_commit_id.contains_key(commit_id)
        {
            return Err(CommitLocalError::RetentionExceeded);
        }
        Ok(())
    }

    fn retain_result(
        &mut self,
        commit_id: &str,
        result: &ExecutionCommitResult,
    ) -> Result<(), CommitLocalError> {
        self.ensure_retention_capacity(commit_id)?;
        self.by_commit_id.insert(
            commit_id.to_string(),
            StoredCommit {
                result: StoredCommitResult::from(result),
            },
        );
        Ok(())
    }
}

fn enforce_tuple_bound(tuple: &ExecutionCommitTuple) -> Result<(), CommitLocalError> {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "context": tuple.context,
        "continuation": tuple.continuation,
        "event_wait": tuple.event_wait,
        "effect_outcomes": tuple.effect_outcomes,
        "evidence": tuple.evidence,
        "usage": tuple.usage,
        "output_refs": tuple.output_refs,
    }))
    .map_err(|e| CommitLocalError::Codec(e.to_string()))?
    .len();
    if bytes > MAX_TUPLE_BYTES {
        return Err(CommitLocalError::TupleTooLarge { bytes });
    }
    Ok(())
}
