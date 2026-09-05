//! Product-neutral execution observation, inspection, output, and read types.
//!
//! These types are the closed Rust mirror for the execution contract schemas.
//! They carry opaque runtime references only; authorization policy remains with
//! the caller's composition boundary.

use apxm_core::grammar::{is_digest, is_identifier};
pub use apxm_core::types::host_capability::HostCapabilityOutcomeKind;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use thiserror::Error;

pub const EXECUTION_OBSERVATION_CONTRACT: &str = "apxm.execution-observation.v1";
pub const NODE_EXECUTION_INSPECTION_CONTRACT: &str = "apxm.node-execution-inspection.v1";
pub const SESSION_OUTPUT_REF_CONTRACT: &str = "apxm.session-output-ref.v1";
pub const EXECUTION_READ_CONTRACT: &str = "apxm.execution-read.v1";

const MAX_REF_BYTES: usize = 256;
const MAX_MEDIA_TYPE_BYTES: usize = 255;
const MAX_PAGE_LIMIT: u32 = 1000;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ContractValidationError {
    #[error("{kind} must be a non-empty APXM identifier")]
    InvalidRef { kind: &'static str },
    #[error("{kind} exceeds the {MAX_REF_BYTES}-byte bound")]
    RefTooLong { kind: &'static str },
    #[error("invalid contract: expected {expected}")]
    InvalidContract { expected: &'static str },
    #[error("sequence and cursor position must match and be non-zero")]
    InvalidSequence,
    #[error("cursor token must be a non-empty APXM identifier")]
    InvalidCursorToken,
    #[error("cursor page entries must be strictly increasing")]
    NonMonotonicPage,
    #[error("cursor page metadata is inconsistent")]
    InvalidPageMetadata,
    #[error("page limit must be between 1 and {MAX_PAGE_LIMIT}")]
    InvalidPageLimit,
    #[error("digest is not a lowercase sha256 digest")]
    InvalidDigest,
    #[error("media type is invalid")]
    InvalidMediaType,
    #[error("observation commitment does not match its kind")]
    InvalidObservationCommitment,
    #[error("observation event reference does not match its kind")]
    InvalidObservationReference,
    #[error("read context purpose does not match the read operation")]
    InvalidReadPurpose,
    #[error("read request has an invalid contract vocabulary")]
    InvalidReadRequest,
    #[error("duplicate {kind} reference in inspection")]
    DuplicateReference { kind: &'static str },
}

macro_rules! opaque_ref {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ContractValidationError> {
                let value = value.into();
                if value.is_empty() || !is_identifier(&value) {
                    return Err(ContractValidationError::InvalidRef { kind: $label });
                }
                if value.len() > MAX_REF_BYTES {
                    return Err(ContractValidationError::RefTooLong { kind: $label });
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ContractValidationError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = ContractValidationError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

opaque_ref!(ProgramRef, "program_ref");
opaque_ref!(ProgramInstanceId, "program_instance_id");
opaque_ref!(ProgramInvocationId, "program_invocation_id");
opaque_ref!(NodeExecutionId, "node_execution_id");
opaque_ref!(AttemptId, "attempt_id");
opaque_ref!(OccurrenceId, "occurrence_id");
opaque_ref!(RegionOccurrenceId, "region_occurrence_id");
opaque_ref!(ContentRef, "content_ref");
opaque_ref!(OutputRef, "output_ref");
opaque_ref!(EvidenceRef, "evidence_ref");
opaque_ref!(ObservationId, "observation_id");
opaque_ref!(RequestId, "request_id");
opaque_ref!(ScopeRef, "scope_ref");
opaque_ref!(PrincipalRef, "principal_ref");
opaque_ref!(GrantRef, "grant_ref");
opaque_ref!(CorrelationId, "correlation_id");

/// A replay-stable, scope-bound cursor position. The token is opaque to the
/// consumer; position is exposed only to prove page ordering and gaps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCursor {
    pub position: u64,
    pub token: String,
}

impl<'de> Deserialize<'de> for ExecutionCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawCursor {
            position: u64,
            token: String,
        }

        let raw = RawCursor::deserialize(deserializer)?;
        Self::new(raw.position, raw.token).map_err(serde::de::Error::custom)
    }
}

impl ExecutionCursor {
    pub fn new(position: u64, token: impl Into<String>) -> Result<Self, ContractValidationError> {
        let token = token.into();
        if token.is_empty() || !is_identifier(&token) || token.len() > MAX_REF_BYTES {
            return Err(ContractValidationError::InvalidCursorToken);
        }
        Ok(Self { position, token })
    }

    pub fn validate(&self) -> Result<(), ContractValidationError> {
        Self::new(self.position, self.token.clone()).map(|_| ())
    }
}

/// A bounded page carrying enough metadata for reconnect and retention-gap
/// handling. It does not grant access; the read context is checked separately.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPage<T> {
    pub items: Vec<T>,
    pub item_cursors: Vec<ExecutionCursor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<ExecutionCursor>,
    pub high_watermark: ExecutionCursor,
    pub retention_floor: ExecutionCursor,
    pub has_more: bool,
}

impl<T> ExecutionPage<T> {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        self.high_watermark.validate()?;
        self.retention_floor.validate()?;
        if self.retention_floor.position > self.high_watermark.position
            || self.item_cursors.len() != self.items.len()
            || self
                .item_cursors
                .iter()
                .any(|cursor| cursor.position < self.retention_floor.position)
            || self
                .item_cursors
                .iter()
                .any(|cursor| cursor.position > self.high_watermark.position)
        {
            return Err(ContractValidationError::InvalidPageMetadata);
        }
        for pair in self.item_cursors.windows(2) {
            if pair[0].position >= pair[1].position {
                return Err(ContractValidationError::NonMonotonicPage);
            }
        }
        if self.has_more != self.next_cursor.is_some() {
            return Err(ContractValidationError::InvalidPageMetadata);
        }
        if let Some(next) = &self.next_cursor {
            next.validate()?;
            if next.position > self.high_watermark.position {
                return Err(ContractValidationError::InvalidPageMetadata);
            }
            if let Some(last) = self.item_cursors.last() {
                // The store returns the last delivered row as the resume
                // cursor. The next request treats `after_cursor` exclusively,
                // so requiring equality prevents a page from skipping rows.
                if next != last {
                    return Err(ContractValidationError::InvalidPageMetadata);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    InvocationStarted,
    NodeStarted,
    OperandPublished,
    BranchTransition,
    JoinTransition,
    LoopTransition,
    ModelAttempt,
    CapabilityAttempt,
    CapabilityRequested,
    CapabilitySettled,
    ProgramAttempt,
    ApprovalRequested,
    ApprovalResolved,
    EventWaiting,
    EventResumed,
    ContentPublished,
    ContentCommitted,
    TerminalCommitted,
    InvocationFailed,
    EvidenceCommitted,
    InvocationCancelled,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Commitment {
    Provisional,
    Committed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationTiming {
    pub observed_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// The authored permission a program's own source requested for one Capability.
///
/// For a host-fulfilled reference APXM records this and does not broker it: it
/// is what the program asked for, carried verbatim to the host, which merges it
/// with its own policy and may only narrow it (ADR-0025).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredPermission {
    Allow,
    Ask,
    Deny,
}

/// The host-fulfilled Capability request or settlement one observation carries.
///
/// A `capability_requested` observation is the whole request: a host that
/// cannot read the arguments cannot perform the call, so `input` travels inline
/// as the exact canonical argument bytes rather than as a reference. A
/// `capability_settled` observation carries how it ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilityObservation {
    pub capability_request_id: String,
    pub capability_ref: String,
    /// The exact canonical JSON argument bytes. Present on a request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// What the program's own source asked for. Present on a request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_permission: Option<AuthoredPermission>,
    /// How the request ended. Present on a settlement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<HostCapabilityOutcomeKind>,
    /// The host's own durable record of the effect. Present on a settlement
    /// when the host stated one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_ref: Option<String>,
}

impl HostCapabilityObservation {
    pub fn validate(&self, kind: ObservationKind) -> Result<(), ContractValidationError> {
        if !is_identifier(&self.capability_request_id) {
            return Err(ContractValidationError::InvalidRef {
                kind: "capability_request_id",
            });
        }
        if self.capability_ref.len() > MAX_REF_BYTES {
            return Err(ContractValidationError::RefTooLong {
                kind: "capability_ref",
            });
        }
        if !self.capability_ref.starts_with("host:") {
            return Err(ContractValidationError::InvalidRef {
                kind: "capability_ref",
            });
        }
        if let Some(receipt_ref) = &self.receipt_ref
            && !is_identifier(receipt_ref)
        {
            return Err(ContractValidationError::InvalidRef {
                kind: "receipt_ref",
            });
        }
        let requested = kind == ObservationKind::CapabilityRequested;
        if requested
            != (self.input.is_some()
                && self.authored_permission.is_some()
                && self.outcome.is_none())
        {
            return Err(ContractValidationError::InvalidObservationReference);
        }
        if !requested && self.outcome.is_none() {
            return Err(ContractValidationError::InvalidObservationReference);
        }
        Ok(())
    }
}

/// The actual runtime Event identity associated with an event wait/resume
/// observation. Generation and source occurrence are optional because the
/// execution port may expose only the exact EventRef at this boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventObservationRef {
    pub event_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<OccurrenceId>,
}

impl EventObservationRef {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.event_ref.len() > MAX_REF_BYTES {
            return Err(ContractValidationError::RefTooLong { kind: "event_ref" });
        }
        if !is_identifier(&self.event_ref) {
            return Err(ContractValidationError::InvalidRef { kind: "event_ref" });
        }
        if let Some(occurrence_id) = &self.occurrence_id {
            OccurrenceId::new(occurrence_id.as_str().to_owned())?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionObservation {
    pub contract: String,
    pub observation_id: ObservationId,
    pub program_invocation_id: ProgramInvocationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_execution_id: Option<NodeExecutionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<OccurrenceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<EventObservationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<AttemptId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_occurrence_id: Option<RegionOccurrenceId>,
    pub sequence: u64,
    pub cursor: ExecutionCursor,
    pub timing: ObservationTiming,
    pub observation_kind: ObservationKind,
    pub commitment: Commitment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_ref: Option<OutputRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_ref: Option<EvidenceRef>,
    /// The host-fulfilled Capability request or settlement this observation is
    /// about. Present exactly on `capability_requested` and
    /// `capability_settled`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_capability: Option<HostCapabilityObservation>,
}

impl ExecutionObservation {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.contract != EXECUTION_OBSERVATION_CONTRACT {
            return Err(ContractValidationError::InvalidContract {
                expected: EXECUTION_OBSERVATION_CONTRACT,
            });
        }
        self.cursor.validate()?;
        if self.sequence == 0 || self.cursor.position != self.sequence {
            return Err(ContractValidationError::InvalidSequence);
        }
        if let Some(event_ref) = &self.event_ref {
            event_ref.validate()?;
        }
        if matches!(
            self.observation_kind,
            ObservationKind::EventWaiting | ObservationKind::EventResumed
        ) != self.event_ref.is_some()
        {
            return Err(ContractValidationError::InvalidObservationReference);
        }
        let host_capability_kind = matches!(
            self.observation_kind,
            ObservationKind::CapabilityRequested | ObservationKind::CapabilitySettled
        );
        match (&self.host_capability, host_capability_kind) {
            (Some(host_capability), true) => host_capability.validate(self.observation_kind)?,
            (None, false) => {}
            _ => return Err(ContractValidationError::InvalidObservationReference),
        }
        if matches!(
            self.observation_kind,
            ObservationKind::TerminalCommitted | ObservationKind::EvidenceCommitted
        ) && self.commitment != Commitment::Committed
        {
            return Err(ContractValidationError::InvalidObservationCommitment);
        }
        if self.observation_kind == ObservationKind::ContentPublished
            && (self.commitment != Commitment::Provisional || self.content_ref.is_none())
        {
            return Err(ContractValidationError::InvalidObservationCommitment);
        }
        if self.observation_kind == ObservationKind::ContentCommitted
            && (self.commitment != Commitment::Committed
                || (self.content_ref.is_none() && self.output_ref.is_none()))
        {
            return Err(ContractValidationError::InvalidObservationCommitment);
        }
        if self.observation_kind == ObservationKind::TerminalCommitted
            && (self.commitment != Commitment::Committed
                || (self.output_ref.is_none() && self.evidence_ref.is_none()))
        {
            return Err(ContractValidationError::InvalidObservationCommitment);
        }
        if self.observation_kind == ObservationKind::EvidenceCommitted
            && (self.commitment != Commitment::Committed || self.evidence_ref.is_none())
        {
            return Err(ContractValidationError::InvalidObservationCommitment);
        }
        if self.observation_kind == ObservationKind::InvocationFailed
            && (self.commitment != Commitment::Committed || self.evidence_ref.is_none())
        {
            return Err(ContractValidationError::InvalidObservationCommitment);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeExecutionStatus {
    Running,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramInvocationStatus {
    AdmissionPending,
    Running,
    WaitingEvent,
    CommittedYield,
    CommittedReturn,
    Cancelling,
    Cancelled,
    CancellationUnconfirmed,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeExecutionInspection {
    pub contract: String,
    pub program_invocation_id: ProgramInvocationId,
    pub node_execution_id: NodeExecutionId,
    pub air_node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_node_execution_id: Option<NodeExecutionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<OccurrenceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_occurrence_id: Option<RegionOccurrenceId>,
    pub status: NodeExecutionStatus,
    pub commitment: Commitment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_or_request_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisional_response_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_response_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_ref: Option<OutputRef>,
    pub attempt_refs: Vec<AttemptId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_ref: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_ref: Option<ContentRef>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub child_program_refs: Vec<ProgramRef>,
}

impl<'de> Deserialize<'de> for NodeExecutionInspection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawNodeExecutionInspection {
            contract: String,
            program_invocation_id: ProgramInvocationId,
            node_execution_id: NodeExecutionId,
            air_node_id: String,
            parent_node_execution_id: Option<NodeExecutionId>,
            occurrence_id: Option<OccurrenceId>,
            region_occurrence_id: Option<RegionOccurrenceId>,
            status: NodeExecutionStatus,
            commitment: Commitment,
            input_ref: Option<ContentRef>,
            prompt_or_request_ref: Option<ContentRef>,
            provisional_response_ref: Option<ContentRef>,
            final_response_ref: Option<ContentRef>,
            output_ref: Option<OutputRef>,
            attempt_refs: Vec<AttemptId>,
            metrics_ref: Option<ContentRef>,
            usage_ref: Option<ContentRef>,
            policy_ref: Option<ContentRef>,
            approval_ref: Option<ContentRef>,
            error_ref: Option<ContentRef>,
            trace_ref: Option<ContentRef>,
            evidence_refs: Vec<EvidenceRef>,
            child_program_refs: Vec<ProgramRef>,
        }

        let raw = RawNodeExecutionInspection::deserialize(deserializer)?;
        let inspection = Self {
            contract: raw.contract,
            program_invocation_id: raw.program_invocation_id,
            node_execution_id: raw.node_execution_id,
            air_node_id: raw.air_node_id,
            parent_node_execution_id: raw.parent_node_execution_id,
            occurrence_id: raw.occurrence_id,
            region_occurrence_id: raw.region_occurrence_id,
            status: raw.status,
            commitment: raw.commitment,
            input_ref: raw.input_ref,
            prompt_or_request_ref: raw.prompt_or_request_ref,
            provisional_response_ref: raw.provisional_response_ref,
            final_response_ref: raw.final_response_ref,
            output_ref: raw.output_ref,
            attempt_refs: raw.attempt_refs,
            metrics_ref: raw.metrics_ref,
            usage_ref: raw.usage_ref,
            policy_ref: raw.policy_ref,
            approval_ref: raw.approval_ref,
            error_ref: raw.error_ref,
            trace_ref: raw.trace_ref,
            evidence_refs: raw.evidence_refs,
            child_program_refs: raw.child_program_refs,
        };
        inspection.validate().map_err(serde::de::Error::custom)?;
        Ok(inspection)
    }
}

impl NodeExecutionInspection {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.contract != NODE_EXECUTION_INSPECTION_CONTRACT {
            return Err(ContractValidationError::InvalidContract {
                expected: NODE_EXECUTION_INSPECTION_CONTRACT,
            });
        }
        if !is_identifier(&self.air_node_id) {
            return Err(ContractValidationError::InvalidRef {
                kind: "air_node_id",
            });
        }
        let mut seen_attempts = HashSet::new();
        for reference in &self.attempt_refs {
            if !seen_attempts.insert(reference) {
                return Err(ContractValidationError::DuplicateReference { kind: "attempt" });
            }
        }
        let mut seen_evidence = HashSet::new();
        for reference in &self.evidence_refs {
            if !seen_evidence.insert(reference) {
                return Err(ContractValidationError::DuplicateReference { kind: "evidence" });
            }
        }
        let mut seen_children = HashSet::new();
        for reference in &self.child_program_refs {
            if !seen_children.insert(reference) {
                return Err(ContractValidationError::DuplicateReference {
                    kind: "child_program",
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputVisibility {
    Provisional,
    Committed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionOutputRef {
    pub contract: String,
    pub ref_type: String,
    #[serde(rename = "ref")]
    pub reference: OutputRef,
    pub program_instance_id: ProgramInstanceId,
    pub program_invocation_id: ProgramInvocationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_execution_id: Option<NodeExecutionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<OccurrenceId>,
    pub content_digest: String,
    pub byte_length: u64,
    pub media_type: String,
    #[serde(rename = "commitment")]
    pub visibility: OutputVisibility,
    pub access_scope_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclosure_ref: Option<String>,
}

impl SessionOutputRef {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.contract != SESSION_OUTPUT_REF_CONTRACT || self.ref_type != "SessionOutputRef" {
            return Err(ContractValidationError::InvalidContract {
                expected: SESSION_OUTPUT_REF_CONTRACT,
            });
        }
        if !is_digest(&self.content_digest) {
            return Err(ContractValidationError::InvalidDigest);
        }
        if self.access_scope_ref.len() > MAX_REF_BYTES || !is_identifier(&self.access_scope_ref) {
            return Err(ContractValidationError::InvalidRef {
                kind: "access_scope_ref",
            });
        }
        if self
            .disclosure_ref
            .as_ref()
            .is_some_and(|value| value.len() > MAX_REF_BYTES || !is_identifier(value))
        {
            return Err(ContractValidationError::InvalidRef {
                kind: "disclosure_ref",
            });
        }
        if self.media_type.is_empty()
            || self.media_type.len() > MAX_MEDIA_TYPE_BYTES
            || self.media_type.contains(['\n', '\r'])
        {
            return Err(ContractValidationError::InvalidMediaType);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadPurpose {
    Observation,
    Inspection,
    Content,
    Output,
    Evidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadContext {
    pub request_id: RequestId,
    pub scope_ref: ScopeRef,
    pub principal_ref: PrincipalRef,
    pub grant_ref: GrantRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
    pub purpose: ReadPurpose,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionReadRequest {
    #[serde(rename = "observation.subscribe")]
    ObservationSubscribe {
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
        #[serde(skip_serializing_if = "Option::is_none")]
        after_cursor: Option<ExecutionCursor>,
        limit: u32,
    },
    #[serde(rename = "program_invocation.inspect")]
    ProgramInvocationInspect {
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
        #[serde(skip_serializing_if = "Option::is_none")]
        node_execution_id: Option<NodeExecutionId>,
    },
    #[serde(rename = "content.read")]
    ContentRead {
        context: ReadContext,
        content_ref: ContentRef,
    },
    #[serde(rename = "output.read")]
    OutputRead {
        context: ReadContext,
        output_ref: OutputRef,
    },
    #[serde(rename = "evidence.read")]
    EvidenceRead {
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
        #[serde(skip_serializing_if = "Option::is_none")]
        after_cursor: Option<ExecutionCursor>,
        limit: u32,
    },
}

impl ExecutionReadRequest {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        let (context, after_cursor, limit, purpose) = match self {
            Self::ObservationSubscribe {
                context,
                after_cursor,
                limit,
                ..
            } => (context, after_cursor, *limit, ReadPurpose::Observation),
            Self::ProgramInvocationInspect { context, .. } => {
                if context.purpose != ReadPurpose::Inspection {
                    return Err(ContractValidationError::InvalidReadPurpose);
                }
                return Ok(());
            }
            Self::ContentRead { context, .. } => {
                if context.purpose != ReadPurpose::Content {
                    return Err(ContractValidationError::InvalidReadPurpose);
                }
                return Ok(());
            }
            Self::OutputRead { context, .. } => {
                if context.purpose != ReadPurpose::Output {
                    return Err(ContractValidationError::InvalidReadPurpose);
                }
                return Ok(());
            }
            Self::EvidenceRead {
                context,
                after_cursor,
                limit,
                ..
            } => (context, after_cursor, *limit, ReadPurpose::Evidence),
        };
        if context.purpose != purpose {
            return Err(ContractValidationError::InvalidReadPurpose);
        }
        if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
            return Err(ContractValidationError::InvalidPageLimit);
        }
        if let Some(cursor) = after_cursor {
            cursor.validate()?;
        }
        Ok(())
    }

    /// Verify that a node-scoped inspection response belongs to the
    /// invocation named by the request. Storage owners use this after lookup;
    /// the wire schema alone cannot express that cross-record invariant.
    pub fn validate_node_inspection(
        &self,
        inspection: &NodeExecutionInspection,
    ) -> Result<(), ContractValidationError> {
        let Self::ProgramInvocationInspect {
            context,
            program_invocation_id,
            node_execution_id: Some(node_execution_id),
        } = self
        else {
            return Err(ContractValidationError::InvalidReadRequest);
        };
        if context.purpose != ReadPurpose::Inspection
            || inspection.program_invocation_id != *program_invocation_id
            || inspection.node_execution_id != *node_execution_id
        {
            return Err(ContractValidationError::InvalidReadRequest);
        }
        inspection.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramInvocationInspection {
    pub program_invocation_id: ProgramInvocationId,
    pub status: ProgramInvocationStatus,
    pub node_execution_refs: Vec<NodeExecutionId>,
    pub output_refs: Vec<OutputRef>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub cursor: ExecutionCursor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentReadResult {
    pub content_ref: ContentRef,
    pub program_instance_id: ProgramInstanceId,
    pub program_invocation_id: ProgramInvocationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_execution_id: Option<NodeExecutionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<OccurrenceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_ref: Option<SessionOutputRef>,
    pub content_digest: String,
    pub byte_length: u64,
    pub media_type: String,
    #[serde(rename = "commitment")]
    pub visibility: OutputVisibility,
    pub access_scope_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclosure_ref: Option<String>,
    pub bytes: Vec<u8>,
}

impl ContentReadResult {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        let mut hasher = Sha256::new();
        hasher.update(&self.bytes);
        let expected_digest = format!("sha256:{:x}", hasher.finalize());
        if !is_digest(&self.content_digest)
            || self.content_digest != expected_digest
            || self.byte_length != self.bytes.len() as u64
            || self.access_scope_ref.len() > MAX_REF_BYTES
            || !is_identifier(&self.access_scope_ref)
            || self.visibility != OutputVisibility::Committed
        {
            return Err(ContractValidationError::InvalidReadRequest);
        }
        if self
            .disclosure_ref
            .as_ref()
            .is_some_and(|value| value.len() > MAX_REF_BYTES || !is_identifier(value))
        {
            return Err(ContractValidationError::InvalidRef {
                kind: "disclosure_ref",
            });
        }
        if self.occurrence_id.is_some() && self.node_execution_id.is_none() {
            return Err(ContractValidationError::InvalidReadRequest);
        }
        if let Some(output_ref) = &self.output_ref {
            output_ref.validate()?;
            if output_ref.visibility != OutputVisibility::Committed
                || output_ref.reference.as_str() != self.content_ref.as_str()
                || output_ref.program_instance_id != self.program_instance_id
                || output_ref.program_invocation_id != self.program_invocation_id
                || output_ref.node_execution_id != self.node_execution_id
                || output_ref.occurrence_id != self.occurrence_id
                || output_ref.content_digest != self.content_digest
                || output_ref.byte_length != self.byte_length
                || output_ref.access_scope_ref != self.access_scope_ref
            {
                return Err(ContractValidationError::InvalidReadRequest);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub evidence_ref: EvidenceRef,
    pub program_invocation_id: ProgramInvocationId,
    pub sequence: u64,
    pub fact_kind: EvidenceFactKind,
    pub evidence_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_execution_id: Option<NodeExecutionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<OccurrenceId>,
    /// The bounded typed error carried by a terminal runtime fact. This keeps
    /// failure classification inspectable without exposing the raw fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed_error: Option<EvidenceTypedError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceErrorCategory {
    Validation,
    Admission,
    Authority,
    Configuration,
    Unavailable,
    Conflict,
    OutcomeUnknown,
    Internal,
}

/// Public projection of the runtime's existing typed-error envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceTypedError {
    pub error_id: String,
    pub category: EvidenceErrorCategory,
    pub code_ref: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details_digest: Option<String>,
}

impl EvidenceTypedError {
    fn validate(&self) -> Result<(), ContractValidationError> {
        if self.error_id.len() > MAX_REF_BYTES || !is_identifier(&self.error_id) {
            return Err(ContractValidationError::InvalidRef { kind: "error_id" });
        }
        if self.code_ref.len() > MAX_REF_BYTES || !is_identifier(&self.code_ref) {
            return Err(ContractValidationError::InvalidRef { kind: "code_ref" });
        }
        if self.message.is_empty() || self.message.len() > 4096 {
            return Err(ContractValidationError::InvalidReadRequest);
        }
        if self
            .details_digest
            .as_ref()
            .is_some_and(|value| !is_digest(value))
        {
            return Err(ContractValidationError::InvalidDigest);
        }
        Ok(())
    }
}

impl EvidenceRecord {
    /// Recompute the digest over the exact typed evidence envelope persisted
    /// by the local adapter. The raw runtime fact is deliberately not exposed
    /// by the read contract, so this envelope is the stable fact projection
    /// callers can verify after reopen.
    #[must_use]
    pub fn computed_digest(&self) -> String {
        #[derive(Serialize)]
        struct EvidenceEnvelope<'a> {
            evidence_ref: &'a EvidenceRef,
            program_invocation_id: &'a ProgramInvocationId,
            sequence: u64,
            fact_kind: EvidenceFactKind,
            #[serde(skip_serializing_if = "Option::is_none")]
            node_execution_id: &'a Option<NodeExecutionId>,
            #[serde(skip_serializing_if = "Option::is_none")]
            occurrence_id: &'a Option<OccurrenceId>,
            #[serde(skip_serializing_if = "Option::is_none")]
            typed_error: &'a Option<EvidenceTypedError>,
        }
        let envelope = EvidenceEnvelope {
            evidence_ref: &self.evidence_ref,
            program_invocation_id: &self.program_invocation_id,
            sequence: self.sequence,
            fact_kind: self.fact_kind,
            node_execution_id: &self.node_execution_id,
            occurrence_id: &self.occurrence_id,
            typed_error: &self.typed_error,
        };
        let bytes = serde_json::to_vec(&envelope).expect("typed evidence envelope serializes");
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("sha256:{:x}", hasher.finalize())
    }

    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.sequence == 0 || !is_digest(&self.evidence_digest) {
            return Err(ContractValidationError::InvalidDigest);
        }
        if self.occurrence_id.is_some() && self.node_execution_id.is_none() {
            return Err(ContractValidationError::InvalidReadRequest);
        }
        if let Some(error) = &self.typed_error {
            if !matches!(
                self.fact_kind,
                EvidenceFactKind::InvocationFailed | EvidenceFactKind::EffectOutcomeUnknown
            ) {
                return Err(ContractValidationError::InvalidReadRequest);
            }
            error.validate()?;
        }
        if self.evidence_digest != self.computed_digest() {
            return Err(ContractValidationError::InvalidDigest);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceFactKind {
    #[serde(rename = "instance.state_changed")]
    InstanceStateChanged,
    #[serde(rename = "invocation.state_changed")]
    InvocationStateChanged,
    #[serde(rename = "instance.created")]
    InstanceCreated,
    #[serde(rename = "invocation.admitted")]
    InvocationAdmitted,
    #[serde(rename = "child.attached")]
    ChildAttached,
    #[serde(rename = "attempt.recorded")]
    AttemptRecorded,
    #[serde(rename = "invocation.attempt_recorded")]
    InvocationAttemptRecorded,
    #[serde(rename = "capability.attempt_recorded")]
    CapabilityAttemptRecorded,
    #[serde(rename = "invocation.committed")]
    InvocationCommitted,
    #[serde(rename = "invocation.failed")]
    InvocationFailed,
    #[serde(rename = "invocation.cancelled")]
    InvocationCancelled,
    #[serde(rename = "event.created")]
    EventCreated,
    #[serde(rename = "event.await_registered")]
    EventAwaitRegistered,
    #[serde(rename = "invocation.parked")]
    InvocationParked,
    #[serde(rename = "event.terminal")]
    EventTerminal,
    #[serde(rename = "invocation.resumed")]
    InvocationResumed,
    #[serde(rename = "instance.closed")]
    InstanceClosed,
    #[serde(rename = "instance.cancelled")]
    InstanceCancelled,
    #[serde(rename = "effect.outcome_unknown")]
    EffectOutcomeUnknown,
    #[serde(rename = "delivery.recorded")]
    DeliveryRecorded,
    #[serde(rename = "region.occurrence_started")]
    RegionOccurrenceStarted,
    #[serde(rename = "node_execution.recorded")]
    NodeExecutionRecorded,
    #[serde(rename = "hook.executed")]
    HookExecuted,
    #[serde(rename = "context.transitioned")]
    ContextTransitioned,
    #[serde(rename = "LoopIterationCompleted")]
    LoopIterationCompleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionReadResult {
    ObservationPage {
        page: ExecutionPage<ExecutionObservation>,
    },
    ProgramInvocationInspection {
        inspection: ProgramInvocationInspection,
    },
    NodeExecutionInspection {
        inspection: NodeExecutionInspection,
    },
    Content {
        content: ContentReadResult,
    },
    Output {
        output: ContentReadResult,
    },
    EvidencePage {
        page: ExecutionPage<EvidenceRecord>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::host_capability::host_capability_request_id;
    use serde_json::json;

    fn cursor(position: u64) -> ExecutionCursor {
        ExecutionCursor::new(position, "scope.cursor").expect("cursor")
    }

    fn host_capability_observation(
        kind: ObservationKind,
        sequence: u64,
    ) -> HostCapabilityObservation {
        let requested = kind == ObservationKind::CapabilityRequested;
        HostCapabilityObservation {
            capability_request_id: host_capability_request_id(
                "invocation.1",
                &format!("node-execution.{sequence}"),
            ),
            capability_ref: "host:notes.search".to_owned(),
            input: requested.then(|| "{}".to_owned()),
            authored_permission: requested.then_some(AuthoredPermission::Allow),
            outcome: (!requested).then_some(HostCapabilityOutcomeKind::Ok),
            receipt_ref: None,
        }
    }

    fn observation(
        commitment: Commitment,
        kind: ObservationKind,
        sequence: u64,
    ) -> ExecutionObservation {
        ExecutionObservation {
            contract: EXECUTION_OBSERVATION_CONTRACT.to_owned(),
            observation_id: ObservationId::new(format!("observation.{sequence}")).expect("id"),
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("id"),
            node_execution_id: None,
            occurrence_id: None,
            event_ref: None,
            attempt_id: None,
            region_occurrence_id: None,
            sequence,
            cursor: cursor(sequence),
            timing: ObservationTiming {
                observed_at_unix_ms: 1,
                duration_ms: None,
            },
            observation_kind: kind,
            commitment,
            content_ref: matches!(
                kind,
                ObservationKind::ContentPublished | ObservationKind::ContentCommitted
            )
            .then(|| ContentRef::new(format!("content.{sequence}")).expect("ref")),
            output_ref: (kind == ObservationKind::TerminalCommitted)
                .then(|| OutputRef::new(format!("output.{sequence}")).expect("ref")),
            evidence_ref: matches!(kind, ObservationKind::EvidenceCommitted)
                .then(|| EvidenceRef::new(format!("evidence.{sequence}")).expect("ref")),
            host_capability: matches!(
                kind,
                ObservationKind::CapabilityRequested | ObservationKind::CapabilitySettled
            )
            .then(|| host_capability_observation(kind, sequence)),
        }
    }

    #[test]
    fn opaque_refs_validate_and_reject_unknown_shape() {
        assert!(ProgramInvocationId::new("invocation.1").is_ok());
        assert!(ProgramInvocationId::new("").is_err());
        assert!(ProgramInvocationId::new("has space").is_err());
        let decoded = serde_json::from_value::<ProgramInvocationId>(json!("has space"));
        assert!(decoded.is_err());
    }

    #[test]
    fn cursor_deserialization_validates_opaque_token() {
        let decoded = serde_json::from_value::<ExecutionCursor>(json!({
            "position": 1,
            "token": "has space"
        }));
        assert!(decoded.is_err());
        let unknown = serde_json::from_value::<ExecutionCursor>(json!({
            "position": 1,
            "token": "scope.cursor",
            "extra": true
        }));
        assert!(unknown.is_err());
    }

    #[test]
    fn cursors_and_pages_require_monotonic_positions() {
        let page = ExecutionPage {
            items: vec!["one", "two"],
            item_cursors: vec![cursor(2), cursor(3)],
            next_cursor: Some(cursor(3)),
            high_watermark: cursor(4),
            retention_floor: cursor(1),
            has_more: true,
        };
        assert!(page.validate().is_ok());
        let mut broken = page.clone();
        broken.item_cursors.swap(0, 1);
        assert_eq!(
            broken.validate(),
            Err(ContractValidationError::NonMonotonicPage)
        );
        let mut advanced = page;
        advanced.next_cursor = Some(cursor(4));
        assert_eq!(
            advanced.validate(),
            Err(ContractValidationError::InvalidPageMetadata)
        );
    }

    #[test]
    fn observations_preserve_provisional_and_committed_distinction() {
        let provisional = observation(
            Commitment::Provisional,
            ObservationKind::ContentPublished,
            1,
        );
        let committed = observation(Commitment::Committed, ObservationKind::ContentCommitted, 2);
        assert!(provisional.validate().is_ok());
        assert!(committed.validate().is_ok());
        assert!(
            observation(
                Commitment::Provisional,
                ObservationKind::TerminalCommitted,
                3
            )
            .validate()
            .is_err()
        );
        let encoded = serde_json::to_value(&provisional).expect("encode");
        assert_eq!(encoded["commitment"], "provisional");
        assert_eq!(
            serde_json::to_value(&committed).expect("encode")["commitment"],
            "committed"
        );
    }

    #[test]
    fn event_observations_require_typed_event_identity() {
        let mut waiting = observation(Commitment::Provisional, ObservationKind::EventWaiting, 1);
        assert_eq!(
            waiting.validate(),
            Err(ContractValidationError::InvalidObservationReference)
        );
        waiting.event_ref = Some(EventObservationRef {
            event_ref: "event.1".to_owned(),
            generation: Some(2),
            occurrence_id: Some(OccurrenceId::new("occurrence.1").expect("ref")),
        });
        assert!(waiting.validate().is_ok());

        let mut ordinary = observation(Commitment::Provisional, ObservationKind::NodeStarted, 2);
        ordinary.event_ref = Some(EventObservationRef {
            event_ref: "event.1".to_owned(),
            generation: None,
            occurrence_id: None,
        });
        assert_eq!(
            ordinary.validate(),
            Err(ContractValidationError::InvalidObservationReference)
        );
    }

    #[test]
    fn observation_commitment_kinds_require_readable_references() {
        let mut published = observation(
            Commitment::Provisional,
            ObservationKind::ContentPublished,
            1,
        );
        published.content_ref = None;
        assert_eq!(
            published.validate(),
            Err(ContractValidationError::InvalidObservationCommitment)
        );

        let mut committed =
            observation(Commitment::Committed, ObservationKind::ContentCommitted, 2);
        committed.content_ref = None;
        assert_eq!(
            committed.validate(),
            Err(ContractValidationError::InvalidObservationCommitment)
        );

        let mut evidence =
            observation(Commitment::Committed, ObservationKind::EvidenceCommitted, 3);
        evidence.evidence_ref = None;
        assert_eq!(
            evidence.validate(),
            Err(ContractValidationError::InvalidObservationCommitment)
        );

        let mut failure = observation(Commitment::Committed, ObservationKind::InvocationFailed, 4);
        failure.evidence_ref = Some(EvidenceRef::new("evidence.invoke.commit.1").expect("ref"));
        assert!(failure.validate().is_ok());
        failure.commitment = Commitment::Provisional;
        assert_eq!(
            failure.validate(),
            Err(ContractValidationError::InvalidObservationCommitment)
        );
    }

    #[test]
    fn event_reference_unknown_fields_fail_closed() {
        let decoded = serde_json::from_value::<EventObservationRef>(json!({
            "event_ref": "event.1",
            "unexpected": true
        }));
        assert!(decoded.is_err());
    }

    #[test]
    fn event_reference_enforces_opaque_ref_bound() {
        let boundary = EventObservationRef {
            event_ref: "e".repeat(MAX_REF_BYTES),
            generation: None,
            occurrence_id: None,
        };
        assert!(boundary.validate().is_ok());

        let event_ref = EventObservationRef {
            event_ref: "e".repeat(MAX_REF_BYTES + 1),
            generation: None,
            occurrence_id: None,
        };
        assert_eq!(
            event_ref.validate(),
            Err(ContractValidationError::RefTooLong { kind: "event_ref" })
        );
    }

    #[test]
    fn unknown_observation_kind_fails_closed() {
        let mut value = serde_json::to_value(observation(
            Commitment::Committed,
            ObservationKind::NodeStarted,
            1,
        ))
        .expect("encode");
        value["observation_kind"] = json!("made_up_kind");
        assert!(serde_json::from_value::<ExecutionObservation>(value).is_err());
    }

    #[test]
    fn read_purpose_and_contract_fields_are_product_neutral() {
        let request = ExecutionReadRequest::ContentRead {
            context: ReadContext {
                request_id: RequestId::new("request.1").expect("id"),
                scope_ref: ScopeRef::new("scope.1").expect("id"),
                principal_ref: PrincipalRef::new("principal.1").expect("id"),
                grant_ref: GrantRef::new("grant.1").expect("id"),
                correlation_id: None,
                purpose: ReadPurpose::Content,
            },
            content_ref: ContentRef::new("content.1").expect("id"),
        };
        request.validate().expect("valid read request");
        let text = serde_json::to_string(&request).expect("encode");
        assert!(!text.contains("product_ref"));
        assert!(!text.contains("organization_ref"));
        assert!(text.contains("content.read"));
    }

    #[test]
    fn output_read_is_a_distinct_typed_operation() {
        let request = ExecutionReadRequest::OutputRead {
            context: ReadContext {
                request_id: RequestId::new("request.1").expect("id"),
                scope_ref: ScopeRef::new("scope.1").expect("id"),
                principal_ref: PrincipalRef::new("principal.1").expect("id"),
                grant_ref: GrantRef::new("grant.1").expect("id"),
                correlation_id: None,
                purpose: ReadPurpose::Output,
            },
            output_ref: OutputRef::new("output.1").expect("id"),
        };
        request.validate().expect("valid output read request");
        let encoded = serde_json::to_value(&request).expect("encode");
        assert_eq!(encoded["method"], "output.read");
        assert_eq!(encoded["output_ref"], "output.1");
        assert_eq!(encoded["context"]["purpose"], "output");
    }

    #[test]
    fn inspection_request_selects_invocation_or_node_result_mode() {
        let context = ReadContext {
            request_id: RequestId::new("request.1").expect("id"),
            scope_ref: ScopeRef::new("scope.1").expect("id"),
            principal_ref: PrincipalRef::new("principal.1").expect("id"),
            grant_ref: GrantRef::new("grant.1").expect("id"),
            correlation_id: None,
            purpose: ReadPurpose::Inspection,
        };
        let invocation = ExecutionReadRequest::ProgramInvocationInspect {
            context: context.clone(),
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("id"),
            node_execution_id: None,
        };
        let node = ExecutionReadRequest::ProgramInvocationInspect {
            context,
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("id"),
            node_execution_id: Some(NodeExecutionId::new("node.1").expect("id")),
        };
        assert!(serde_json::to_value(invocation).expect("encode")["node_execution_id"].is_null());
        assert_eq!(
            serde_json::to_value(node).expect("encode")["node_execution_id"],
            json!("node.1")
        );
    }

    #[test]
    fn output_ref_requires_digest_and_exact_contract() {
        let output = SessionOutputRef {
            contract: SESSION_OUTPUT_REF_CONTRACT.to_owned(),
            ref_type: "SessionOutputRef".to_owned(),
            reference: OutputRef::new("output.1").expect("id"),
            program_instance_id: ProgramInstanceId::new("instance.1").expect("id"),
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("id"),
            node_execution_id: None,
            occurrence_id: None,
            content_digest: format!("sha256:{}", "a".repeat(64)),
            byte_length: 4,
            media_type: "text/plain".to_owned(),
            visibility: OutputVisibility::Committed,
            access_scope_ref: "scope.1".to_owned(),
            disclosure_ref: None,
        };
        assert!(output.validate().is_ok());
        let mut invalid = output;
        invalid.content_digest = "sha256:not-a-digest".to_owned();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn committed_content_rejects_provisional_nested_output_ref() {
        let bytes = b"test".to_vec();
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        let mut output_ref = SessionOutputRef {
            contract: SESSION_OUTPUT_REF_CONTRACT.to_owned(),
            ref_type: "SessionOutputRef".to_owned(),
            reference: OutputRef::new("output.1").expect("ref"),
            program_instance_id: ProgramInstanceId::new("instance.1").expect("id"),
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("id"),
            node_execution_id: None,
            occurrence_id: None,
            content_digest: digest.clone(),
            byte_length: bytes.len() as u64,
            media_type: "text/plain".to_owned(),
            visibility: OutputVisibility::Provisional,
            access_scope_ref: "scope.1".to_owned(),
            disclosure_ref: None,
        };
        let mut content = ContentReadResult {
            content_ref: ContentRef::new("output.1").expect("ref"),
            program_instance_id: ProgramInstanceId::new("instance.1").expect("id"),
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("id"),
            node_execution_id: None,
            occurrence_id: None,
            output_ref: Some(output_ref.clone()),
            content_digest: digest,
            byte_length: bytes.len() as u64,
            media_type: "text/plain".to_owned(),
            visibility: OutputVisibility::Committed,
            access_scope_ref: "scope.1".to_owned(),
            disclosure_ref: None,
            bytes,
        };
        assert_eq!(
            content.validate(),
            Err(ContractValidationError::InvalidReadRequest)
        );

        output_ref.visibility = OutputVisibility::Committed;
        content.output_ref = Some(output_ref);
        assert!(content.validate().is_ok());
    }

    #[test]
    fn node_inspection_rejects_duplicate_typed_references() {
        let inspection = NodeExecutionInspection {
            contract: NODE_EXECUTION_INSPECTION_CONTRACT.to_owned(),
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("id"),
            node_execution_id: NodeExecutionId::new("node.1").expect("id"),
            air_node_id: "node.1".to_owned(),
            parent_node_execution_id: None,
            occurrence_id: None,
            region_occurrence_id: None,
            status: NodeExecutionStatus::Running,
            commitment: Commitment::Provisional,
            input_ref: None,
            prompt_or_request_ref: None,
            provisional_response_ref: None,
            final_response_ref: None,
            output_ref: None,
            attempt_refs: vec![AttemptId::new("attempt.1").expect("id")],
            metrics_ref: None,
            usage_ref: None,
            policy_ref: None,
            approval_ref: None,
            error_ref: None,
            trace_ref: None,
            evidence_refs: vec![EvidenceRef::new("evidence.1").expect("id")],
            child_program_refs: vec![ProgramRef::new("program.1").expect("id")],
        };
        assert!(inspection.validate().is_ok());
        let mut duplicate = inspection;
        duplicate
            .evidence_refs
            .push(EvidenceRef::new("evidence.1").expect("id"));
        assert_eq!(
            duplicate.validate(),
            Err(ContractValidationError::DuplicateReference { kind: "evidence" })
        );
        let mut wire = serde_json::to_value(duplicate).expect("encode duplicate");
        wire["evidence_refs"] = json!(["evidence.1", "evidence.1"]);
        assert!(serde_json::from_value::<NodeExecutionInspection>(wire).is_err());
    }

    #[test]
    fn node_inspection_must_match_the_requested_invocation_and_node() {
        let request = ExecutionReadRequest::ProgramInvocationInspect {
            context: ReadContext {
                request_id: RequestId::new("request.1").expect("id"),
                scope_ref: ScopeRef::new("scope.1").expect("id"),
                principal_ref: PrincipalRef::new("principal.1").expect("id"),
                grant_ref: GrantRef::new("grant.1").expect("id"),
                correlation_id: None,
                purpose: ReadPurpose::Inspection,
            },
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("id"),
            node_execution_id: Some(NodeExecutionId::new("node.1").expect("id")),
        };
        let inspection = NodeExecutionInspection {
            contract: NODE_EXECUTION_INSPECTION_CONTRACT.to_owned(),
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("id"),
            node_execution_id: NodeExecutionId::new("node.1").expect("id"),
            air_node_id: "node.model".to_owned(),
            parent_node_execution_id: None,
            occurrence_id: None,
            region_occurrence_id: None,
            status: NodeExecutionStatus::Running,
            commitment: Commitment::Provisional,
            input_ref: None,
            prompt_or_request_ref: None,
            provisional_response_ref: None,
            final_response_ref: None,
            output_ref: None,
            attempt_refs: vec![],
            metrics_ref: None,
            usage_ref: None,
            policy_ref: None,
            approval_ref: None,
            error_ref: None,
            trace_ref: None,
            evidence_refs: vec![],
            child_program_refs: vec![],
        };
        assert!(request.validate_node_inspection(&inspection).is_ok());
        let mut cross_invocation = inspection.clone();
        cross_invocation.program_invocation_id =
            ProgramInvocationId::new("invocation.2").expect("id");
        assert_eq!(
            request.validate_node_inspection(&cross_invocation),
            Err(ContractValidationError::InvalidReadRequest)
        );
        let mut unknown_node = inspection;
        unknown_node.node_execution_id = NodeExecutionId::new("node.unknown").expect("id");
        assert_eq!(
            request.validate_node_inspection(&unknown_node),
            Err(ContractValidationError::InvalidReadRequest)
        );
    }

    #[test]
    fn invocation_status_and_evidence_kind_are_closed() {
        let status = serde_json::from_value::<ProgramInvocationStatus>(json!("outcome_unknown"))
            .expect("canonical status");
        assert_eq!(status, ProgramInvocationStatus::OutcomeUnknown);
        assert!(serde_json::from_value::<ProgramInvocationStatus>(json!("succeeded")).is_err());
        assert!(serde_json::from_value::<EvidenceFactKind>(json!("made_up")).is_err());
    }

    #[test]
    fn read_result_uses_typed_page_envelope_and_omits_absent_optionals() {
        let page = ExecutionPage {
            items: vec![observation(
                Commitment::Committed,
                ObservationKind::NodeStarted,
                1,
            )],
            item_cursors: vec![cursor(1)],
            next_cursor: None,
            high_watermark: cursor(1),
            retention_floor: cursor(1),
            has_more: false,
        };
        page.validate().expect("valid observation page");
        let result = ExecutionReadResult::ObservationPage { page };
        let encoded = serde_json::to_value(result).expect("encode read result");
        assert_eq!(encoded["kind"], "observation_page");
        assert_eq!(encoded["page"]["items"][0]["sequence"], 1);
        assert!(
            !encoded["page"]
                .as_object()
                .unwrap()
                .contains_key("next_cursor")
        );
        assert!(
            !encoded["page"]["items"][0]
                .as_object()
                .unwrap()
                .contains_key("content_ref")
        );
    }
}
