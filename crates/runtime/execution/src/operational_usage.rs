//! Runtime-owned post-commit native model usage handoff.
//!
//! The runtime publishes the exact typed attempt already present in the atomic
//! evidence commit. Composition can therefore verify membership without
//! trusting a parallel, independently assembled DTO.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use apxm_program::runtime_evidence::ModelAttemptRecordedFact;

/// The single accepted schema version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommittedNativeModelUsageVersion {
    #[serde(rename = "apxm.committed-native-model-usage.v1")]
    V1,
}

/// The closed evidence-position reference kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidencePositionRefType {
    #[serde(rename = "EvidencePositionRef")]
    EvidencePositionRef,
}

/// The committed evidence position returned by the atomic commit port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePositionRef {
    pub ref_type: EvidencePositionRefType,
    pub r#ref: String,
}

/// One Agents-owned native model usage measurement published only after the
/// atomic commit succeeds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedNativeModelUsage {
    pub schema_version: CommittedNativeModelUsageVersion,
    pub source_contract_digest: String,
    pub usage_measurement_id: String,
    pub commit_id: String,
    pub evidence_position_ref: EvidencePositionRef,
    pub attempt: ModelAttemptRecordedFact,
}

impl CommittedNativeModelUsage {
    /// Digest of the exact schema bytes hashed by this crate's build script.
    pub const SOURCE_CONTRACT_DIGEST: &'static str =
        env!("APXM_COMMITTED_NATIVE_MODEL_USAGE_SCHEMA_DIGEST");

    /// Derive the replay-stable identity for one committed attempt.
    #[must_use]
    pub fn measurement_id(commit_id: &str, attempt_fact_id: &str) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(b"apxm.committed-native-model-usage.v1\0");
        hasher.update(commit_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(attempt_fact_id.as_bytes());
        format!("sha256:{:x}", hasher.finalize())
    }
}

/// The narrow post-commit publication seam supplied by composition.
#[async_trait]
pub trait CommittedNativeModelUsagePort: Send + Sync {
    async fn publish(
        &self,
        usage: CommittedNativeModelUsage,
    ) -> Result<(), CommittedNativeModelUsageError>;
}

/// A composition publisher's closed delivery result as reported by runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommittedNativeModelUsageError {
    Unavailable,
    Rejected,
}

impl fmt::Display for CommittedNativeModelUsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for CommittedNativeModelUsageError {}

/// The report-visible post-commit publication outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommittedNativeModelUsageOutcome {
    NotApplicable,
    NotConfigured,
    Published,
    Failed(CommittedNativeModelUsageError),
}
