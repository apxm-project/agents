//! Shared durable record shape for owner-local Execution Commit adapters.
//!
//! One writer mutates one store. Compare-and-commit publishes the whole five-
//! member write set or none of it. Replay identity is scoped to one commit,
//! Program Instance, and Program Invocation.

use std::collections::{HashMap, HashSet};

use apxm_kernel::{
    CommittedContinuation, ExecutionCommitRequest, ExecutionCommitResult, ExecutionCommitTuple,
    ProgramInstanceRef, ProgramInvocationRef,
};
use apxm_program::runtime_evidence::Fact;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// On-disk / in-memory schema identity for owner-local commit records.
pub const COMMIT_LOCAL_SCHEMA: &str = "apxm.execution-commit-local.v2";

/// Maximum serialized tuple bytes accepted by owner-local adapters (8 MiB).
/// Larger payloads fail closed; hosted multi-tenant retention is out of scope.
pub const MAX_TUPLE_BYTES: usize = 8 * 1024 * 1024;

/// Maximum serialized owner-local store accepted during restart recovery.
/// Persisted state is local, but a damaged or unexpectedly replaced file must
/// not turn recovery into an unbounded allocation before JSON validation.
pub const MAX_STORE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum retained idempotent commit results per store (restart / retention bound).
pub const MAX_COMMIT_RESULTS: usize = 10_000;

/// Maximum bytes accepted by the owner-local Session Output preparation path.
pub const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Maximum prepared plus committed output records retained by one local store.
pub const MAX_OUTPUT_RECORDS: usize = 10_000;

const LOCAL_OUTPUT_REF_PREFIX: &str = "apxm.local-session-output/";

#[derive(Debug, Error)]
pub enum CommitLocalError {
    #[error("commit-local schema mismatch: expected {COMMIT_LOCAL_SCHEMA}, found {found}")]
    SchemaMismatch { found: String },
    #[error("checkpoint/output tuple exceeds owner-local bound ({MAX_TUPLE_BYTES} bytes)")]
    TupleTooLarge { bytes: usize },
    /// The persisted owner-local store exceeds the recovery allocation bound.
    #[error("commit-local store exceeds owner-local bound ({bytes} bytes)")]
    StoreTooLarge { bytes: u64 },
    #[error("commit-local retention bound exceeded ({MAX_COMMIT_RESULTS} commit results)")]
    RetentionExceeded,
    #[error("commit-local request is invalid: {0}")]
    InvalidRequest(String),
    #[error("commit-local replay conflicts with the existing request for commit {commit_id}")]
    ConflictingReplay { commit_id: String },
    #[error("commit-local filesystem directory is already owned by another process")]
    OwnershipContended,
    #[error("commit-local I/O error: {0}")]
    Io(String),
    #[error("commit-local encode/decode error: {0}")]
    Codec(String),
    #[error("commit-local persistence authentication failed: {0}")]
    AuthenticationFailed(String),
    #[error("session output exceeds owner-local bound ({MAX_OUTPUT_BYTES} bytes)")]
    OutputTooLarge { bytes: usize },
    #[error("session output field is invalid: {0}")]
    InvalidOutput(String),
    #[error("session output {output_ref} was not prepared for this commit")]
    OutputNotPrepared { output_ref: String },
    #[error("session output {output_ref} belongs to a different commit scope")]
    OutputScopeMismatch { output_ref: String },
    #[error("session output {output_ref} is already committed")]
    OutputAlreadyCommitted { output_ref: String },
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
    pub request_identity: CommitRequestIdentity,
    pub result: StoredCommitResult,
}

/// A typed, digest-addressed Session Output reference prepared by an
/// owner-local composition root. Its bytes remain non-authoritative until the
/// corresponding Execution Commit succeeds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedOutputRef {
    pub output_ref: String,
    pub content_digest: String,
    pub byte_length: usize,
    pub media_type: String,
}

impl PreparedOutputRef {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "ref_type": "SessionOutputRef",
            "ref": self.output_ref,
            "content_digest": self.content_digest,
            "byte_length": self.byte_length,
            "media_type": self.media_type,
        })
    }
}

/// Input to the owner-local Session Output preparation boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionOutputPreparation {
    pub commit_id: String,
    pub program_instance_ref: ProgramInstanceRef,
    pub program_invocation_ref: ProgramInvocationRef,
    pub content: Vec<u8>,
    pub media_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredOutput {
    pub prepared: PreparedOutputRef,
    pub commit_id: String,
    pub program_instance_ref: String,
    pub program_invocation_ref: String,
    pub content: Vec<u8>,
}

/// Request dimensions that must remain identical for an idempotent replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitRequestIdentity {
    pub commit_id: String,
    pub program_instance_ref: String,
    pub program_invocation_ref: String,
    pub idempotency_key: String,
    pub expected_program_state_version: u64,
    pub write_set: apxm_kernel::AtomicWriteSet,
    pub tuple: CommitLocalTuple,
    pub evidence_batch: Vec<Fact>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitLocalTuple {
    pub context: Value,
    pub continuation: Option<Value>,
    pub event_wait: Option<Value>,
    pub effect_outcomes: Vec<Value>,
    pub evidence: Vec<Fact>,
    pub usage: Value,
    pub output_refs: Vec<Value>,
}

impl From<&ExecutionCommitRequest> for CommitRequestIdentity {
    fn from(request: &ExecutionCommitRequest) -> Self {
        Self {
            commit_id: request.commit_id.clone(),
            program_instance_ref: request.program_instance_ref.as_str().to_string(),
            program_invocation_ref: request.program_invocation_ref.as_str().to_string(),
            idempotency_key: request.idempotency_key.clone(),
            expected_program_state_version: request.expected_program_state_version,
            write_set: request.write_set.clone(),
            tuple: CommitLocalTuple {
                context: request.tuple.context.clone(),
                continuation: request.tuple.continuation.clone(),
                event_wait: request.tuple.event_wait.clone(),
                effect_outcomes: request.tuple.effect_outcomes.clone(),
                evidence: request.tuple.evidence.clone(),
                usage: request.tuple.usage.clone(),
                output_refs: request.tuple.output_refs.clone(),
            },
            evidence_batch: request.evidence_batch.clone(),
        }
    }
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
    /// HMAC over the canonical store body with this field empty. The key is
    /// held outside the JSON record by the filesystem owner-local adapter.
    #[serde(default)]
    pub integrity_tag: String,
    pub instances: HashMap<String, CommitLocalRecord>,
    pub by_commit_scope: HashMap<String, StoredCommit>,
    /// Prepared bytes are durable but not visible until their commit wins.
    #[serde(default)]
    pub prepared_outputs: HashMap<String, StoredOutput>,
    /// Only successfully committed output refs are readable as Session Output.
    #[serde(default)]
    pub committed_outputs: HashMap<String, StoredOutput>,
    /// Commit ids forced to `outcome_unknown` without writing (failure injection).
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub force_unknown: HashSet<String>,
}

impl CommitLocalStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema_version: COMMIT_LOCAL_SCHEMA.to_string(),
            integrity_tag: String::new(),
            instances: HashMap::new(),
            by_commit_scope: HashMap::new(),
            prepared_outputs: HashMap::new(),
            committed_outputs: HashMap::new(),
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
            .map_or(0, |r| r.program_state_version)
    }

    pub fn load_continuation(&self, program_instance_ref: &ProgramInstanceRef) -> Option<Value> {
        self.instances
            .get(program_instance_ref.as_str())
            .and_then(|r| r.continuation.clone())
    }

    /// Read the payload and its write-set digest from one authoritative record.
    /// The filesystem adapter exposes this pair without a second lookup, so a
    /// resume caller can verify it before deserializing the continuation.
    pub fn load_continuation_with_integrity(
        &self,
        program_instance_ref: &ProgramInstanceRef,
    ) -> Option<CommittedContinuation> {
        self.instances
            .get(program_instance_ref.as_str())
            .and_then(|record| {
                record
                    .continuation
                    .clone()
                    .map(|payload| CommittedContinuation {
                        digest: record.write_set.continuation_digest.clone(),
                        payload,
                    })
            })
    }

    pub fn prepare_output(
        &mut self,
        preparation: SessionOutputPreparation,
    ) -> Result<PreparedOutputRef, CommitLocalError> {
        if preparation.commit_id.trim().is_empty()
            || preparation.program_instance_ref.as_str().trim().is_empty()
            || preparation
                .program_invocation_ref
                .as_str()
                .trim()
                .is_empty()
        {
            return Err(CommitLocalError::InvalidOutput(
                "commit and invocation scope must be non-empty".to_string(),
            ));
        }
        if preparation.content.len() > MAX_OUTPUT_BYTES {
            return Err(CommitLocalError::OutputTooLarge {
                bytes: preparation.content.len(),
            });
        }
        if preparation.media_type.trim().is_empty()
            || preparation.media_type.contains('\n')
            || preparation.media_type.contains('\r')
        {
            return Err(CommitLocalError::InvalidOutput(
                "media_type must be one non-empty line".to_string(),
            ));
        }

        let content_digest = sha256_digest(&preparation.content);
        let identity_digest = sha256_digest(
            format!(
                "{}\n{}\n{}\n{}",
                preparation.commit_id,
                preparation.program_instance_ref.as_str(),
                preparation.program_invocation_ref.as_str(),
                content_digest,
            )
            .as_bytes(),
        );
        let output_ref = format!("{LOCAL_OUTPUT_REF_PREFIX}{identity_digest}");
        let prepared = PreparedOutputRef {
            output_ref: output_ref.clone(),
            content_digest,
            byte_length: preparation.content.len(),
            media_type: preparation.media_type,
        };
        let stored = StoredOutput {
            prepared: prepared.clone(),
            commit_id: preparation.commit_id,
            program_instance_ref: preparation.program_instance_ref.as_str().to_string(),
            program_invocation_ref: preparation.program_invocation_ref.as_str().to_string(),
            content: preparation.content,
        };

        if let Some(existing) = self.prepared_outputs.get(&output_ref) {
            if existing != &stored {
                return Err(CommitLocalError::InvalidOutput(
                    "replayed preparation has different content or scope".to_string(),
                ));
            }
            return Ok(prepared);
        }
        if let Some(existing) = self.committed_outputs.get(&output_ref) {
            if existing != &stored {
                return Err(CommitLocalError::InvalidOutput(
                    "replayed preparation has different content or scope".to_string(),
                ));
            }
            return Ok(prepared);
        }
        if self.prepared_outputs.len() + self.committed_outputs.len() >= MAX_OUTPUT_RECORDS {
            return Err(CommitLocalError::RetentionExceeded);
        }
        self.prepared_outputs.insert(output_ref, stored);
        Ok(prepared)
    }

    pub fn read_output(&self, output_ref: &str) -> Option<Vec<u8>> {
        self.committed_outputs
            .get(output_ref)
            .map(|output| output.content.clone())
    }

    pub fn reclaim_prepared_output(&mut self, output_ref: &str) -> Result<bool, CommitLocalError> {
        if !output_ref.starts_with(LOCAL_OUTPUT_REF_PREFIX) {
            return Err(CommitLocalError::InvalidOutput(
                "output ref is not an owner-local Session Output ref".to_string(),
            ));
        }
        if self.committed_outputs.contains_key(output_ref) {
            return Err(CommitLocalError::OutputAlreadyCommitted {
                output_ref: output_ref.to_string(),
            });
        }
        Ok(self.prepared_outputs.remove(output_ref).is_some())
    }

    /// Apply one compare-and-commit against this store. Writes all members or none.
    pub fn commit(
        &mut self,
        request: &ExecutionCommitRequest,
    ) -> Result<ExecutionCommitResult, CommitLocalError> {
        self.validate_schema()?;
        request
            .validate()
            .map_err(|error| CommitLocalError::InvalidRequest(error.to_string()))?;
        enforce_tuple_bound(&request.tuple)?;

        let scope_key = commit_scope_key(request);
        let request_identity = CommitRequestIdentity::from(request);
        if let Some(prior) = self.by_commit_scope.get(&scope_key) {
            if prior.request_identity != request_identity {
                return Err(CommitLocalError::ConflictingReplay {
                    commit_id: request.commit_id.clone(),
                });
            }
            return Ok(ExecutionCommitResult::from(&prior.result));
        }

        if self.force_unknown.contains(&request.commit_id) {
            let result = ExecutionCommitResult::OutcomeUnknown {
                reconciliation_ref: format!("reconcile:{}", request.commit_id),
            };
            self.retain_result(&scope_key, request_identity, &result)?;
            return Ok(result);
        }

        let output_refs = self.validate_prepared_outputs(request)?;

        let current = self.current_version(&request.program_instance_ref);
        if request.expected_program_state_version != current {
            let result = ExecutionCommitResult::CompareConflict {
                current_program_state_version: current,
            };
            self.retain_result(&scope_key, request_identity, &result)?;
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
        self.ensure_retention_capacity(&scope_key)?;
        self.instances
            .insert(request.program_instance_ref.as_str().to_string(), record);
        for output_ref in output_refs {
            let output = self
                .prepared_outputs
                .remove(&output_ref)
                .expect("validated prepared output remains present");
            self.committed_outputs.insert(output_ref, output);
        }
        self.retain_result(&scope_key, request_identity, &result)?;
        Ok(result)
    }

    fn validate_prepared_outputs(
        &self,
        request: &ExecutionCommitRequest,
    ) -> Result<Vec<String>, CommitLocalError> {
        let mut output_refs = HashSet::new();
        for value in &request.tuple.output_refs {
            let Some(output_ref) = value.get("ref").and_then(Value::as_str) else {
                continue;
            };
            if !output_ref.starts_with(LOCAL_OUTPUT_REF_PREFIX) {
                continue;
            }
            if !output_refs.insert(output_ref.to_string()) {
                continue;
            }
            if value.get("ref_type").and_then(Value::as_str) != Some("SessionOutputRef") {
                return Err(CommitLocalError::InvalidOutput(
                    "local Session Output ref has the wrong ref_type".to_string(),
                ));
            }
            let Some(output) = self.prepared_outputs.get(output_ref) else {
                return Err(CommitLocalError::OutputNotPrepared {
                    output_ref: output_ref.to_string(),
                });
            };
            let Some(content_digest) = value.get("content_digest").and_then(Value::as_str) else {
                return Err(CommitLocalError::InvalidOutput(
                    "local Session Output ref is missing content_digest".to_string(),
                ));
            };
            let Some(byte_length) = value.get("byte_length").and_then(Value::as_u64) else {
                return Err(CommitLocalError::InvalidOutput(
                    "local Session Output ref is missing byte_length".to_string(),
                ));
            };
            let Some(media_type) = value.get("media_type").and_then(Value::as_str) else {
                return Err(CommitLocalError::InvalidOutput(
                    "local Session Output ref is missing media_type".to_string(),
                ));
            };
            if content_digest != output.prepared.content_digest
                || byte_length != output.prepared.byte_length as u64
                || media_type != output.prepared.media_type
            {
                return Err(CommitLocalError::InvalidOutput(
                    "local Session Output ref metadata does not match prepared bytes".to_string(),
                ));
            }
            if output.commit_id != request.commit_id
                || output.program_instance_ref != request.program_instance_ref.as_str()
                || output.program_invocation_ref != request.program_invocation_ref.as_str()
            {
                return Err(CommitLocalError::OutputScopeMismatch {
                    output_ref: output_ref.to_string(),
                });
            }
        }
        Ok(output_refs.into_iter().collect())
    }

    fn ensure_retention_capacity(&self, scope_key: &str) -> Result<(), CommitLocalError> {
        if self.by_commit_scope.len() >= MAX_COMMIT_RESULTS
            && !self.by_commit_scope.contains_key(scope_key)
        {
            return Err(CommitLocalError::RetentionExceeded);
        }
        Ok(())
    }

    fn retain_result(
        &mut self,
        scope_key: &str,
        request_identity: CommitRequestIdentity,
        result: &ExecutionCommitResult,
    ) -> Result<(), CommitLocalError> {
        self.ensure_retention_capacity(scope_key)?;
        self.by_commit_scope.insert(
            scope_key.to_string(),
            StoredCommit {
                request_identity,
                result: StoredCommitResult::from(result),
            },
        );
        Ok(())
    }
}

fn commit_scope_key(request: &ExecutionCommitRequest) -> String {
    serde_json::to_string(&[
        request.commit_id.as_str(),
        request.program_instance_ref.as_str(),
        request.program_invocation_ref.as_str(),
    ])
    .expect("commit scope strings are serializable")
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

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
