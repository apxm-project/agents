//! Shared durable record shape for owner-local Execution Commit adapters.
//!
//! One writer mutates one store. Compare-and-commit publishes the whole five-
//! member write set or none of it. Replay identity is scoped to one commit,
//! Program Instance, and Program Invocation.

use std::collections::{HashMap, HashSet};

use apxm_kernel::{
    CommittedContinuation, ExecutionCommitRequest, ExecutionCommitResult, ExecutionCommitTuple,
    PrecommitEvidenceRef, ProgramInstanceRef, ProgramInvocationRef,
};
use apxm_program::grammar::is_identifier;
use apxm_program::runtime_evidence::Fact;
use apxm_runtime_protocol::execution_contracts::OccurrenceId;
use apxm_runtime_protocol::{
    Commitment, ContentReadResult, ContentRef, CorrelationId, EvidenceFactKind, EvidenceRecord,
    ExecutionCursor, ExecutionObservation, ExecutionPage, ExecutionReadRequest,
    ExecutionReadResult, GrantRef, NodeExecutionId, ObservationKind, OutputRef, OutputVisibility,
    PrincipalRef, ProgramInstanceId, ReadContext, ReadPurpose, SESSION_OUTPUT_REF_CONTRACT,
    ScopeRef, SessionOutputRef,
};
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

/// Maximum number of retained non-authoritative observations and evidence
/// records in one local store. Older records are reclaimed as a typed gap.
pub const MAX_READ_RECORDS: usize = 10_000;

const LOCAL_OUTPUT_REF_PREFIX: &str = "apxm.local-session-output/";

fn empty_program_instance_ref() -> ProgramInstanceRef {
    ProgramInstanceRef::new("")
}

fn empty_program_invocation_ref() -> ProgramInvocationRef {
    ProgramInvocationRef::new("")
}

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
    #[error("commit-local program state version is exhausted")]
    VersionExhausted,
    #[error("typed read is unauthorized: {0}")]
    Unauthorized(String),
    #[error("typed read cursor is invalid: {0}")]
    InvalidCursor(String),
    #[error("typed read is below the retention floor ({floor})")]
    RetentionGap { floor: u64 },
    #[error("provisional Session Output is not readable")]
    ProvisionalOutput,
    #[error("typed read request is invalid: {0}")]
    InvalidRead(String),
}

/// Read operation supplied to the composition-owned authorization hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOperation {
    /// Read the ordered observation stream.
    Observation,
    /// Inspect one invocation's committed metadata.
    Inspection,
    /// Read committed Session Output bytes.
    Content,
    /// Read one committed Session Output by its output reference.
    Output,
    /// Read the ordered committed evidence stream.
    Evidence,
}

/// Exact durable object a read is authorizing.  The composition hook receives
/// this target in addition to the caller context so it can make a resource-
/// scoped decision instead of authorizing an operation in the abstract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadTarget {
    Observation {
        program_invocation_ref: String,
    },
    Invocation {
        program_invocation_ref: String,
        node_execution_ref: Option<String>,
    },
    Content {
        content_ref: String,
    },
    Output {
        output_ref: String,
    },
    Evidence {
        program_invocation_ref: String,
    },
}

/// One caller authorization decision input.  APXM validates its shape and
/// durable joins; the composition root owns the actual policy decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadAuthorization {
    pub context: ReadContext,
    pub operation: ReadOperation,
    pub target: ReadTarget,
}

/// Audit information emitted after a typed read authorization decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadAudit {
    /// Caller request identity.
    pub request_id: String,
    /// Scope presented by the caller.
    pub scope_ref: String,
    /// Principal presented by the caller.
    pub principal_ref: String,
    /// Grant presented by the caller.
    pub grant_ref: String,
    /// Operation that was attempted.
    pub operation: ReadOperation,
    /// Exact invocation/node/content/evidence target that was attempted.
    pub target: ReadTarget,
    /// Whether the read completed successfully.
    pub allowed: bool,
}

/// Composition-owned reauthorization and audit hook. The local adapter has no
/// product policy; it always invokes this hook before exposing durable data.
pub trait ReadAccessHook: Send + Sync {
    /// Reauthorize the current principal, grant, and scope for one operation.
    fn reauthorize(&self, request: &ReadAuthorization) -> Result<(), String>;

    /// Record the access decision for the caller's audit system.
    fn audit(&self, event: ReadAudit) -> Result<(), String>;
}

/// Explicit allow hook for tests or a Composition Root that has already
/// performed policy evaluation. It is never installed by a default adapter.
#[derive(Default)]
pub struct AllowReadAccess;

impl ReadAccessHook for AllowReadAccess {
    fn reauthorize(&self, _request: &ReadAuthorization) -> Result<(), String> {
        Ok(())
    }

    fn audit(&self, _event: ReadAudit) -> Result<(), String> {
        Ok(())
    }
}

/// Fail-closed hook installed when an adapter is constructed without an
/// explicit Composition Root authorization binding.
#[derive(Default)]
pub struct DenyReadAccess;

impl ReadAccessHook for DenyReadAccess {
    fn reauthorize(&self, _request: &ReadAuthorization) -> Result<(), String> {
        Err("no read authorization binding configured".into())
    }

    fn audit(&self, _event: ReadAudit) -> Result<(), String> {
        Ok(())
    }
}

/// Explicit owner-local authorization binding for a stdio composition root.
///
/// The binding carries only caller-owned opaque references.  It does not
/// interpret grants or implement product policy; the embedding owner decides
/// that this exact identity/scope binding is authorized before starting the
/// child service.  Every request must repeat the same typed binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadAuthorizationBinding {
    pub scope_ref: ScopeRef,
    pub principal_ref: PrincipalRef,
    pub grant_ref: GrantRef,
    pub correlation_id: Option<CorrelationId>,
}

impl ReadAuthorizationBinding {
    /// Load an explicit binding supplied by a composition root.  Missing all
    /// variables means no binding (and therefore the normal deny default).
    pub fn from_env() -> Result<Option<Self>, String> {
        const SCOPE: &str = "APXM_RUNTIME_READ_SCOPE_REF";
        const PRINCIPAL: &str = "APXM_RUNTIME_READ_PRINCIPAL_REF";
        const GRANT: &str = "APXM_RUNTIME_READ_GRANT_REF";
        const CORRELATION: &str = "APXM_RUNTIME_READ_CORRELATION_REF";
        let scope = std::env::var(SCOPE).ok();
        let principal = std::env::var(PRINCIPAL).ok();
        let grant = std::env::var(GRANT).ok();
        let correlation = normalize_optional_correlation(std::env::var(CORRELATION).ok());
        if scope.is_none() && principal.is_none() && grant.is_none() && correlation.is_none() {
            return Ok(None);
        }
        let scope = scope.ok_or_else(|| format!("{SCOPE} is required with read authorization"))?;
        let principal =
            principal.ok_or_else(|| format!("{PRINCIPAL} is required with read authorization"))?;
        let grant = grant.ok_or_else(|| format!("{GRANT} is required with read authorization"))?;
        Ok(Some(Self {
            scope_ref: ScopeRef::new(scope).map_err(|error| error.to_string())?,
            principal_ref: PrincipalRef::new(principal).map_err(|error| error.to_string())?,
            grant_ref: GrantRef::new(grant).map_err(|error| error.to_string())?,
            correlation_id: correlation
                .map(CorrelationId::new)
                .transpose()
                .map_err(|error| error.to_string())?,
        }))
    }

    fn matches(&self, request: &ReadAuthorization) -> bool {
        request.context.scope_ref == self.scope_ref
            && request.context.principal_ref == self.principal_ref
            && request.context.grant_ref == self.grant_ref
            && request.context.correlation_id == self.correlation_id
    }
}

fn normalize_optional_correlation(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

impl ReadAccessHook for ReadAuthorizationBinding {
    fn reauthorize(&self, request: &ReadAuthorization) -> Result<(), String> {
        if self.matches(request) {
            Ok(())
        } else {
            Err("caller read authorization binding does not match request".into())
        }
    }

    fn audit(&self, _event: ReadAudit) -> Result<(), String> {
        // The owner may wrap this binding with its fallible audit sink.  This
        // transport-level binding only authenticates the exact caller refs.
        Ok(())
    }
}

/// One authoritative instance record after a successful atomic commit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitLocalRecord {
    /// Invocation owner of this latest instance commit.
    #[serde(default)]
    pub program_invocation_ref: String,
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
    #[serde(default = "empty_program_instance_ref")]
    pub program_instance_ref: ProgramInstanceRef,
    #[serde(default = "empty_program_invocation_ref")]
    pub program_invocation_ref: ProgramInvocationRef,
    pub content_digest: String,
    pub byte_length: usize,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_execution_id: Option<NodeExecutionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<OccurrenceId>,
    pub access_scope_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disclosure_ref: Option<String>,
}

impl PreparedOutputRef {
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "ref_type": "SessionOutputRef",
            "ref": self.output_ref,
            "program_instance_id": self.program_instance_ref,
            "program_invocation_id": self.program_invocation_ref,
            "content_digest": self.content_digest,
            "byte_length": self.byte_length,
            "media_type": self.media_type,
            "contract": SESSION_OUTPUT_REF_CONTRACT,
            "commitment": "committed",
            "access_scope_ref": self.access_scope_ref,
        });
        if let Some(disclosure_ref) = &self.disclosure_ref {
            value["disclosure_ref"] = json!(disclosure_ref);
        }
        if let Some(node_execution_id) = &self.node_execution_id {
            value["node_execution_id"] = json!(node_execution_id);
        }
        if let Some(occurrence_id) = &self.occurrence_id {
            value["occurrence_id"] = json!(occurrence_id);
        }
        value
    }

    pub(crate) fn to_kernel(&self) -> apxm_kernel::PreparedSessionOutputRef {
        apxm_kernel::PreparedSessionOutputRef {
            contract: SESSION_OUTPUT_REF_CONTRACT.to_owned(),
            ref_type: "SessionOutputRef".to_owned(),
            output_ref: self.output_ref.clone(),
            program_instance_id: self.program_instance_ref.as_str().to_owned(),
            program_invocation_id: self.program_invocation_ref.as_str().to_owned(),
            node_execution_id: self
                .node_execution_id
                .as_ref()
                .map(|reference| reference.as_str().to_owned()),
            occurrence_id: self
                .occurrence_id
                .as_ref()
                .map(|reference| reference.as_str().to_owned()),
            content_digest: self.content_digest.clone(),
            byte_length: self.byte_length as u64,
            media_type: self.media_type.clone(),
            visibility: apxm_kernel::SessionOutputVisibility::Committed,
            access_scope_ref: self.access_scope_ref.clone(),
            disclosure_ref: self.disclosure_ref.clone(),
        }
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
    /// Dynamic NodeExecution that produced this output, when the runtime has
    /// one. Static node identity is never sufficient for a repeated visit.
    pub node_execution_id: Option<NodeExecutionId>,
    /// Dynamic occurrence containing the producing NodeExecution.
    pub occurrence_id: Option<OccurrenceId>,
    pub access_scope_ref: String,
    pub disclosure_ref: Option<String>,
}

impl SessionOutputPreparation {
    pub(crate) fn from_kernel(
        preparation: apxm_kernel::SessionOutputPreparation,
    ) -> Result<Self, CommitLocalError> {
        preparation
            .validate()
            .map_err(|error| CommitLocalError::InvalidOutput(error.to_owned()))?;
        let node_execution_id = preparation
            .node_execution_id
            .map(NodeExecutionId::new)
            .transpose()
            .map_err(|error| CommitLocalError::InvalidOutput(error.to_string()))?;
        let occurrence_id = preparation
            .occurrence_id
            .map(OccurrenceId::new)
            .transpose()
            .map_err(|error| CommitLocalError::InvalidOutput(error.to_string()))?;
        if occurrence_id.is_some() && node_execution_id.is_none() {
            return Err(CommitLocalError::InvalidOutput(
                "occurrence_id requires node_execution_id".into(),
            ));
        }
        Ok(Self {
            commit_id: preparation.commit_id,
            program_instance_ref: ProgramInstanceRef::new(preparation.program_instance_ref),
            program_invocation_ref: ProgramInvocationRef::new(preparation.program_invocation_ref),
            content: preparation.content,
            media_type: preparation.media_type,
            node_execution_id,
            occurrence_id,
            access_scope_ref: preparation.access_scope_ref,
            disclosure_ref: preparation.disclosure_ref,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredOutput {
    pub prepared: PreparedOutputRef,
    pub commit_id: String,
    pub program_instance_ref: String,
    pub program_invocation_ref: String,
    pub content: Vec<u8>,
}

/// Historical invocation index retained independently from the latest
/// instance continuation. Multiple invocations of one instance must never
/// overwrite one another's inspection identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredInvocation {
    pub program_instance_ref: String,
    pub status: apxm_runtime_protocol::ProgramInvocationStatus,
    pub node_execution_refs: Vec<NodeExecutionId>,
    pub output_refs: Vec<OutputRef>,
    pub evidence_refs: Vec<apxm_runtime_protocol::EvidenceRef>,
    pub child_program_refs: Vec<apxm_runtime_protocol::ProgramRef>,
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
    pub observations: Vec<Value>,
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
                observations: request.tuple.observations.clone(),
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
    /// Durable projections of committed evidence and non-authoritative
    /// observations. Both sequences are scoped to their invocation.
    #[serde(default)]
    pub evidence_records: HashMap<String, Vec<EvidenceRecord>>,
    #[serde(default)]
    pub observations: HashMap<String, Vec<ExecutionObservation>>,
    /// Next sequence values are durable high-watermarks, not derived from
    /// retained vector lengths, so retention cannot reuse identities.
    #[serde(default)]
    pub next_evidence_sequence: HashMap<String, u64>,
    #[serde(default)]
    pub next_observation_sequence: HashMap<String, u64>,
    /// Historical invocation and dynamic NodeExecution inspection indexes.
    #[serde(default)]
    pub invocations: HashMap<String, StoredInvocation>,
    #[serde(default)]
    pub node_executions: HashMap<String, apxm_runtime_protocol::NodeExecutionInspection>,
    /// Commit ids forced to `outcome_unknown` without writing (failure injection).
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub force_unknown: HashSet<String>,
    /// Composition-provided scope bound to staged outputs. This is optional
    /// for legacy in-process fixtures; production composition sets it before
    /// admission so the caller's scope, not a runtime-generated identity,
    /// reaches every committed output reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_access_scope_ref: Option<String>,
    /// Optional product-neutral composition metadata owned by an embedding
    /// service. It is authenticated and atomically persisted with the
    /// execution record, but the commit adapter never interprets its shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_metadata: Option<Value>,
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
            evidence_records: HashMap::new(),
            observations: HashMap::new(),
            next_evidence_sequence: HashMap::new(),
            next_observation_sequence: HashMap::new(),
            invocations: HashMap::new(),
            node_executions: HashMap::new(),
            force_unknown: HashSet::new(),
            default_access_scope_ref: None,
            runtime_metadata: None,
        }
    }

    /// Return opaque composition metadata without giving the commit adapter
    /// authority to interpret it.
    #[must_use]
    pub fn runtime_metadata(&self) -> Option<Value> {
        self.runtime_metadata.clone()
    }

    /// Replace opaque composition metadata in the staged store.
    pub fn set_runtime_metadata(&mut self, metadata: Option<Value>) {
        self.runtime_metadata = metadata;
    }

    pub fn validate_schema(&self) -> Result<(), CommitLocalError> {
        if self.schema_version != COMMIT_LOCAL_SCHEMA {
            return Err(CommitLocalError::SchemaMismatch {
                found: self.schema_version.clone(),
            });
        }
        if self
            .default_access_scope_ref
            .as_ref()
            .is_some_and(|reference| reference.len() > 256 || !is_identifier(reference))
        {
            return Err(CommitLocalError::Codec(
                "invalid default access_scope_ref".to_owned(),
            ));
        }
        for records in self.evidence_records.values() {
            for record in records {
                record
                    .validate()
                    .map_err(|error| CommitLocalError::Codec(error.to_string()))?;
            }
        }
        Ok(())
    }

    pub fn inject_outcome_unknown(&mut self, commit_id: impl Into<String>) {
        self.force_unknown.insert(commit_id.into());
    }

    pub fn set_default_access_scope_ref(&mut self, reference: String) {
        self.default_access_scope_ref = Some(reference);
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
        if preparation.occurrence_id.is_some() && preparation.node_execution_id.is_none() {
            return Err(CommitLocalError::InvalidOutput(
                "occurrence_id requires node_execution_id".to_string(),
            ));
        }
        if preparation.access_scope_ref.trim().is_empty() {
            return Err(CommitLocalError::InvalidOutput(
                "access_scope_ref must be non-empty".to_string(),
            ));
        }

        let content_digest = sha256_digest(&preparation.content);
        let identity_digest = sha256_digest(
            format!(
                "{}\n{}\n{}\n{}\n{}\n{}",
                preparation.commit_id,
                preparation.program_instance_ref.as_str(),
                preparation.program_invocation_ref.as_str(),
                content_digest,
                preparation
                    .node_execution_id
                    .as_ref()
                    .map_or("", NodeExecutionId::as_str),
                preparation
                    .occurrence_id
                    .as_ref()
                    .map_or("", OccurrenceId::as_str),
            )
            .as_bytes(),
        );
        let output_ref = format!("{LOCAL_OUTPUT_REF_PREFIX}{identity_digest}");
        let access_scope_ref = self
            .default_access_scope_ref
            .clone()
            .unwrap_or_else(|| preparation.access_scope_ref.clone());
        let prepared = PreparedOutputRef {
            output_ref: output_ref.clone(),
            program_instance_ref: preparation.program_instance_ref.clone(),
            program_invocation_ref: preparation.program_invocation_ref.clone(),
            content_digest,
            byte_length: preparation.content.len(),
            media_type: preparation.media_type,
            node_execution_id: preparation.node_execution_id.clone(),
            occurrence_id: preparation.occurrence_id.clone(),
            access_scope_ref,
            disclosure_ref: preparation.disclosure_ref,
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

    /// Execute one typed read after the composition-owned authorization hook
    /// has reauthorized the principal, grant, and scope.
    pub fn read_execution(
        &self,
        request: ExecutionReadRequest,
        key: &[u8; 32],
        hook: &dyn ReadAccessHook,
    ) -> Result<ExecutionReadResult, CommitLocalError> {
        self.read_execution_with_live(request, key, hook, &[])
    }

    /// Execute a typed read with an optional bounded process-local live
    /// observation overlay. Live records are never persisted; records whose
    /// identity is already durable use the committed copy, and the page
    /// builder mints one scope-bound cursor sequence over the merged view.
    pub fn read_execution_with_live(
        &self,
        request: ExecutionReadRequest,
        key: &[u8; 32],
        hook: &dyn ReadAccessHook,
        live_observations: &[ExecutionObservation],
    ) -> Result<ExecutionReadResult, CommitLocalError> {
        let authorization = read_authorization(&request);
        let request_id = authorization.context.request_id.as_str().to_owned();
        let scope_ref = authorization.context.scope_ref.as_str().to_owned();
        let principal_ref = authorization.context.principal_ref.as_str().to_owned();
        let grant_ref = authorization.context.grant_ref.as_str().to_owned();
        let operation = authorization.operation;
        let target = authorization.target.clone();
        let audit = |allowed| {
            hook.audit(ReadAudit {
                request_id: request_id.clone(),
                scope_ref: scope_ref.clone(),
                principal_ref: principal_ref.clone(),
                grant_ref: grant_ref.clone(),
                operation,
                target: target.clone(),
                allowed,
            })
        };
        if let Err(error) = hook.reauthorize(&authorization) {
            return Err(CommitLocalError::Unauthorized(
                audit(false).err().unwrap_or(error),
            ));
        }
        let result = self.read_execution_authorized(request, key, live_observations);
        if let Err(error) = audit(result.is_ok()) {
            return Err(CommitLocalError::Unauthorized(error));
        }
        result
    }

    fn read_execution_authorized(
        &self,
        request: ExecutionReadRequest,
        key: &[u8; 32],
        live_observations: &[ExecutionObservation],
    ) -> Result<ExecutionReadResult, CommitLocalError> {
        request
            .validate()
            .map_err(|error| CommitLocalError::InvalidRead(error.to_string()))?;
        match request {
            ExecutionReadRequest::ObservationSubscribe {
                context,
                program_invocation_id,
                after_cursor,
                limit,
            } => {
                let mut items = self
                    .observations
                    .get(program_invocation_id.as_str())
                    .cloned()
                    .unwrap_or_default();
                let durable_floor = items.first().map(|item| item.sequence);
                // Overlay only exact invocation records. Once a commit lands,
                // its observation identity suppresses the process-local copy.
                for live in live_observations.iter().filter(|item| {
                    item.program_invocation_id.as_str() == program_invocation_id.as_str()
                }) {
                    live.validate()
                        .map_err(|error| CommitLocalError::InvalidRead(error.to_string()))?;
                    // Do not resurrect records below the durable retention
                    // floor from the process-local recorder.
                    if durable_floor.is_some_and(|floor| live.sequence < floor) {
                        continue;
                    }
                    if items.iter().any(|item| {
                        item.sequence == live.sequence
                            && item.observation_id.as_str() != live.observation_id.as_str()
                    }) {
                        return Err(CommitLocalError::InvalidRead(
                            "live observation sequence collision".into(),
                        ));
                    }
                    if !items
                        .iter()
                        .any(|item| item.observation_id.as_str() == live.observation_id.as_str())
                    {
                        items.push(live.clone());
                    }
                }
                if items.iter().any(|item| {
                    item.program_invocation_id.as_str() != program_invocation_id.as_str()
                }) {
                    return Err(CommitLocalError::InvalidRead(
                        "observation invocation join is invalid".into(),
                    ));
                }
                let page = page_observations(
                    items,
                    &context,
                    program_invocation_id.as_str(),
                    after_cursor.as_ref(),
                    limit,
                    key,
                    self.next_observation_sequence(program_invocation_id.as_str())
                        .saturating_sub(1),
                )?;
                Ok(ExecutionReadResult::ObservationPage { page })
            }
            ExecutionReadRequest::EvidenceRead {
                context,
                program_invocation_id,
                after_cursor,
                limit,
            } => {
                let items = self
                    .evidence_records
                    .get(program_invocation_id.as_str())
                    .cloned()
                    .unwrap_or_default();
                if items.iter().any(|item| {
                    item.program_invocation_id.as_str() != program_invocation_id.as_str()
                }) {
                    return Err(CommitLocalError::InvalidRead(
                        "evidence invocation join is invalid".into(),
                    ));
                }
                let page = page_evidence(
                    items,
                    &context,
                    program_invocation_id.as_str(),
                    after_cursor.as_ref(),
                    limit,
                    key,
                    self.next_evidence_sequence(program_invocation_id.as_str())
                        .saturating_sub(1),
                )?;
                Ok(ExecutionReadResult::EvidencePage { page })
            }
            ExecutionReadRequest::ContentRead {
                context,
                content_ref,
            } => Ok(ExecutionReadResult::Content {
                content: self.read_committed_content(context, content_ref)?,
            }),
            ExecutionReadRequest::OutputRead {
                context,
                output_ref,
            } => {
                let content_ref = ContentRef::new(output_ref.as_str().to_owned())
                    .map_err(|error| CommitLocalError::InvalidRead(error.to_string()))?;
                let content = self.read_committed_content(context, content_ref)?;
                if content
                    .output_ref
                    .as_ref()
                    .is_none_or(|reference| reference.reference.as_str() != output_ref.as_str())
                {
                    return Err(CommitLocalError::OutputScopeMismatch {
                        output_ref: output_ref.as_str().to_owned(),
                    });
                }
                Ok(ExecutionReadResult::Output { output: content })
            }
            ExecutionReadRequest::ProgramInvocationInspect {
                context,
                program_invocation_id,
                node_execution_id,
            } => {
                let invocation = self
                    .invocations
                    .get(program_invocation_id.as_str())
                    .ok_or_else(|| CommitLocalError::InvalidRead("invocation not found".into()))?;
                if let Some(node_execution_id) = node_execution_id.as_ref()
                    && !invocation.node_execution_refs.contains(node_execution_id)
                {
                    return Err(CommitLocalError::InvalidRead(
                        "node execution is not part of the invocation".into(),
                    ));
                }
                let filter = node_execution_id
                    .as_ref()
                    .map_or("all", NodeExecutionId::as_str);
                let evidence_high = self
                    .next_evidence_sequence(program_invocation_id.as_str())
                    .saturating_sub(1);
                let cursor = ExecutionCursor::new(
                    evidence_high,
                    keyed_cursor_token(
                        key,
                        ReadOperation::Inspection,
                        program_invocation_id.as_str(),
                        &context,
                        filter,
                        evidence_high,
                    ),
                )
                .map_err(|error| CommitLocalError::InvalidCursor(error.to_string()))?;
                let inspection = apxm_runtime_protocol::ProgramInvocationInspection {
                    program_invocation_id: program_invocation_id.clone(),
                    status: invocation.status,
                    node_execution_refs: invocation.node_execution_refs.clone(),
                    output_refs: invocation.output_refs.clone(),
                    evidence_refs: invocation.evidence_refs.clone(),
                    cursor,
                };
                if let Some(node_execution_id) = node_execution_id {
                    let node_key = format!(
                        "{}:{}",
                        program_invocation_id.as_str(),
                        node_execution_id.as_str()
                    );
                    let node = self.node_executions.get(&node_key).ok_or_else(|| {
                        CommitLocalError::InvalidRead("node execution not found".into())
                    })?;
                    if node.program_invocation_id != program_invocation_id
                        || node.node_execution_id != node_execution_id
                    {
                        return Err(CommitLocalError::InvalidRead(
                            "node execution is not part of the invocation".into(),
                        ));
                    }
                    node.validate()
                        .map_err(|error| CommitLocalError::InvalidRead(error.to_string()))?;
                    if node
                        .evidence_refs
                        .iter()
                        .any(|reference| !invocation.evidence_refs.contains(reference))
                        || node
                            .output_ref
                            .as_ref()
                            .is_some_and(|reference| !invocation.output_refs.contains(reference))
                    {
                        return Err(CommitLocalError::InvalidRead(
                            "node execution references do not belong to the invocation".into(),
                        ));
                    }
                    return Ok(ExecutionReadResult::NodeExecutionInspection {
                        inspection: node.clone(),
                    });
                }
                Ok(ExecutionReadResult::ProgramInvocationInspection { inspection })
            }
        }
    }

    fn read_committed_content(
        &self,
        context: ReadContext,
        content_ref: ContentRef,
    ) -> Result<ContentReadResult, CommitLocalError> {
        if self.prepared_outputs.contains_key(content_ref.as_str()) {
            return Err(CommitLocalError::ProvisionalOutput);
        }
        let output = self
            .committed_outputs
            .get(content_ref.as_str())
            .ok_or_else(|| CommitLocalError::InvalidRead("content not found".into()))?;
        if output.prepared.output_ref != content_ref.as_str() {
            return Err(CommitLocalError::OutputScopeMismatch {
                output_ref: content_ref.as_str().to_owned(),
            });
        }
        let invocation = self
            .invocations
            .get(&output.program_invocation_ref)
            .ok_or_else(|| {
                CommitLocalError::InvalidRead("content invocation is not indexed".into())
            })?;
        if !invocation
            .output_refs
            .iter()
            .any(|reference| reference.as_str() == content_ref.as_str())
        {
            return Err(CommitLocalError::OutputScopeMismatch {
                output_ref: content_ref.as_str().to_owned(),
            });
        }
        if let Some(node_execution_id) = output.prepared.node_execution_id.as_ref() {
            let node_key = format!(
                "{}:{}",
                output.program_invocation_ref,
                node_execution_id.as_str()
            );
            if let Some(node) = self.node_executions.get(&node_key)
                && (node.program_invocation_id.as_str() != output.program_invocation_ref
                    || node.node_execution_id != *node_execution_id
                    || node
                        .output_ref
                        .as_ref()
                        .is_some_and(|reference| reference.as_str() != content_ref.as_str())
                    || node.occurrence_id != output.prepared.occurrence_id)
            {
                return Err(CommitLocalError::OutputScopeMismatch {
                    output_ref: content_ref.as_str().to_owned(),
                });
            }
        }
        let output_ref = typed_output_ref(output)?;
        let program_instance_id = ProgramInstanceId::new(output.program_instance_ref.clone())
            .map_err(|error| CommitLocalError::InvalidRead(error.to_string()))?;
        let program_invocation_id =
            apxm_runtime_protocol::ProgramInvocationId::new(output.program_invocation_ref.clone())
                .map_err(|error| CommitLocalError::InvalidRead(error.to_string()))?;
        let content = ContentReadResult {
            content_ref,
            program_instance_id,
            program_invocation_id,
            node_execution_id: output.prepared.node_execution_id.clone(),
            occurrence_id: output.prepared.occurrence_id.clone(),
            output_ref: Some(output_ref),
            content_digest: output.prepared.content_digest.clone(),
            byte_length: output.prepared.byte_length as u64,
            media_type: output.prepared.media_type.clone(),
            visibility: OutputVisibility::Committed,
            access_scope_ref: output.prepared.access_scope_ref.clone(),
            disclosure_ref: output.prepared.disclosure_ref.clone(),
            bytes: output.content.clone(),
        };
        if context.purpose != ReadPurpose::Content && context.purpose != ReadPurpose::Output {
            return Err(CommitLocalError::InvalidRead(
                "content read purpose mismatch".into(),
            ));
        }
        if context.scope_ref.as_str() != output.prepared.access_scope_ref {
            return Err(CommitLocalError::Unauthorized(
                "content scope does not match the stored access_scope_ref".into(),
            ));
        }
        content
            .validate()
            .map_err(|error| CommitLocalError::InvalidRead(error.to_string()))?;
        Ok(content)
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
        // Validate coordinates before any result path, including the forced
        // OutcomeUnknown path below.  Otherwise a malformed evidence fact
        // could still advance invocation state while bypassing indexing.
        for fact in &request.evidence_batch {
            validate_fact_coordinates(fact)?;
        }
        enforce_tuple_bound(&request.tuple)?;
        validate_tuple_session_output_refs(&request.tuple.output_refs)?;

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

        if self
            .invocations
            .get(request.program_invocation_ref.as_str())
            .is_some_and(|invocation| {
                invocation.program_instance_ref != request.program_instance_ref.as_str()
            })
        {
            return Err(CommitLocalError::InvalidRequest(
                "program invocation is bound to a different program instance".into(),
            ));
        }

        if self.force_unknown.contains(&request.commit_id) {
            self.ensure_retention_capacity(&scope_key)?;
            let invocation = self
                .invocations
                .entry(request.program_invocation_ref.as_str().to_owned())
                .or_insert_with(|| StoredInvocation {
                    program_instance_ref: request.program_instance_ref.as_str().to_owned(),
                    status: apxm_runtime_protocol::ProgramInvocationStatus::OutcomeUnknown,
                    node_execution_refs: Vec::new(),
                    output_refs: Vec::new(),
                    evidence_refs: Vec::new(),
                    child_program_refs: Vec::new(),
                });
            invocation.status = fold_invocation_status(
                invocation.status,
                apxm_runtime_protocol::ProgramInvocationStatus::OutcomeUnknown,
                false,
            );
            let result = ExecutionCommitResult::OutcomeUnknown {
                reconciliation_ref: format!("reconcile:{}", request.commit_id),
            };
            self.retain_result(&scope_key, request_identity, &result)?;
            return Ok(result);
        }

        let output_refs = self.validate_prepared_outputs(request)?;

        let current = self.current_version(&request.program_instance_ref);
        let next_version = current
            .checked_add(1)
            .ok_or(CommitLocalError::VersionExhausted)?;
        let evidence_position_ref = format!(
            "evidence:{}:{next_version}",
            request.program_invocation_ref.as_str()
        );
        let prior_event_wait = self
            .instances
            .get(request.program_instance_ref.as_str())
            .filter(|record| {
                record.program_invocation_ref == request.program_invocation_ref.as_str()
            })
            .and_then(|record| record.event_wait.clone());
        let (evidence_records, observations) = self.prepare_commit_records(
            request,
            &evidence_position_ref,
            prior_event_wait.as_ref(),
        )?;
        let invocation_key = request.program_invocation_ref.as_str().to_owned();
        let evidence_start = self.next_evidence_sequence(&invocation_key);

        if request.expected_program_state_version != current {
            let result = ExecutionCommitResult::CompareConflict {
                current_program_state_version: current,
            };
            self.retain_result(&scope_key, request_identity, &result)?;
            return Ok(result);
        }

        let record = CommitLocalRecord {
            program_invocation_ref: request.program_invocation_ref.as_str().to_owned(),
            program_state_version: next_version,
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
            new_program_state_version: next_version,
            evidence_position_ref,
        };
        self.ensure_retention_capacity(&scope_key)?;
        self.instances
            .insert(request.program_instance_ref.as_str().to_string(), record);
        self.evidence_records
            .entry(invocation_key.clone())
            .or_default()
            .extend(evidence_records);
        self.observations
            .entry(invocation_key.clone())
            .or_default()
            .extend(observations.clone());
        self.next_evidence_sequence.insert(
            invocation_key.clone(),
            evidence_start
                .checked_add(request.evidence_batch.len() as u64)
                .ok_or(CommitLocalError::VersionExhausted)?,
        );
        if let Some(last) = observations.last() {
            self.next_observation_sequence.insert(
                invocation_key.clone(),
                last.sequence
                    .checked_add(1)
                    .ok_or(CommitLocalError::VersionExhausted)?,
            );
        }
        if let Some(records) = self.evidence_records.get_mut(&invocation_key) {
            let retained = records.len().min(MAX_READ_RECORDS);
            let remove = records.len() - retained;
            if remove > 0 {
                records.drain(..remove);
            }
        }
        if let Some(records) = self.observations.get_mut(&invocation_key) {
            let retained = records.len().min(MAX_READ_RECORDS);
            let remove = records.len() - retained;
            if remove > 0 {
                records.drain(..remove);
            }
        }
        for output_ref in output_refs {
            let output = self
                .prepared_outputs
                .remove(&output_ref)
                .expect("validated prepared output remains present");
            self.committed_outputs.insert(output_ref, output);
        }
        self.index_commit(request);
        self.retain_result(&scope_key, request_identity, &result)?;
        Ok(result)
    }

    fn prepare_commit_records(
        &self,
        request: &ExecutionCommitRequest,
        evidence_position_ref: &str,
        prior_event_wait: Option<&Value>,
    ) -> Result<(Vec<EvidenceRecord>, Vec<ExecutionObservation>), CommitLocalError> {
        let invocation = apxm_runtime_protocol::ProgramInvocationId::new(
            request.program_invocation_ref.as_str(),
        )
        .map_err(|error| CommitLocalError::InvalidRequest(error.to_string()))?;
        let evidence = self.next_evidence_sequence(request.program_invocation_ref.as_str()) - 1;
        let mut records = Vec::with_capacity(request.evidence_batch.len());
        for (offset, fact) in request.evidence_batch.iter().enumerate() {
            if !fact_belongs_to_invocation(fact, request.program_invocation_ref.as_str()) {
                return Err(CommitLocalError::InvalidRequest(
                    "evidence fact invocation does not match the commit invocation".into(),
                ));
            }
            let sequence = evidence
                .checked_add(offset as u64 + 1)
                .ok_or(CommitLocalError::VersionExhausted)?;
            let fact_kind = evidence_kind(fact);
            let precommit_ref = PrecommitEvidenceRef::new(
                request.commit_id.clone(),
                request.program_invocation_ref.as_str(),
                offset as u64,
            )
            .map_err(|error| CommitLocalError::InvalidRequest(error.to_owned()))?;
            precommit_ref
                .validate()
                .map_err(|error| CommitLocalError::InvalidRequest(error.to_owned()))?;
            let (node_execution_id, occurrence_id) =
                node_coordinates(fact).map_or((None, None), |(node, _, _, occurrence, _, _)| {
                    (
                        NodeExecutionId::new(node).ok(),
                        occurrence.and_then(|value| OccurrenceId::new(value).ok()),
                    )
                });
            let mut evidence_ref = EvidenceRecord {
                evidence_ref: apxm_runtime_protocol::EvidenceRef::new(precommit_ref.evidence_ref)
                    .map_err(|error| {
                    CommitLocalError::InvalidRequest(error.to_string())
                })?,
                program_invocation_id: invocation.clone(),
                sequence,
                fact_kind,
                evidence_digest: String::new(),
                node_execution_id,
                occurrence_id,
            };
            evidence_ref.evidence_digest = evidence_ref.computed_digest();
            evidence_ref
                .validate()
                .map_err(|error| CommitLocalError::InvalidRequest(error.to_string()))?;
            records.push(evidence_ref);
        }

        let observations = request
            .tuple
            .observations
            .iter()
            .map(|value| {
                serde_json::from_value::<ExecutionObservation>(value.clone()).map_err(|error| {
                    CommitLocalError::InvalidRequest(format!(
                        "invalid driver-owned execution observation: {error}"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        validate_observation_batch(
            &observations,
            request.program_invocation_ref.as_str(),
            self.next_observation_sequence(request.program_invocation_ref.as_str()),
            request.tuple.output_refs.as_slice(),
            records.as_slice(),
            evidence_position_ref,
            ObservationEventWaits {
                current: request.tuple.event_wait.as_ref(),
                prior: prior_event_wait,
            },
        )?;
        Ok((records, observations))
    }

    fn next_evidence_sequence(&self, invocation: &str) -> u64 {
        self.next_evidence_sequence
            .get(invocation)
            .copied()
            .unwrap_or_else(|| {
                self.evidence_records
                    .get(invocation)
                    .and_then(|records| records.last())
                    .map_or(1, |record| record.sequence.saturating_add(1))
            })
    }

    fn next_observation_sequence(&self, invocation: &str) -> u64 {
        self.next_observation_sequence
            .get(invocation)
            .copied()
            .unwrap_or_else(|| {
                self.observations
                    .get(invocation)
                    .and_then(|records| records.last())
                    .map_or(1, |record| record.sequence.saturating_add(1))
            })
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
                return Err(CommitLocalError::InvalidOutput(
                    "local Session Output ref is duplicated in the commit tuple".into(),
                ));
            }
            if value.get("ref_type").and_then(Value::as_str) != Some("SessionOutputRef") {
                return Err(CommitLocalError::InvalidOutput(
                    "local Session Output ref has the wrong ref_type".to_string(),
                ));
            }
            let submitted: SessionOutputRef =
                serde_json::from_value(value.clone()).map_err(|error| {
                    CommitLocalError::InvalidOutput(format!(
                        "invalid Session Output metadata: {error}"
                    ))
                })?;
            submitted
                .validate()
                .map_err(|error| CommitLocalError::InvalidOutput(error.to_string()))?;
            if submitted.visibility != OutputVisibility::Committed {
                return Err(CommitLocalError::ProvisionalOutput);
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
            if submitted.reference.as_str() != output_ref
                || submitted.program_instance_id.as_str() != request.program_instance_ref.as_str()
                || submitted.program_invocation_id.as_str()
                    != request.program_invocation_ref.as_str()
                || submitted.content_digest != output.prepared.content_digest
                || submitted.byte_length != output.prepared.byte_length as u64
                || submitted.media_type != output.prepared.media_type
                || submitted.access_scope_ref != output.prepared.access_scope_ref
                || submitted.disclosure_ref != output.prepared.disclosure_ref
                || sha256_digest(&output.content) != submitted.content_digest
            {
                return Err(CommitLocalError::OutputScopeMismatch {
                    output_ref: output_ref.to_string(),
                });
            }
            if output.commit_id != request.commit_id
                || output.program_instance_ref != request.program_instance_ref.as_str()
                || output.program_invocation_ref != request.program_invocation_ref.as_str()
                || output.prepared.program_instance_ref != request.program_instance_ref
                || output.prepared.program_invocation_ref != request.program_invocation_ref
            {
                return Err(CommitLocalError::OutputScopeMismatch {
                    output_ref: output_ref.to_string(),
                });
            }
            if value.get("node_execution_id").and_then(Value::as_str)
                != output
                    .prepared
                    .node_execution_id
                    .as_ref()
                    .map(NodeExecutionId::as_str)
                || value.get("occurrence_id").and_then(Value::as_str)
                    != output
                        .prepared
                        .occurrence_id
                        .as_ref()
                        .map(OccurrenceId::as_str)
            {
                return Err(CommitLocalError::OutputScopeMismatch {
                    output_ref: output_ref.to_string(),
                });
            }
        }
        Ok(output_refs.into_iter().collect())
    }

    fn index_commit(&mut self, request: &ExecutionCommitRequest) {
        let invocation_ref = request.program_invocation_ref.as_str().to_owned();
        let Ok(invocation_id) = apxm_runtime_protocol::ProgramInvocationId::new(&invocation_ref)
        else {
            return;
        };
        let mut status = request.evidence_batch.iter().fold(
            apxm_runtime_protocol::ProgramInvocationStatus::Running,
            |status, fact| match fact {
                Fact::InvocationCommitted(fact) => match fact.invocation_state {
                    Some(apxm_program::runtime_evidence::InvocationState::CommittedYield) => {
                        apxm_runtime_protocol::ProgramInvocationStatus::CommittedYield
                    }
                    Some(apxm_program::runtime_evidence::InvocationState::CommittedReturn) => {
                        apxm_runtime_protocol::ProgramInvocationStatus::CommittedReturn
                    }
                    Some(apxm_program::runtime_evidence::InvocationState::Failed) => {
                        apxm_runtime_protocol::ProgramInvocationStatus::Failed
                    }
                    Some(apxm_program::runtime_evidence::InvocationState::Cancelled) => {
                        apxm_runtime_protocol::ProgramInvocationStatus::Cancelled
                    }
                    _ => status,
                },
                Fact::InvocationFailed(_) => apxm_runtime_protocol::ProgramInvocationStatus::Failed,
                Fact::InvocationCancelled(_) => {
                    apxm_runtime_protocol::ProgramInvocationStatus::Cancelled
                }
                Fact::InvocationParked(_) => {
                    apxm_runtime_protocol::ProgramInvocationStatus::WaitingEvent
                }
                Fact::InvocationStateChanged(fact) => match fact.invocation_state {
                    Some(apxm_program::runtime_evidence::InvocationState::WaitingEvent) => {
                        apxm_runtime_protocol::ProgramInvocationStatus::WaitingEvent
                    }
                    Some(apxm_program::runtime_evidence::InvocationState::CommittedYield) => {
                        apxm_runtime_protocol::ProgramInvocationStatus::CommittedYield
                    }
                    Some(apxm_program::runtime_evidence::InvocationState::CommittedReturn) => {
                        apxm_runtime_protocol::ProgramInvocationStatus::CommittedReturn
                    }
                    Some(apxm_program::runtime_evidence::InvocationState::Failed) => {
                        apxm_runtime_protocol::ProgramInvocationStatus::Failed
                    }
                    Some(apxm_program::runtime_evidence::InvocationState::Cancelled) => {
                        apxm_runtime_protocol::ProgramInvocationStatus::Cancelled
                    }
                    _ => status,
                },
                Fact::EffectOutcomeUnknown(_) => {
                    apxm_runtime_protocol::ProgramInvocationStatus::OutcomeUnknown
                }
                _ => status,
            },
        );
        let explicit_resolution = request
            .evidence_batch
            .iter()
            .any(fact_is_explicit_invocation_resolution);
        if !matches!(
            status,
            apxm_runtime_protocol::ProgramInvocationStatus::CommittedReturn
                | apxm_runtime_protocol::ProgramInvocationStatus::Failed
                | apxm_runtime_protocol::ProgramInvocationStatus::Cancelled
                | apxm_runtime_protocol::ProgramInvocationStatus::OutcomeUnknown
        ) && request.tuple.observations.iter().any(|value| {
            serde_json::from_value::<ExecutionObservation>(value.clone()).is_ok_and(|observation| {
                observation.observation_kind == ObservationKind::OutcomeUnknown
            })
        }) {
            status = apxm_runtime_protocol::ProgramInvocationStatus::OutcomeUnknown;
        }
        let evidence_refs = self
            .evidence_records
            .get(&invocation_ref)
            .map(|records| {
                records
                    .iter()
                    .map(|record| record.evidence_ref.clone())
                    .collect()
            })
            .unwrap_or_default();
        let output_refs = self
            .committed_outputs
            .values()
            .filter(|output| output.program_invocation_ref == invocation_ref)
            .filter_map(|output| OutputRef::new(output.prepared.output_ref.clone()).ok())
            .collect::<Vec<_>>();

        // This key is the invocation, never the instance.  A later invocation
        // therefore cannot overwrite an earlier invocation's inspection row.
        let invocation = self
            .invocations
            .entry(invocation_ref.clone())
            .or_insert_with(|| StoredInvocation {
                program_instance_ref: request.program_instance_ref.as_str().to_owned(),
                status,
                node_execution_refs: Vec::new(),
                output_refs: Vec::new(),
                evidence_refs: Vec::new(),
                child_program_refs: Vec::new(),
            });
        request
            .program_instance_ref
            .as_str()
            .clone_into(&mut invocation.program_instance_ref);
        invocation.status = fold_invocation_status(invocation.status, status, explicit_resolution);
        invocation.evidence_refs = evidence_refs;
        invocation.output_refs = output_refs;

        let evidence_tail = self
            .evidence_records
            .get(&invocation_ref)
            .map(|records| {
                records
                    .iter()
                    .rev()
                    .take(request.evidence_batch.len())
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (index, fact) in request.evidence_batch.iter().enumerate() {
            if let Some((node_id, air_node_id, parent, occurrence, region, attempt)) =
                node_coordinates(fact)
            {
                let status = node_status(fact).unwrap_or({
                    if matches!(fact, Fact::AttemptRecorded(_)) {
                        apxm_runtime_protocol::NodeExecutionStatus::Succeeded
                    } else {
                        apxm_runtime_protocol::NodeExecutionStatus::Running
                    }
                });
                self.index_node(
                    &invocation_ref,
                    &invocation_id,
                    node_id,
                    air_node_id,
                    parent,
                    occurrence,
                    region,
                    attempt,
                    None,
                    status,
                    Commitment::Committed,
                    evidence_tail
                        .get(index)
                        .map(|record| record.evidence_ref.clone()),
                );
            }
        }
        for observation in &request.tuple.observations {
            let Ok(observation) =
                serde_json::from_value::<ExecutionObservation>(observation.clone())
            else {
                continue;
            };
            let Some(node_id) = observation.node_execution_id.as_ref() else {
                continue;
            };
            if let Some(invocation) = self.invocations.get_mut(&invocation_ref)
                && !invocation.node_execution_refs.contains(node_id)
            {
                invocation.node_execution_refs.push(node_id.clone());
            }
            self.apply_observation(&invocation_ref, &observation);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn index_node(
        &mut self,
        invocation_ref: &str,
        invocation_id: &apxm_runtime_protocol::ProgramInvocationId,
        node_id: String,
        air_node_id: Option<String>,
        parent: Option<String>,
        occurrence: Option<String>,
        region: Option<String>,
        attempt: Option<String>,
        output_ref: Option<OutputRef>,
        status: apxm_runtime_protocol::NodeExecutionStatus,
        commitment: Commitment,
        evidence_ref: Option<apxm_runtime_protocol::EvidenceRef>,
    ) {
        let Ok(node_execution_id) = NodeExecutionId::new(node_id.clone()) else {
            return;
        };
        let key = format!("{invocation_ref}:{node_id}");
        if air_node_id.is_none() && !self.node_executions.contains_key(&key) {
            return;
        }
        {
            let inspection = self.node_executions.entry(key).or_insert_with(|| {
                apxm_runtime_protocol::NodeExecutionInspection {
                    contract: apxm_runtime_protocol::NODE_EXECUTION_INSPECTION_CONTRACT.to_owned(),
                    program_invocation_id: invocation_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                    air_node_id: air_node_id.clone().expect("air node checked above"),
                    parent_node_execution_id: parent
                        .as_deref()
                        .and_then(|value| NodeExecutionId::new(value).ok()),
                    occurrence_id: occurrence
                        .as_deref()
                        .and_then(|value| OccurrenceId::new(value).ok()),
                    region_occurrence_id: region.as_deref().and_then(|value| {
                        apxm_runtime_protocol::RegionOccurrenceId::new(value).ok()
                    }),
                    status,
                    commitment,
                    input_ref: None,
                    prompt_or_request_ref: None,
                    provisional_response_ref: None,
                    final_response_ref: None,
                    output_ref: output_ref.clone(),
                    attempt_refs: Vec::new(),
                    metrics_ref: None,
                    usage_ref: None,
                    policy_ref: None,
                    approval_ref: None,
                    error_ref: None,
                    trace_ref: None,
                    evidence_refs: Vec::new(),
                    child_program_refs: Vec::new(),
                }
            });
            if let Some(air_node_id) = air_node_id {
                inspection.air_node_id = air_node_id;
            }
            if let Some(occurrence) = occurrence.and_then(|value| OccurrenceId::new(value).ok()) {
                inspection.occurrence_id = Some(occurrence);
            }
            if let Some(region) =
                region.and_then(|value| apxm_runtime_protocol::RegionOccurrenceId::new(value).ok())
            {
                inspection.region_occurrence_id = Some(region);
            }
            if let Some(output_ref) = output_ref {
                inspection.output_ref = Some(output_ref);
            }
            if let Some(evidence_ref) = evidence_ref
                && !inspection.evidence_refs.contains(&evidence_ref)
            {
                inspection.evidence_refs.push(evidence_ref);
            }
            if let Some(attempt) =
                attempt.and_then(|value| apxm_runtime_protocol::AttemptId::new(value).ok())
                && !inspection.attempt_refs.contains(&attempt)
            {
                inspection.attempt_refs.push(attempt);
            }
        }
        if let Some(invocation) = self.invocations.get_mut(invocation_ref)
            && !invocation.node_execution_refs.contains(&node_execution_id)
        {
            invocation.node_execution_refs.push(node_execution_id);
        }
    }

    fn apply_observation(&mut self, invocation_ref: &str, observation: &ExecutionObservation) {
        let Some(node_id) = observation.node_execution_id.as_ref() else {
            return;
        };
        let key = format!("{invocation_ref}:{}", node_id.as_str());
        let Some(inspection) = self.node_executions.get_mut(&key) else {
            // An observation carries dynamic coordinates, but not the static
            // AIR node identity required by NodeExecutionInspection. Do not
            // manufacture an "unknown" node row from telemetry alone.
            return;
        };
        if let Some(occurrence) = observation.occurrence_id.clone() {
            inspection.occurrence_id = Some(occurrence);
        }
        if let Some(region) = observation.region_occurrence_id.clone() {
            inspection.region_occurrence_id = Some(region);
        }
        if let Some(attempt) = observation.attempt_id.clone()
            && !inspection.attempt_refs.contains(&attempt)
        {
            inspection.attempt_refs.push(attempt);
        }
        if let Some(evidence_ref) = observation.evidence_ref.clone()
            && !inspection.evidence_refs.contains(&evidence_ref)
        {
            inspection.evidence_refs.push(evidence_ref);
        }
        match observation.observation_kind {
            ObservationKind::NodeStarted
            | ObservationKind::ModelAttempt
            | ObservationKind::CapabilityAttempt
            | ObservationKind::ProgramAttempt
            | ObservationKind::ApprovalRequested
            | ObservationKind::ApprovalResolved
            | ObservationKind::EventResumed => {
                inspection.status = fold_node_status(
                    inspection.status,
                    apxm_runtime_protocol::NodeExecutionStatus::Running,
                );
            }
            ObservationKind::EventWaiting => {
                inspection.status = fold_node_status(
                    inspection.status,
                    apxm_runtime_protocol::NodeExecutionStatus::Waiting,
                );
            }
            ObservationKind::OutcomeUnknown => {
                inspection.status = fold_node_status(
                    inspection.status,
                    apxm_runtime_protocol::NodeExecutionStatus::Unknown,
                );
            }
            ObservationKind::InvocationCancelled => {
                inspection.status = fold_node_status(
                    inspection.status,
                    apxm_runtime_protocol::NodeExecutionStatus::Cancelled,
                );
            }
            ObservationKind::InvocationFailed => {
                inspection.status = fold_node_status(
                    inspection.status,
                    apxm_runtime_protocol::NodeExecutionStatus::Failed,
                );
            }
            ObservationKind::ContentPublished => {
                inspection
                    .provisional_response_ref
                    .clone_from(&observation.content_ref);
            }
            ObservationKind::ContentCommitted => {
                inspection.status = fold_node_status(
                    inspection.status,
                    apxm_runtime_protocol::NodeExecutionStatus::Succeeded,
                );
                inspection.final_response_ref = observation.content_ref.clone().or_else(|| {
                    observation
                        .output_ref
                        .as_ref()
                        .and_then(|reference| ContentRef::new(reference.as_str()).ok())
                });
                inspection.output_ref.clone_from(&observation.output_ref);
            }
            ObservationKind::TerminalCommitted => {
                inspection.status = fold_node_status(
                    inspection.status,
                    apxm_runtime_protocol::NodeExecutionStatus::Succeeded,
                );
            }
            _ => {}
        }
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

fn fold_invocation_status(
    prior: apxm_runtime_protocol::ProgramInvocationStatus,
    next: apxm_runtime_protocol::ProgramInvocationStatus,
    explicit_resolution: bool,
) -> apxm_runtime_protocol::ProgramInvocationStatus {
    use apxm_runtime_protocol::ProgramInvocationStatus;
    match prior {
        ProgramInvocationStatus::OutcomeUnknown => {
            if explicit_resolution
                && matches!(
                    next,
                    ProgramInvocationStatus::CommittedReturn
                        | ProgramInvocationStatus::Failed
                        | ProgramInvocationStatus::Cancelled
                )
            {
                next
            } else {
                prior
            }
        }
        ProgramInvocationStatus::CommittedYield => {
            if matches!(
                next,
                ProgramInvocationStatus::CommittedReturn
                    | ProgramInvocationStatus::Failed
                    | ProgramInvocationStatus::Cancelled
                    | ProgramInvocationStatus::OutcomeUnknown
            ) {
                next
            } else {
                prior
            }
        }
        ProgramInvocationStatus::CommittedReturn
        | ProgramInvocationStatus::Failed
        | ProgramInvocationStatus::Cancelled => prior,
        _ => next,
    }
}

fn fact_is_explicit_invocation_resolution(fact: &Fact) -> bool {
    match fact {
        Fact::InvocationCommitted(fact) => matches!(
            fact.invocation_state,
            Some(
                apxm_program::runtime_evidence::InvocationState::CommittedReturn
                    | apxm_program::runtime_evidence::InvocationState::Failed
                    | apxm_program::runtime_evidence::InvocationState::Cancelled
            )
        ),
        Fact::InvocationFailed(_) | Fact::InvocationCancelled(_) => true,
        Fact::InvocationStateChanged(fact) => matches!(
            fact.invocation_state,
            Some(
                apxm_program::runtime_evidence::InvocationState::CommittedReturn
                    | apxm_program::runtime_evidence::InvocationState::Failed
                    | apxm_program::runtime_evidence::InvocationState::Cancelled
            )
        ),
        _ => false,
    }
}

fn fold_node_status(
    prior: apxm_runtime_protocol::NodeExecutionStatus,
    next: apxm_runtime_protocol::NodeExecutionStatus,
) -> apxm_runtime_protocol::NodeExecutionStatus {
    use apxm_runtime_protocol::NodeExecutionStatus;
    if matches!(
        prior,
        NodeExecutionStatus::Succeeded
            | NodeExecutionStatus::Failed
            | NodeExecutionStatus::Cancelled
            | NodeExecutionStatus::Unknown
    ) {
        return prior;
    }
    next
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
        "observations": tuple.observations,
    }))
    .map_err(|e| CommitLocalError::Codec(e.to_string()))?
    .len();
    if bytes > MAX_TUPLE_BYTES {
        return Err(CommitLocalError::TupleTooLarge { bytes });
    }
    Ok(())
}

fn validate_tuple_session_output_refs(values: &[Value]) -> Result<(), CommitLocalError> {
    for value in values {
        let typed = value.get("ref_type").and_then(Value::as_str) == Some("SessionOutputRef")
            || value.get("contract").and_then(Value::as_str) == Some(SESSION_OUTPUT_REF_CONTRACT);
        if !typed {
            continue;
        }
        let output_ref: SessionOutputRef =
            serde_json::from_value(value.clone()).map_err(|error| {
                CommitLocalError::InvalidOutput(format!("invalid Session Output metadata: {error}"))
            })?;
        output_ref
            .validate()
            .map_err(|error| CommitLocalError::InvalidOutput(error.to_string()))?;
    }
    Ok(())
}

struct ObservationEventWaits<'a> {
    current: Option<&'a Value>,
    prior: Option<&'a Value>,
}

fn validate_observation_batch(
    observations: &[ExecutionObservation],
    invocation: &str,
    next_sequence: u64,
    output_values: &[Value],
    evidence_records: &[EvidenceRecord],
    evidence_position_ref: &str,
    event_waits: ObservationEventWaits<'_>,
) -> Result<(), CommitLocalError> {
    let mut ids = HashSet::new();
    let output_refs = output_values
        .iter()
        .filter_map(|value| value.get("ref").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    let evidence_refs = evidence_records
        .iter()
        .map(|record| record.evidence_ref.as_str())
        .collect::<HashSet<_>>();
    let current_event = event_waits
        .current
        .map(event_coordinate_from_wait)
        .transpose()?
        .flatten();
    let prior_event = event_waits
        .prior
        .map(event_coordinate_from_wait)
        .transpose()?
        .flatten();
    let mut previous = next_sequence.saturating_sub(1);
    for observation in observations {
        observation
            .validate()
            .map_err(|error| CommitLocalError::InvalidRequest(error.to_string()))?;
        if observation.program_invocation_id.as_str() != invocation {
            return Err(CommitLocalError::InvalidRequest(
                "observation invocation does not match the commit invocation".into(),
            ));
        }
        if observation.sequence <= previous {
            return Err(CommitLocalError::InvalidRequest(
                "observation sequence is not strictly increasing".into(),
            ));
        }
        let expected_id = format!("observation.{invocation}.{}", observation.sequence);
        if observation.observation_id.as_str() != expected_id {
            return Err(CommitLocalError::InvalidRequest(
                "observation id does not bind to invocation and sequence".into(),
            ));
        }
        if !ids.insert(observation.observation_id.as_str()) {
            return Err(CommitLocalError::InvalidRequest(
                "observation id is duplicated in the durable batch".into(),
            ));
        }
        validate_observation_output_binding(observation, output_values, &output_refs)?;
        let expected_event = match observation.observation_kind {
            ObservationKind::EventWaiting => current_event.as_ref(),
            ObservationKind::EventResumed => prior_event.as_ref(),
            _ => None,
        };
        let same_commit_event_handoff = observation.event_ref.as_ref().is_some_and(|event_ref| {
            observations
                .iter()
                .any(|candidate| match observation.observation_kind {
                    ObservationKind::EventWaiting => {
                        candidate.observation_kind == ObservationKind::EventResumed
                            && candidate.sequence > observation.sequence
                            && candidate.event_ref.as_ref() == Some(event_ref)
                    }
                    ObservationKind::EventResumed => {
                        candidate.observation_kind == ObservationKind::EventWaiting
                            && candidate.sequence < observation.sequence
                            && candidate.event_ref.as_ref() == Some(event_ref)
                    }
                    _ => false,
                })
        });
        let event_matches = same_commit_event_handoff
            || expected_event
                .is_some_and(|expected| observation.event_ref.as_ref() == Some(expected));
        if matches!(
            observation.observation_kind,
            ObservationKind::EventWaiting | ObservationKind::EventResumed
        ) && !event_matches
        {
            let source = if observation.observation_kind == ObservationKind::EventWaiting {
                "current commit event_wait"
            } else {
                "prior committed event_wait"
            };
            return Err(CommitLocalError::InvalidRequest(format!(
                "observation event ref does not match the {source} coordinates"
            )));
        }
        if observation.evidence_ref.as_ref().is_some_and(|reference| {
            !evidence_refs.contains(reference.as_str())
                // The execution driver may point at the evidence position
                // that this same commit will durably append. Keep that
                // forward reference exact; arbitrary evidence refs remain
                // rejected.
                && reference.as_str() != evidence_position_ref
        }) {
            return Err(CommitLocalError::InvalidRequest(
                "observation evidence ref is not present in the commit evidence batch".into(),
            ));
        }
        previous = observation.sequence;
    }
    Ok(())
}

fn event_coordinate_from_wait(
    value: &Value,
) -> Result<Option<apxm_runtime_protocol::EventObservationRef>, CommitLocalError> {
    let (event_ref, coordinates) = match value.get("event_ref") {
        Some(Value::String(event_ref)) => (event_ref.as_str(), value),
        Some(event @ Value::Object(object)) => {
            let Some(event_ref) = object.get("event_ref").and_then(Value::as_str) else {
                return Ok(None);
            };
            (event_ref, event)
        }
        _ => return Ok(None),
    };
    let generation = match coordinates.get("generation") {
        None => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| {
            CommitLocalError::InvalidRequest("invalid event_wait generation coordinate".to_owned())
        })?),
    };
    let occurrence_id = match coordinates.get("occurrence_id") {
        None => None,
        Some(value) => {
            let value = value.as_str().ok_or_else(|| {
                CommitLocalError::InvalidRequest(
                    "invalid event_wait occurrence coordinate".to_owned(),
                )
            })?;
            Some(OccurrenceId::new(value.to_owned()).map_err(|error| {
                CommitLocalError::InvalidRequest(format!(
                    "invalid event_wait occurrence coordinate: {error}"
                ))
            })?)
        }
    };
    let coordinate = apxm_runtime_protocol::EventObservationRef {
        event_ref: event_ref.to_owned(),
        generation,
        occurrence_id,
    };
    coordinate
        .validate()
        .map_err(|error| CommitLocalError::InvalidRequest(error.to_string()))?;
    Ok(Some(coordinate))
}

fn validate_observation_output_binding(
    observation: &ExecutionObservation,
    output_values: &[Value],
    output_refs: &HashSet<&str>,
) -> Result<(), CommitLocalError> {
    let reference = match (&observation.content_ref, &observation.output_ref) {
        (Some(content_ref), Some(output_ref)) if content_ref.as_str() != output_ref.as_str() => {
            return Err(CommitLocalError::InvalidRequest(
                "observation content and output refs do not identify one tuple output".into(),
            ));
        }
        (Some(content_ref), _) => Some(content_ref.as_str()),
        (None, Some(output_ref)) => Some(output_ref.as_str()),
        (None, None) => None,
    };
    let Some(reference) = reference else {
        return Ok(());
    };
    if !output_refs.contains(reference) {
        return Err(CommitLocalError::InvalidRequest(
            "observation output ref is not present in the commit tuple".into(),
        ));
    }
    let Some(value) = output_values
        .iter()
        .find(|value| value.get("ref").and_then(Value::as_str) == Some(reference))
    else {
        return Err(CommitLocalError::InvalidRequest(
            "observation output ref has no tuple metadata".into(),
        ));
    };
    let output = serde_json::from_value::<SessionOutputRef>(value.clone()).map_err(|error| {
        CommitLocalError::InvalidRequest(format!(
            "observation output ref has invalid tuple metadata: {error}"
        ))
    })?;
    if output.reference.as_str() != reference
        || observation.node_execution_id != output.node_execution_id
        || observation.occurrence_id != output.occurrence_id
    {
        return Err(CommitLocalError::InvalidRequest(
            "observation output coordinates do not match the commit tuple".into(),
        ));
    }
    Ok(())
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn keyed_digest(key: &[u8; 32], bytes: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut inner = [0x36_u8; BLOCK];
    let mut outer = [0x5c_u8; BLOCK];
    for (index, value) in key.iter().enumerate() {
        inner[index] ^= value;
        outer[index] ^= value;
    }
    let mut inner_hash = Sha256::new();
    inner_hash.update(inner);
    inner_hash.update(bytes);
    let inner_digest = inner_hash.finalize();
    let mut outer_hash = Sha256::new();
    outer_hash.update(outer);
    outer_hash.update(inner_digest);
    format!("sha256:{:x}", outer_hash.finalize())
}

fn read_authorization(request: &ExecutionReadRequest) -> ReadAuthorization {
    match request {
        ExecutionReadRequest::ObservationSubscribe {
            context,
            program_invocation_id,
            ..
        } => ReadAuthorization {
            context: context.clone(),
            operation: ReadOperation::Observation,
            target: ReadTarget::Observation {
                program_invocation_ref: program_invocation_id.as_str().to_owned(),
            },
        },
        ExecutionReadRequest::ProgramInvocationInspect {
            context,
            program_invocation_id,
            node_execution_id,
        } => ReadAuthorization {
            context: context.clone(),
            operation: ReadOperation::Inspection,
            target: ReadTarget::Invocation {
                program_invocation_ref: program_invocation_id.as_str().to_owned(),
                node_execution_ref: node_execution_id
                    .as_ref()
                    .map(|reference| reference.as_str().to_owned()),
            },
        },
        ExecutionReadRequest::ContentRead {
            context,
            content_ref,
        } => ReadAuthorization {
            context: context.clone(),
            operation: ReadOperation::Content,
            target: ReadTarget::Content {
                content_ref: content_ref.as_str().to_owned(),
            },
        },
        ExecutionReadRequest::OutputRead {
            context,
            output_ref,
        } => ReadAuthorization {
            context: context.clone(),
            operation: ReadOperation::Output,
            target: ReadTarget::Output {
                output_ref: output_ref.as_str().to_owned(),
            },
        },
        ExecutionReadRequest::EvidenceRead {
            context,
            program_invocation_id,
            ..
        } => ReadAuthorization {
            context: context.clone(),
            operation: ReadOperation::Evidence,
            target: ReadTarget::Evidence {
                program_invocation_ref: program_invocation_id.as_str().to_owned(),
            },
        },
    }
}

fn keyed_cursor_token(
    key: &[u8; 32],
    operation: ReadOperation,
    invocation: &str,
    context: &ReadContext,
    filter: &str,
    position: u64,
) -> String {
    let operation = match operation {
        ReadOperation::Observation => "observation",
        ReadOperation::Inspection => "inspection",
        ReadOperation::Content => "content",
        ReadOperation::Output => "output",
        ReadOperation::Evidence => "evidence",
    };
    let payload = format!(
        "cursor:v1:{operation}:{invocation}:{filter}:{}:{}:{}:{position}",
        context.scope_ref.as_str(),
        context.principal_ref.as_str(),
        context.grant_ref.as_str(),
    );
    keyed_digest(key, payload.as_bytes())
}

#[allow(clippy::too_many_arguments)]
fn validate_after_cursor(
    cursor: Option<&ExecutionCursor>,
    key: &[u8; 32],
    operation: ReadOperation,
    invocation: &str,
    context: &ReadContext,
    filter: &str,
    floor: u64,
    high_watermark: u64,
) -> Result<u64, CommitLocalError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    if cursor.position > high_watermark {
        return Err(CommitLocalError::InvalidCursor(
            "cursor is ahead of the high watermark".into(),
        ));
    }
    let expected = keyed_cursor_token(key, operation, invocation, context, filter, cursor.position);
    if cursor.token != expected {
        return Err(CommitLocalError::InvalidCursor(
            "cursor scope or filter does not match this read".into(),
        ));
    }
    if floor > 0 && cursor.position.saturating_add(1) < floor {
        return Err(CommitLocalError::RetentionGap { floor });
    }
    Ok(cursor.position)
}

#[allow(clippy::too_many_arguments)]
fn page_cursors(
    key: &[u8; 32],
    operation: ReadOperation,
    invocation: &str,
    context: &ReadContext,
    filter: &str,
    positions: &[u64],
    next_position: Option<u64>,
    high: u64,
    floor: u64,
) -> Result<
    (
        Vec<ExecutionCursor>,
        Option<ExecutionCursor>,
        ExecutionCursor,
        ExecutionCursor,
    ),
    CommitLocalError,
> {
    let cursor = |position| {
        ExecutionCursor::new(
            position,
            keyed_cursor_token(key, operation, invocation, context, filter, position),
        )
        .map_err(|error| CommitLocalError::InvalidCursor(error.to_string()))
    };
    Ok((
        positions
            .iter()
            .copied()
            .map(cursor)
            .collect::<Result<_, _>>()?,
        next_position.map(cursor).transpose()?,
        cursor(high)?,
        cursor(floor)?,
    ))
}

fn page_observations(
    mut items: Vec<ExecutionObservation>,
    context: &ReadContext,
    invocation: &str,
    after: Option<&ExecutionCursor>,
    limit: u32,
    key: &[u8; 32],
    durable_high: u64,
) -> Result<ExecutionPage<ExecutionObservation>, CommitLocalError> {
    items.sort_by_key(|item| item.sequence);
    let floor = items.first().map_or(0, |item| item.sequence);
    let high = durable_high.max(items.last().map_or(0, |item| item.sequence));
    let after = validate_after_cursor(
        after,
        key,
        ReadOperation::Observation,
        invocation,
        context,
        "all",
        floor,
        high,
    )?;
    let start = items.partition_point(|item| item.sequence <= after);
    let end = (start + limit as usize).min(items.len());
    let has_more = end < items.len();
    let positions: Vec<u64> = items[start..end].iter().map(|item| item.sequence).collect();
    // The cursor is the last delivered position.  Consumers resume with an
    // exclusive `after_cursor`, so advancing it here would skip that row.
    let next = has_more.then(|| items[end - 1].sequence);
    let (cursors, next_cursor, high_watermark, retention_floor) = page_cursors(
        key,
        ReadOperation::Observation,
        invocation,
        context,
        "all",
        &positions,
        next,
        high,
        floor,
    )?;
    let mut page_items = items[start..end].to_vec();
    for (item, cursor) in page_items.iter_mut().zip(&cursors) {
        item.cursor = cursor.clone();
    }
    Ok(ExecutionPage {
        items: page_items,
        item_cursors: cursors,
        next_cursor,
        high_watermark,
        retention_floor,
        has_more,
    })
}

fn page_evidence(
    items: Vec<EvidenceRecord>,
    context: &ReadContext,
    invocation: &str,
    after: Option<&ExecutionCursor>,
    limit: u32,
    key: &[u8; 32],
    durable_high: u64,
) -> Result<ExecutionPage<EvidenceRecord>, CommitLocalError> {
    for item in &items {
        item.validate()
            .map_err(|error| CommitLocalError::InvalidRead(error.to_string()))?;
    }
    let floor = items.first().map_or(0, |item| item.sequence);
    let high = durable_high.max(items.last().map_or(0, |item| item.sequence));
    let after = validate_after_cursor(
        after,
        key,
        ReadOperation::Evidence,
        invocation,
        context,
        "all",
        floor,
        high,
    )?;
    let start = items.partition_point(|item| item.sequence <= after);
    let end = (start + limit as usize).min(items.len());
    let has_more = end < items.len();
    let positions: Vec<u64> = items[start..end].iter().map(|item| item.sequence).collect();
    let next = has_more.then(|| items[end - 1].sequence);
    let (item_cursors, next_cursor, high_watermark, retention_floor) = page_cursors(
        key,
        ReadOperation::Evidence,
        invocation,
        context,
        "all",
        &positions,
        next,
        high,
        floor,
    )?;
    Ok(ExecutionPage {
        items: items[start..end].to_vec(),
        item_cursors,
        next_cursor,
        high_watermark,
        retention_floor,
        has_more,
    })
}

fn typed_output_ref(output: &StoredOutput) -> Result<SessionOutputRef, CommitLocalError> {
    if output.prepared.program_instance_ref.as_str() != output.program_instance_ref
        || output.prepared.program_invocation_ref.as_str() != output.program_invocation_ref
    {
        return Err(CommitLocalError::OutputScopeMismatch {
            output_ref: output.prepared.output_ref.clone(),
        });
    }
    let typed = SessionOutputRef {
        contract: SESSION_OUTPUT_REF_CONTRACT.to_owned(),
        ref_type: "SessionOutputRef".to_owned(),
        reference: OutputRef::new(output.prepared.output_ref.clone())
            .map_err(|error| CommitLocalError::InvalidOutput(error.to_string()))?,
        program_instance_id: apxm_runtime_protocol::ProgramInstanceId::new(
            output.program_instance_ref.clone(),
        )
        .map_err(|error| CommitLocalError::InvalidOutput(error.to_string()))?,
        program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(
            output.program_invocation_ref.clone(),
        )
        .map_err(|error| CommitLocalError::InvalidOutput(error.to_string()))?,
        node_execution_id: output.prepared.node_execution_id.clone(),
        occurrence_id: output.prepared.occurrence_id.clone(),
        content_digest: output.prepared.content_digest.clone(),
        byte_length: output.prepared.byte_length as u64,
        media_type: output.prepared.media_type.clone(),
        visibility: OutputVisibility::Committed,
        access_scope_ref: output.prepared.access_scope_ref.clone(),
        disclosure_ref: output.prepared.disclosure_ref.clone(),
    };
    typed
        .validate()
        .map_err(|error| CommitLocalError::InvalidOutput(error.to_string()))?;
    if typed.visibility != OutputVisibility::Committed
        || typed.byte_length != output.content.len() as u64
        || typed.content_digest != sha256_digest(&output.content)
    {
        return Err(CommitLocalError::InvalidOutput(
            "committed Session Output metadata does not match bytes".into(),
        ));
    }
    Ok(typed)
}

fn evidence_kind(fact: &Fact) -> EvidenceFactKind {
    use apxm_program::runtime_evidence::FactKind;
    match fact.kind() {
        Some(FactKind::InstanceStateChanged) => EvidenceFactKind::InstanceStateChanged,
        Some(FactKind::InvocationStateChanged) => EvidenceFactKind::InvocationStateChanged,
        Some(FactKind::InstanceCreated) => EvidenceFactKind::InstanceCreated,
        Some(FactKind::InvocationAdmitted) => EvidenceFactKind::InvocationAdmitted,
        Some(FactKind::ChildAttached) => EvidenceFactKind::ChildAttached,
        Some(FactKind::AttemptRecorded) => EvidenceFactKind::AttemptRecorded,
        Some(FactKind::InvocationAttemptRecorded) => EvidenceFactKind::InvocationAttemptRecorded,
        Some(FactKind::CapabilityAttemptRecorded) => EvidenceFactKind::CapabilityAttemptRecorded,
        Some(FactKind::InvocationCommitted) => EvidenceFactKind::InvocationCommitted,
        Some(FactKind::InvocationFailed) => EvidenceFactKind::InvocationFailed,
        Some(FactKind::InvocationCancelled) => EvidenceFactKind::InvocationCancelled,
        Some(FactKind::EventCreated) => EvidenceFactKind::EventCreated,
        Some(FactKind::EventAwaitRegistered) => EvidenceFactKind::EventAwaitRegistered,
        Some(FactKind::InvocationParked) => EvidenceFactKind::InvocationParked,
        Some(FactKind::EventTerminal) => EvidenceFactKind::EventTerminal,
        Some(FactKind::InvocationResumed) => EvidenceFactKind::InvocationResumed,
        Some(FactKind::InstanceClosed) => EvidenceFactKind::InstanceClosed,
        Some(FactKind::InstanceCancelled) => EvidenceFactKind::InstanceCancelled,
        Some(FactKind::EffectOutcomeUnknown) => EvidenceFactKind::EffectOutcomeUnknown,
        Some(FactKind::DeliveryRecorded) => EvidenceFactKind::DeliveryRecorded,
        Some(FactKind::RegionOccurrenceStarted) => EvidenceFactKind::RegionOccurrenceStarted,
        Some(FactKind::NodeExecutionRecorded) => EvidenceFactKind::NodeExecutionRecorded,
        Some(FactKind::HookExecuted) => EvidenceFactKind::HookExecuted,
        Some(FactKind::ContextTransitioned) => EvidenceFactKind::ContextTransitioned,
        None => EvidenceFactKind::LoopIterationCompleted,
    }
}

fn fact_belongs_to_invocation(fact: &Fact, invocation: &str) -> bool {
    match fact {
        Fact::NodeExecutionRecorded(record) => record.program_invocation_id == invocation,
        Fact::AttemptRecorded(record) => record.program_invocation_id == invocation,
        Fact::LoopIterationCompleted(record) => record.program_invocation_id == invocation,
        Fact::InstanceStateChanged(value)
        | Fact::InvocationStateChanged(value)
        | Fact::InstanceCreated(value)
        | Fact::InvocationAdmitted(value)
        | Fact::ChildAttached(value)
        | Fact::InvocationAttemptRecorded(value)
        | Fact::CapabilityAttemptRecorded(value)
        | Fact::InvocationCommitted(value)
        | Fact::InvocationFailed(value)
        | Fact::InvocationCancelled(value)
        | Fact::EventCreated(value)
        | Fact::EventAwaitRegistered(value)
        | Fact::InvocationParked(value)
        | Fact::EventTerminal(value)
        | Fact::InvocationResumed(value)
        | Fact::InstanceClosed(value)
        | Fact::InstanceCancelled(value)
        | Fact::EffectOutcomeUnknown(value)
        | Fact::DeliveryRecorded(value)
        | Fact::RegionOccurrenceStarted(value)
        | Fact::HookExecuted(value)
        | Fact::ContextTransitioned(value) => fact_id_binds_invocation(&value.fact_id, invocation),
    }
}

/// Validate every identifier that can be projected into the owner-local
/// evidence/index joins before any records are staged.  `Fact` deliberately
/// keeps these fields as strings because it is the program-facing wire shape;
/// the commit adapter must not turn a malformed coordinate into `None` and
/// silently publish an incomplete inspection row.
fn validate_fact_coordinates(fact: &Fact) -> Result<(), CommitLocalError> {
    fn validate(field: &str, value: &str) -> Result<(), CommitLocalError> {
        if is_identifier(value) {
            Ok(())
        } else {
            Err(CommitLocalError::InvalidRequest(format!(
                "evidence fact {field} is not a valid identifier"
            )))
        }
    }

    fn validate_optional(field: &str, value: Option<&String>) -> Result<(), CommitLocalError> {
        value.map_or(Ok(()), |value| validate(field, value))
    }

    match fact {
        Fact::NodeExecutionRecorded(record) => {
            validate("fact_id", &record.fact_id)?;
            validate("program_invocation_id", &record.program_invocation_id)?;
            validate("node_execution_id", &record.node_execution_id)?;
            validate("air_node_id", &record.air_node_id)?;
            validate_optional(
                "parent_node_execution_id",
                record.parent_node_execution_id.as_ref(),
            )?;
            match &record.execution_scope {
                apxm_program::runtime_evidence::NodeExecutionScope::NonLoop => {}
                apxm_program::runtime_evidence::NodeExecutionScope::Loop {
                    region_occurrence_id,
                    static_region_id,
                    loop_memberships,
                } => {
                    validate("region_occurrence_id", region_occurrence_id)?;
                    validate("static_region_id", static_region_id)?;
                    if loop_memberships.is_empty() {
                        return Err(CommitLocalError::InvalidRequest(
                            "evidence fact loop_memberships must be non-empty".into(),
                        ));
                    }
                    for membership in loop_memberships {
                        validate("static_loop_id", &membership.static_loop_id)?;
                        validate("loop_occurrence_id", &membership.loop_occurrence_id)?;
                    }
                }
            }
        }
        Fact::AttemptRecorded(record) => {
            validate("fact_id", &record.fact_id)?;
            validate("program_invocation_id", &record.program_invocation_id)?;
            validate("node_execution_id", &record.node_execution_id)?;
            validate("air_node_id", &record.air_node_id)?;
            validate("attempt_id", &record.attempt_id)?;
        }
        Fact::LoopIterationCompleted(record) => {
            validate("fact_id", &record.fact_id)?;
            validate("static_loop_id", &record.static_loop_id)?;
            validate("loop_occurrence_id", &record.loop_occurrence_id)?;
            validate("program_invocation_id", &record.program_invocation_id)?;
            if record.causal_node_execution_ids.is_empty() {
                return Err(CommitLocalError::InvalidRequest(
                    "evidence fact causal_node_execution_ids must be non-empty".into(),
                ));
            }
            for node_id in &record.causal_node_execution_ids {
                validate("causal_node_execution_id", node_id)?;
            }
        }
        Fact::InstanceStateChanged(value)
        | Fact::InvocationStateChanged(value)
        | Fact::InstanceCreated(value)
        | Fact::InvocationAdmitted(value)
        | Fact::ChildAttached(value)
        | Fact::InvocationAttemptRecorded(value)
        | Fact::CapabilityAttemptRecorded(value)
        | Fact::InvocationCommitted(value)
        | Fact::InvocationFailed(value)
        | Fact::InvocationCancelled(value)
        | Fact::EventCreated(value)
        | Fact::EventAwaitRegistered(value)
        | Fact::InvocationParked(value)
        | Fact::EventTerminal(value)
        | Fact::InvocationResumed(value)
        | Fact::InstanceClosed(value)
        | Fact::InstanceCancelled(value)
        | Fact::EffectOutcomeUnknown(value)
        | Fact::DeliveryRecorded(value)
        | Fact::RegionOccurrenceStarted(value)
        | Fact::HookExecuted(value)
        | Fact::ContextTransitioned(value) => {
            validate("fact_id", &value.fact_id)?;
            validate_optional("node_execution_id", value.node_execution_id.as_ref())?;
            validate_optional("air_node_id", value.air_node_id.as_ref())?;
            validate_optional(
                "parent_node_execution_id",
                value.parent_node_execution_id.as_ref(),
            )?;
            validate_optional("attempt_id", value.attempt_id.as_ref())?;
            validate_optional("region_occurrence_id", value.region_occurrence_id.as_ref())?;
            validate_optional("static_region_id", value.static_region_id.as_ref())?;
            if let Some(loop_memberships) = value.loop_memberships.as_ref() {
                for membership in loop_memberships {
                    validate("static_loop_id", &membership.static_loop_id)?;
                    validate("loop_occurrence_id", &membership.loop_occurrence_id)?;
                }
            }
            validate_optional("hook_execution_id", value.hook_execution_id.as_ref())?;
            validate_optional("hook_id", value.hook_id.as_ref())?;
            validate_optional(
                "context_transition_id",
                value.context_transition_id.as_ref(),
            )?;
            validate_optional("capability_ref", value.capability_ref.as_ref())?;
        }
    }
    Ok(())
}

fn fact_id_binds_invocation(fact_id: &str, invocation: &str) -> bool {
    fact_id
        .strip_prefix("fact.")
        .is_some_and(|suffix| suffix.starts_with(&format!("{invocation}.")))
}

#[allow(clippy::type_complexity)]
fn node_coordinates(
    fact: &Fact,
) -> Option<(
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
)> {
    match fact {
        Fact::NodeExecutionRecorded(record) => {
            let (occurrence, region) = match &record.execution_scope {
                apxm_program::runtime_evidence::NodeExecutionScope::Loop {
                    region_occurrence_id,
                    ..
                } => (None, Some(region_occurrence_id.clone())),
                apxm_program::runtime_evidence::NodeExecutionScope::NonLoop => (None, None),
            };
            Some((
                record.node_execution_id.clone(),
                Some(record.air_node_id.clone()),
                record.parent_node_execution_id.clone(),
                occurrence,
                region,
                None,
            ))
        }
        Fact::AttemptRecorded(record) => Some((
            record.node_execution_id.clone(),
            Some(record.air_node_id.clone()),
            None,
            None,
            None,
            Some(record.attempt_id.clone()),
        )),
        Fact::InvocationFailed(value)
        | Fact::InvocationCancelled(value)
        | Fact::EffectOutcomeUnknown(value) => value.node_execution_id.clone().map(|node| {
            (
                node,
                value.air_node_id.clone(),
                value.parent_node_execution_id.clone(),
                None,
                value.region_occurrence_id.clone(),
                value.attempt_id.clone(),
            )
        }),
        _ => None,
    }
}

fn node_status(fact: &Fact) -> Option<apxm_runtime_protocol::NodeExecutionStatus> {
    match fact {
        Fact::InvocationFailed(_) => Some(apxm_runtime_protocol::NodeExecutionStatus::Failed),
        Fact::InvocationCancelled(_) => Some(apxm_runtime_protocol::NodeExecutionStatus::Cancelled),
        Fact::EffectOutcomeUnknown(_) => Some(apxm_runtime_protocol::NodeExecutionStatus::Unknown),
        _ => None,
    }
}

#[cfg(test)]
mod status_tests {
    use super::fold_invocation_status;
    use apxm_runtime_protocol::ProgramInvocationStatus;

    #[test]
    fn committed_yield_can_progress_to_a_terminal_resolution() {
        assert_eq!(
            fold_invocation_status(
                ProgramInvocationStatus::CommittedYield,
                ProgramInvocationStatus::CommittedReturn,
                true,
            ),
            ProgramInvocationStatus::CommittedReturn
        );
        assert_eq!(
            fold_invocation_status(
                ProgramInvocationStatus::CommittedYield,
                ProgramInvocationStatus::Failed,
                true,
            ),
            ProgramInvocationStatus::Failed
        );
        assert_eq!(
            fold_invocation_status(
                ProgramInvocationStatus::CommittedYield,
                ProgramInvocationStatus::Cancelled,
                true,
            ),
            ProgramInvocationStatus::Cancelled
        );
        assert_eq!(
            fold_invocation_status(
                ProgramInvocationStatus::CommittedYield,
                ProgramInvocationStatus::OutcomeUnknown,
                false,
            ),
            ProgramInvocationStatus::OutcomeUnknown
        );
        assert_eq!(
            fold_invocation_status(
                ProgramInvocationStatus::CommittedYield,
                ProgramInvocationStatus::Running,
                false,
            ),
            ProgramInvocationStatus::CommittedYield
        );
    }

    #[test]
    fn outcome_unknown_requires_an_explicit_terminal_resolution() {
        assert_eq!(
            fold_invocation_status(
                ProgramInvocationStatus::OutcomeUnknown,
                ProgramInvocationStatus::Running,
                false,
            ),
            ProgramInvocationStatus::OutcomeUnknown
        );
        assert_eq!(
            fold_invocation_status(
                ProgramInvocationStatus::OutcomeUnknown,
                ProgramInvocationStatus::CommittedReturn,
                false,
            ),
            ProgramInvocationStatus::OutcomeUnknown
        );
        assert_eq!(
            fold_invocation_status(
                ProgramInvocationStatus::OutcomeUnknown,
                ProgramInvocationStatus::CommittedReturn,
                true,
            ),
            ProgramInvocationStatus::CommittedReturn
        );
    }
}

#[cfg(test)]
mod read_authorization_binding_tests {
    use super::normalize_optional_correlation;

    #[test]
    fn blank_optional_correlation_is_absent() {
        assert_eq!(normalize_optional_correlation(Some(String::new())), None);
        assert_eq!(
            normalize_optional_correlation(Some(" \t\n ".to_owned())),
            None
        );
    }

    #[test]
    fn non_blank_optional_correlation_is_preserved() {
        assert_eq!(
            normalize_optional_correlation(Some("correlation.1".to_owned())),
            Some("correlation.1".to_owned())
        );
    }
}
