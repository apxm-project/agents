//! Runtime-owned post-commit native model usage handoff.
//!
//! The runtime publishes the exact typed attempt already present in the atomic
//! evidence commit. Composition can therefore verify membership without
//! trusting a parallel, independently assembled DTO.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use apxm_inference::{InferenceUsageLineage, LineageError, Usage};
use apxm_program::runtime_evidence::ModelAttemptRecordedFact;

/// The single accepted schema version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommittedNativeModelUsageVersion {
    #[serde(rename = "apxm.committed-native-model-usage")]
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
        hasher.update(b"apxm.committed-native-model-usage\0");
        hasher.update(commit_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(attempt_fact_id.as_bytes());
        format!("sha256:{:x}", hasher.finalize())
    }

    /// Build one publishable usage measurement only from commit-bound sealed
    /// lineage evidence.
    pub fn from_lineage(
        commit_id: impl Into<String>,
        evidence_position_ref: EvidencePositionRef,
        attempt: ModelAttemptRecordedFact,
        lineage: &InferenceUsageLineage,
    ) -> Result<Self, CommittedNativeModelUsageGateError> {
        let commit_id = commit_id.into();
        if commit_id.trim().is_empty() {
            return Err(CommittedNativeModelUsageGateError::EmptyField("commit_id"));
        }
        if evidence_position_ref.r#ref.trim().is_empty() {
            return Err(CommittedNativeModelUsageGateError::EmptyField(
                "evidence_position_ref.ref",
            ));
        }
        lineage
            .authorize_exporter_claim(
                &commit_id,
                Usage {
                    input_tokens: attempt.native_input_tokens,
                    output_tokens: attempt.native_output_tokens,
                },
            )
            .map_err(CommittedNativeModelUsageGateError::Lineage)?;
        if lineage.evidence_fact_id.as_deref() != Some(attempt.fact_id.as_str()) {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "fact_id",
            ));
        }
        if lineage.effect_id != attempt.model_effect_id {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "model_effect_id",
            ));
        }
        if lineage.attempt_index != attempt.attempt_index {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "attempt_index",
            ));
        }
        if lineage.request_digest != attempt.request_digest {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "request_digest",
            ));
        }
        if lineage.model_target_ref != attempt.model_target_ref {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "model_target_ref",
            ));
        }
        if lineage.model_target_digest != attempt.model_target_digest {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "model_target_digest",
            ));
        }
        if lineage.model_deployment_ref != attempt.model_deployment_ref {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "model_deployment_ref",
            ));
        }
        if lineage.exact_port_binding_digest != attempt.exact_port_binding_digest {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "exact_port_binding_digest",
            ));
        }
        if lineage.target_commitment_digest.as_deref()
            != Some(attempt.target_commitment_digest.as_str())
        {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "target_commitment_digest",
            ));
        }
        if lineage.generation_cohort_digest.as_deref()
            != Some(attempt.generation_cohort_digest.as_str())
        {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "generation_cohort_digest",
            ));
        }
        if lineage.target_generation != Some(attempt.target_generation) {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "target_generation",
            ));
        }
        if lineage.target_port_contract_digest.as_deref()
            != Some(attempt.target_port_contract_digest.as_str())
        {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "target_port_contract_digest",
            ));
        }
        if lineage.target_composition_digest.as_deref()
            != Some(attempt.target_composition_digest.as_str())
        {
            return Err(CommittedNativeModelUsageGateError::EvidenceMismatch(
                "target_composition_digest",
            ));
        }
        Ok(Self {
            schema_version: CommittedNativeModelUsageVersion::V1,
            source_contract_digest: Self::SOURCE_CONTRACT_DIGEST.to_string(),
            usage_measurement_id: Self::measurement_id(&commit_id, &attempt.fact_id),
            commit_id,
            evidence_position_ref,
            attempt,
        })
    }
}

/// Why post-commit native usage publication is rejected before calling a
/// composition publisher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommittedNativeModelUsageGateError {
    EmptyField(&'static str),
    Lineage(LineageError),
    EvidenceMismatch(&'static str),
}

impl fmt::Display for CommittedNativeModelUsageGateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => {
                write!(formatter, "native usage gate field {field} is empty")
            }
            Self::Lineage(error) => write!(formatter, "{error}"),
            Self::EvidenceMismatch(field) => {
                write!(formatter, "native usage gate evidence mismatch for {field}")
            }
        }
    }
}

impl std::error::Error for CommittedNativeModelUsageGateError {}

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
