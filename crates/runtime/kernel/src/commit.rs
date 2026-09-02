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
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use apxm_program::grammar::{is_digest, is_identifier};
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
/// `runtime_evidence_batch_digest` canonically covers both the evidence facts
/// and the driver-owned observation batch carried by the tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomicWriteSet {
    pub next_program_state_digest: String,
    pub continuation_digest: String,
    pub checkpoint_effect_outcomes_digest: String,
    pub runtime_evidence_batch_digest: String,
    pub usage_facts_digest: String,
    pub session_output_refs_digest: String,
}

/// A continuation payload read together with the digest committed beside it.
///
/// The pair is returned by one port operation so a caller cannot accidentally
/// verify a digest fetched from a different commit record than the payload it
/// is about to resume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedContinuation {
    pub payload: Value,
    pub digest: String,
}

/// Canonicalize a JSON value for digesting and authenticated local records.
/// Object keys are sorted recursively; arrays retain their authored order.
#[must_use]
pub fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json_value).collect()),
        Value::Object(object) => {
            let mut canonical = Map::new();
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (key, value) in entries {
                canonical.insert(key.clone(), canonical_json_value(value));
            }
            Value::Object(canonical)
        }
        scalar => scalar.clone(),
    }
}

/// Serialize one JSON value into its canonical, key-order-independent bytes.
#[must_use]
pub fn canonical_json_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(&canonical_json_value(value))
        .expect("serde_json::Value is always canonically serializable")
}

/// Canonical digest of the exact continuation state carried by a commit.
///
/// The continuation's nested `write_set.continuation_digest` is replaced with
/// a fixed marker before hashing. That field carries this digest, so including
/// it verbatim would create an impossible self-referential digest. Every other
/// continuation field—including the remaining write-set members—is hashed.
#[must_use]
pub fn continuation_digest(payload: Option<&Value>) -> String {
    let normalized = payload.map(normalize_continuation_payload);
    let envelope = json!({
        "schema_version": "apxm.continuation-integrity.v1",
        "present": payload.is_some(),
        "payload": normalized.unwrap_or(Value::Null),
    });
    format!(
        "sha256:{:x}",
        Sha256::digest(canonical_json_bytes(&envelope))
    )
}

/// Canonical digest for the runtime evidence batch and its driver-owned
/// observation records. Observations are an atomic companion to evidence;
/// binding both here prevents an adapter from accepting an unsigned
/// observation side channel.
#[must_use]
pub fn runtime_evidence_and_observation_digest(
    evidence: &[Fact],
    observations: &[Value],
) -> String {
    let envelope = json!({
        "evidence": evidence,
        "observations": observations,
    });
    format!(
        "sha256:{:x}",
        Sha256::digest(canonical_json_bytes(&envelope))
    )
}

/// Canonical digest for the complete ordered Session Output reference member.
/// This is checked even when the tuple contains no output refs, so an empty
/// output set cannot silently carry a stale digest from another request.
#[must_use]
pub fn session_output_refs_digest(output_refs: &[Value]) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(canonical_json_bytes(&Value::Array(output_refs.to_vec())))
    )
}

/// A deterministic evidence reference that can be prepared before the commit
/// is attempted. `index` is zero-based at this boundary; the encoded suffix is
/// one-based. The commit id makes references unique across multiple commits
/// of one invocation while the invocation keeps them scope-bound.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrecommitEvidenceRef {
    pub commit_id: String,
    pub program_invocation_ref: String,
    pub index: u64,
    pub evidence_ref: String,
}

impl PrecommitEvidenceRef {
    pub fn new(
        commit_id: impl Into<String>,
        program_invocation_ref: impl Into<String>,
        index: u64,
    ) -> Result<Self, &'static str> {
        let commit_id = commit_id.into();
        let program_invocation_ref = program_invocation_ref.into();
        if !is_identifier(&commit_id) || !is_identifier(&program_invocation_ref) {
            return Err("invalid evidence reference scope");
        }
        let ordinal = index
            .checked_add(1)
            .ok_or("evidence reference index is exhausted")?;
        let evidence_ref = format!("evidence.{program_invocation_ref}.{commit_id}.{ordinal}");
        if !is_identifier(&evidence_ref) {
            return Err("invalid evidence reference");
        }
        Ok(Self {
            commit_id,
            program_invocation_ref,
            index,
            evidence_ref,
        })
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let expected = Self::new(
            self.commit_id.clone(),
            self.program_invocation_ref.clone(),
            self.index,
        )?;
        if self.evidence_ref != expected.evidence_ref {
            return Err("evidence reference does not match its commit scope and index");
        }
        Ok(())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.evidence_ref
    }
}

/// Contract identity for the product-neutral Session Output staging boundary.
pub const SESSION_OUTPUT_REF_CONTRACT: &str = "apxm.session-output-ref.v1";

/// Visibility supplied while staging Session Output bytes. Staged bytes are
/// never readable; the returned reference is marked committed only after the
/// enclosing Execution Commit wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOutputVisibility {
    Provisional,
    Committed,
}

/// Product-neutral input to the pre-commit Session Output staging port.
/// Dynamic node and occurrence references remain opaque strings here so the
/// kernel does not depend on the service-protocol crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOutputPreparation {
    pub contract: String,
    pub commit_id: String,
    pub program_instance_ref: String,
    pub program_invocation_ref: String,
    pub content: Vec<u8>,
    pub media_type: String,
    pub visibility: SessionOutputVisibility,
    pub node_execution_id: Option<String>,
    pub occurrence_id: Option<String>,
    /// Opaque caller/composition-supplied disclosure scope. APXM persists it
    /// and never interprets product authorization policy.
    pub access_scope_ref: String,
    pub disclosure_ref: Option<String>,
}

impl SessionOutputPreparation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.contract != SESSION_OUTPUT_REF_CONTRACT
            || self.commit_id.trim().is_empty()
            || self.program_instance_ref.trim().is_empty()
            || self.program_invocation_ref.trim().is_empty()
            || self.media_type.trim().is_empty()
            || self.media_type.len() > 255
            || self.media_type.contains(['\n', '\r'])
            || self.visibility != SessionOutputVisibility::Provisional
            || self.access_scope_ref.len() > 256
            || !is_identifier(&self.access_scope_ref)
            || self
                .disclosure_ref
                .as_ref()
                .is_some_and(|reference| reference.len() > 256 || !is_identifier(reference))
            || (self.occurrence_id.is_some() && self.node_execution_id.is_none())
        {
            return Err("invalid Session Output preparation");
        }
        Ok(())
    }
}

/// Opaque validated Session Output reference returned by staging. The adapter
/// must bind this exact value into `ExecutionCommitTuple.output_refs`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedSessionOutputRef {
    pub contract: String,
    pub ref_type: String,
    #[serde(rename = "ref")]
    pub output_ref: String,
    pub program_instance_id: String,
    pub program_invocation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<String>,
    pub access_scope_ref: String,
    pub disclosure_ref: Option<String>,
    pub content_digest: String,
    pub byte_length: u64,
    pub media_type: String,
    #[serde(rename = "commitment")]
    pub visibility: SessionOutputVisibility,
}

impl PreparedSessionOutputRef {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.contract != SESSION_OUTPUT_REF_CONTRACT
            || self.ref_type != "SessionOutputRef"
            || self.output_ref.trim().is_empty()
            || self.program_instance_id.trim().is_empty()
            || self.program_invocation_id.trim().is_empty()
            || self.content_digest.trim().is_empty()
            || self.media_type.trim().is_empty()
            || self.media_type.contains(['\n', '\r'])
            || self.visibility != SessionOutputVisibility::Committed
            || self.access_scope_ref.len() > 256
            || !is_identifier(&self.access_scope_ref)
            || self
                .disclosure_ref
                .as_ref()
                .is_some_and(|reference| reference.len() > 256 || !is_identifier(reference))
            || (self.occurrence_id.is_some() && self.node_execution_id.is_none())
        {
            return Err("invalid committed Session Output reference");
        }
        if !is_digest(&self.content_digest) {
            return Err("invalid committed Session Output digest");
        }
        Ok(())
    }
}

fn normalize_continuation_payload(payload: &Value) -> Value {
    let Value::Object(object) = payload else {
        return canonical_json_value(payload);
    };
    let mut normalized = object.clone();
    if let Some(Value::Object(write_set)) = normalized.get_mut("write_set")
        && write_set.contains_key("continuation_digest")
    {
        write_set.insert(
            "continuation_digest".to_string(),
            Value::String("<continuation-digest>".to_string()),
        );
    }
    canonical_json_value(&Value::Object(normalized))
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
    /// Driver-owned durable observation records.  The kernel keeps these
    /// backend-neutral; an owner-local adapter validates and stores their
    /// typed execution-observation representation in the same atomic write.
    pub observations: Vec<Value>,
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
            observations: Vec::new(),
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
    /// Runtime-evidence facts published atomically with this commit. Their
    /// content, together with `tuple.observations`, is summarized by
    /// `write_set.runtime_evidence_batch_digest`.
    pub evidence_batch: Vec<Fact>,
}

/// Why a commit request cannot cross the single atomic boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitRequestError {
    EmptyField(&'static str),
    InvalidDigest(&'static str),
    InvalidObservation(String),
    EvidenceBatchMismatch,
    EvidenceObservationDigestMismatch { expected: String, actual: String },
    SessionOutputRefsDigestMismatch { expected: String, actual: String },
    ContinuationDigestMismatch { expected: String, actual: String },
}

impl std::fmt::Display for CommitRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "commit field {field} must be non-empty"),
            Self::InvalidDigest(field) => write!(f, "commit field {field} is not a sha256 digest"),
            Self::InvalidObservation(message) => {
                write!(f, "invalid execution observation: {message}")
            }
            Self::EvidenceBatchMismatch => {
                f.write_str("tuple evidence and evidence_batch must be identical")
            }
            Self::EvidenceObservationDigestMismatch { expected, actual } => write!(
                f,
                "runtime evidence/observation digest mismatch: expected {expected}, got {actual}"
            ),
            Self::SessionOutputRefsDigestMismatch { expected, actual } => write!(
                f,
                "session output refs digest mismatch: expected {expected}, got {actual}"
            ),
            Self::ContinuationDigestMismatch { expected, actual } => write!(
                f,
                "continuation payload digest mismatch: expected {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for CommitRequestError {}

/// One host-fulfilled Capability request or settlement carried by an
/// observation (ADR-0025). A `capability_requested` record is the whole request
/// the host acts on; a `capability_settled` record is how it ended. The two are
/// held apart here so a settlement cannot restate a request it did not make.
fn validate_host_capability_observation(value: &Value, kind: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "host_capability must be an object".to_owned())?;
    const FIELDS: &[&str] = &[
        "capability_request_id",
        "capability_ref",
        "input",
        "authored_permission",
        "outcome",
        "receipt_ref",
    ];
    if let Some(field) = object
        .keys()
        .find(|field| !FIELDS.contains(&field.as_str()))
    {
        return Err(format!("unknown host_capability field {field}"));
    }
    for field in ["capability_request_id", "capability_ref"] {
        let Some(reference) = object.get(field).and_then(Value::as_str) else {
            return Err(format!("host_capability {field} must be a reference"));
        };
        if !is_identifier(reference) {
            return Err(format!("host_capability {field} is not a reference"));
        }
    }
    if object
        .get("capability_ref")
        .and_then(Value::as_str)
        .is_none_or(|reference| !reference.starts_with("host:"))
    {
        return Err("host_capability capability_ref is not host-fulfilled".to_owned());
    }
    if let Some(receipt_ref) = object.get("receipt_ref") {
        if receipt_ref.as_str().is_none_or(|value| !is_identifier(value)) {
            return Err("host_capability receipt_ref is not a reference".to_owned());
        }
    }
    let requested = kind == "capability_requested";
    if requested {
        if object.get("input").and_then(Value::as_str).is_none() {
            return Err("a published request carries its input".to_owned());
        }
        if !matches!(
            object.get("authored_permission").and_then(Value::as_str),
            Some("allow" | "ask" | "deny")
        ) {
            return Err("a published request carries the authored permission".to_owned());
        }
        if object.contains_key("outcome") {
            return Err("a published request has not settled".to_owned());
        }
    } else {
        if !matches!(
            object.get("outcome").and_then(Value::as_str),
            Some("ok" | "denied" | "failed" | "unknown" | "cancelled")
        ) {
            return Err("a settlement states how the request ended".to_owned());
        }
        if object.contains_key("input") || object.contains_key("authored_permission") {
            return Err("a settlement does not restate the request".to_owned());
        }
    }
    Ok(())
}

fn validate_execution_observation(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "observation must be an object".to_owned())?;
    const FIELDS: &[&str] = &[
        "contract",
        "observation_id",
        "program_invocation_id",
        "node_execution_id",
        "occurrence_id",
        "event_ref",
        "attempt_id",
        "region_occurrence_id",
        "sequence",
        "cursor",
        "timing",
        "observation_kind",
        "commitment",
        "content_ref",
        "output_ref",
        "evidence_ref",
        "host_capability",
    ];
    if let Some(field) = object
        .keys()
        .find(|field| !FIELDS.contains(&field.as_str()))
    {
        return Err(format!("unknown field {field}"));
    }
    for field in [
        "contract",
        "observation_id",
        "program_invocation_id",
        "sequence",
        "cursor",
        "timing",
        "observation_kind",
        "commitment",
    ] {
        if !object.contains_key(field) {
            return Err(format!("missing field {field}"));
        }
    }
    if object.get("contract").and_then(Value::as_str) != Some("apxm.execution-observation.v1") {
        return Err("unsupported observation contract".to_owned());
    }
    for field in ["observation_id", "program_invocation_id"] {
        let Some(reference) = object.get(field).and_then(Value::as_str) else {
            return Err(format!("{field} must be an opaque reference"));
        };
        if !is_identifier(reference) {
            return Err(format!("{field} is not an opaque reference"));
        }
    }
    let sequence = object
        .get("sequence")
        .and_then(Value::as_u64)
        .filter(|sequence| *sequence > 0)
        .ok_or_else(|| "sequence must be a positive integer".to_owned())?;
    let cursor = object
        .get("cursor")
        .and_then(Value::as_object)
        .ok_or_else(|| "cursor must be an object".to_owned())?;
    if cursor.get("position").and_then(Value::as_u64) != Some(sequence)
        || cursor
            .get("token")
            .and_then(Value::as_str)
            .is_none_or(|token| !is_identifier(token))
        || cursor
            .keys()
            .any(|field| !matches!(field.as_str(), "position" | "token"))
    {
        return Err("cursor must match the observation sequence".to_owned());
    }
    let timing = object
        .get("timing")
        .and_then(Value::as_object)
        .ok_or_else(|| "timing must be an object".to_owned())?;
    if timing
        .get("observed_at_unix_ms")
        .and_then(Value::as_u64)
        .is_none()
        || timing
            .keys()
            .any(|field| !matches!(field.as_str(), "observed_at_unix_ms" | "duration_ms"))
    {
        return Err("timing must contain observed_at_unix_ms".to_owned());
    }
    let kind = object
        .get("observation_kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "observation_kind must be a string".to_owned())?;
    const KINDS: &[&str] = &[
        "invocation_started",
        "node_started",
        "operand_published",
        "branch_transition",
        "join_transition",
        "loop_transition",
        "model_attempt",
        "capability_attempt",
        "capability_requested",
        "capability_settled",
        "program_attempt",
        "approval_requested",
        "approval_resolved",
        "event_waiting",
        "event_resumed",
        "content_published",
        "content_committed",
        "terminal_committed",
        "invocation_failed",
        "evidence_committed",
        "invocation_cancelled",
        "outcome_unknown",
    ];
    if !KINDS.contains(&kind) {
        return Err("unknown observation kind".to_owned());
    }
    let commitment = object
        .get("commitment")
        .and_then(Value::as_str)
        .ok_or_else(|| "commitment must be a string".to_owned())?;
    if !matches!(commitment, "provisional" | "committed") {
        return Err("unknown observation commitment".to_owned());
    }
    let event_kind = matches!(kind, "event_waiting" | "event_resumed");
    if event_kind != object.contains_key("event_ref") {
        return Err("event_ref does not match observation kind".to_owned());
    }
    let host_capability_kind = matches!(kind, "capability_requested" | "capability_settled");
    if host_capability_kind != object.contains_key("host_capability") {
        return Err("host_capability does not match observation kind".to_owned());
    }
    if let Some(host_capability) = object.get("host_capability") {
        validate_host_capability_observation(host_capability, kind)?;
    }
    for field in [
        "node_execution_id",
        "occurrence_id",
        "attempt_id",
        "region_occurrence_id",
        "content_ref",
        "output_ref",
        "evidence_ref",
    ] {
        if let Some(reference) = object.get(field) {
            let reference = reference
                .as_str()
                .ok_or_else(|| format!("{field} must be an opaque reference"))?;
            if !is_identifier(reference) {
                return Err(format!("{field} is not an opaque reference"));
            }
        }
    }
    if let Some(event_ref) = object.get("event_ref") {
        let event_ref = event_ref
            .as_object()
            .ok_or_else(|| "event_ref must be an object".to_owned())?;
        if event_ref
            .keys()
            .any(|field| !matches!(field.as_str(), "event_ref" | "generation" | "occurrence_id"))
        {
            return Err("event_ref contains an unknown field".to_owned());
        }
        if event_ref
            .get("event_ref")
            .and_then(Value::as_str)
            .is_none_or(|reference| !is_identifier(reference))
        {
            return Err("event_ref must contain an opaque event reference".to_owned());
        }
        if event_ref
            .get("generation")
            .is_some_and(|generation| !generation.is_u64())
            || event_ref.get("occurrence_id").is_some_and(|occurrence| {
                occurrence
                    .as_str()
                    .is_none_or(|reference| !is_identifier(reference))
            })
        {
            return Err("event_ref contains an invalid generation or occurrence".to_owned());
        }
    }
    if kind == "content_published"
        && (commitment != "provisional" || !object.contains_key("content_ref"))
    {
        return Err("content_published requires provisional content_ref".to_owned());
    }
    if kind == "content_committed"
        && (commitment != "committed"
            || (!object.contains_key("content_ref") && !object.contains_key("output_ref")))
    {
        return Err("content_committed requires committed content/output ref".to_owned());
    }
    if kind == "terminal_committed"
        && (commitment != "committed"
            || (!object.contains_key("output_ref") && !object.contains_key("evidence_ref")))
    {
        return Err("terminal_committed requires committed output/evidence ref".to_owned());
    }
    if kind == "evidence_committed"
        && (commitment != "committed" || !object.contains_key("evidence_ref"))
    {
        return Err("evidence_committed requires committed evidence_ref".to_owned());
    }
    if kind == "invocation_failed"
        && (commitment != "committed" || !object.contains_key("evidence_ref"))
    {
        return Err("invocation_failed requires committed evidence_ref".to_owned());
    }
    Ok(())
}

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
        let expected =
            runtime_evidence_and_observation_digest(&self.tuple.evidence, &self.tuple.observations);
        if self.write_set.runtime_evidence_batch_digest != expected {
            return Err(CommitRequestError::EvidenceObservationDigestMismatch {
                expected,
                actual: self.write_set.runtime_evidence_batch_digest.clone(),
            });
        }
        let expected = session_output_refs_digest(&self.tuple.output_refs);
        if self.write_set.session_output_refs_digest != expected {
            return Err(CommitRequestError::SessionOutputRefsDigestMismatch {
                expected,
                actual: self.write_set.session_output_refs_digest.clone(),
            });
        }
        for observation in &self.tuple.observations {
            validate_execution_observation(observation)
                .map_err(CommitRequestError::InvalidObservation)?;
        }
        if let Some(payload) = self.tuple.continuation.as_ref() {
            let expected = continuation_digest(Some(payload));
            if self.write_set.continuation_digest != expected {
                return Err(CommitRequestError::ContinuationDigestMismatch {
                    expected,
                    actual: self.write_set.continuation_digest.clone(),
                });
            }
        }
        Ok(())
    }
}

/// The typed result of an atomic commit. Mirrors the closed
/// `apxm.execution-commit` `commit_result` set.
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
    /// Project this request and its result as an exact `apxm.execution-commit`
    /// object, including the constant atomic write set. No per-member split field
    /// is ever emitted.
    #[must_use]
    pub fn to_contract_json(&self, result: &ExecutionCommitResult) -> Value {
        let mut object = json!({
            "schema_version": "apxm.execution-commit",
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
    /// Stage Session Output bytes before compare-and-commit. The returned
    /// reference is provisional storage and becomes readable only when the
    /// exact reference is included in a winning commit tuple.
    async fn prepare_output(
        &self,
        _preparation: SessionOutputPreparation,
    ) -> Result<PreparedSessionOutputRef, String> {
        Err("Session Output staging is not supported by this commit port".into())
    }

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

    /// Read the current continuation and the digest bound to that same commit
    /// record. Implementations with a richer durable record should override
    /// this method atomically; the compatibility default protects older ports
    /// by hashing the value they already expose.
    async fn load_continuation_with_integrity(
        &self,
        program_instance_ref: &ProgramInstanceRef,
    ) -> Option<CommittedContinuation> {
        self.load_continuation(program_instance_ref)
            .await
            .map(|payload| CommittedContinuation {
                digest: continuation_digest(Some(&payload)),
                payload,
            })
    }
}
