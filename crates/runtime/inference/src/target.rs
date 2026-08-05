//! Immutable target commitment for one inference invocation.
//!
//! A target reference is not enough to identify what an invocation executed:
//! deployment and binding records can move independently.  This value freezes
//! the complete target tuple and its generation cohort before a driver may
//! redeem a credential or send a request.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::identity::{ModelDeploymentRef, ModelTargetRef, ResolvedModelBinding};
use apxm_program::grammar::is_digest;

/// Schema identity for the driver-owned target commitment.
pub const INFERENCE_TARGET_COMMITMENT_SCHEMA: &str = "apxm.inference-target-commitment.v1";

/// Closed state vocabulary for a target snapshot presented to admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetCommitState {
    Committed,
    Moving,
    Stale,
    Dirty,
    MixedGeneration,
    Ambiguous,
}

/// One immutable target/deployment/binding commitment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceTargetCommitment {
    pub schema_version: String,
    pub target_ref: String,
    pub target_digest: String,
    pub model_deployment_ref: String,
    pub exact_port_binding_digest: String,
    pub port_contract_digest: String,
    pub composition_digest: String,
    pub target_generation: u64,
    pub deployment_generation: u64,
    pub binding_generation: u64,
    pub composition_generation: u64,
    pub generation_cohort_digest: String,
    pub commit_digest: String,
    pub state: TargetCommitState,
}

/// Why an inference target commitment cannot authorize an invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetCommitmentError {
    EmptyField(&'static str),
    InvalidDigest(&'static str),
    InvalidSchema,
    Moving,
    Stale,
    Dirty,
    MixedGeneration,
    Ambiguous,
    CohortDigestMismatch,
    CommitDigestMismatch,
    TargetMismatch,
    DeploymentMismatch,
    BindingMismatch,
    PortContractMismatch,
    CompositionMismatch,
}

impl std::fmt::Display for TargetCommitmentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField(field) => {
                write!(formatter, "target commitment field {field} is empty")
            }
            Self::InvalidDigest(field) => {
                write!(
                    formatter,
                    "target commitment field {field} is not a sha256 digest"
                )
            }
            Self::InvalidSchema => write!(formatter, "target commitment schema is not supported"),
            Self::Moving => write!(formatter, "inference target is moving"),
            Self::Stale => write!(formatter, "inference target commitment is stale"),
            Self::Dirty => write!(formatter, "inference target provenance is dirty"),
            Self::MixedGeneration => write!(formatter, "inference target uses mixed generations"),
            Self::Ambiguous => write!(formatter, "inference target commitment is ambiguous"),
            Self::CohortDigestMismatch => {
                write!(
                    formatter,
                    "inference target generation cohort digest does not match"
                )
            }
            Self::CommitDigestMismatch => {
                write!(formatter, "inference target commit digest does not match")
            }
            Self::TargetMismatch => write!(formatter, "inference target does not match admission"),
            Self::DeploymentMismatch => {
                write!(formatter, "inference deployment does not match admission")
            }
            Self::BindingMismatch => {
                write!(formatter, "inference Port Binding does not match admission")
            }
            Self::PortContractMismatch => {
                write!(
                    formatter,
                    "inference Port Contract does not match admission"
                )
            }
            Self::CompositionMismatch => {
                write!(formatter, "inference composition does not match admission")
            }
        }
    }
}

impl std::error::Error for TargetCommitmentError {}

impl InferenceTargetCommitment {
    /// Commit one exact tuple at one generation. All components are stamped
    /// with the same generation; callers cannot create a mixed commitment via
    /// this constructor.
    pub fn commit(
        target_ref: impl Into<String>,
        target_digest: impl Into<String>,
        model_deployment_ref: impl Into<String>,
        exact_port_binding_digest: impl Into<String>,
        port_contract_digest: impl Into<String>,
        composition_digest: impl Into<String>,
        generation: u64,
    ) -> Result<Self, TargetCommitmentError> {
        let target_ref = target_ref.into();
        let target_digest = target_digest.into();
        let model_deployment_ref = model_deployment_ref.into();
        let exact_port_binding_digest = exact_port_binding_digest.into();
        let port_contract_digest = port_contract_digest.into();
        let composition_digest = composition_digest.into();
        let mut commitment = Self {
            schema_version: INFERENCE_TARGET_COMMITMENT_SCHEMA.to_string(),
            target_ref,
            target_digest,
            model_deployment_ref,
            exact_port_binding_digest,
            port_contract_digest,
            composition_digest,
            target_generation: generation,
            deployment_generation: generation,
            binding_generation: generation,
            composition_generation: generation,
            generation_cohort_digest: String::new(),
            commit_digest: String::new(),
            state: TargetCommitState::Committed,
        };
        commitment.generation_cohort_digest = commitment.expected_cohort_digest();
        commitment.commit_digest = commitment.expected_commit_digest();
        commitment.validate()?;
        Ok(commitment)
    }

    /// Read the already committed snapshot carried by an admitted resolution.
    /// Generation is never invented at this boundary.
    pub fn from_resolved(resolved: &ResolvedModelBinding) -> Result<Self, TargetCommitmentError> {
        resolved.target_commitment.validate()?;
        Ok(resolved.target_commitment.clone())
    }

    /// Validate a wire candidate before any target or credential is used.
    pub fn validate(&self) -> Result<(), TargetCommitmentError> {
        if self.schema_version != INFERENCE_TARGET_COMMITMENT_SCHEMA {
            return Err(TargetCommitmentError::InvalidSchema);
        }
        for (field, value) in [
            ("target_ref", self.target_ref.as_str()),
            ("model_deployment_ref", self.model_deployment_ref.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(TargetCommitmentError::EmptyField(field));
            }
        }
        for (field, digest) in [
            ("target_digest", self.target_digest.as_str()),
            (
                "exact_port_binding_digest",
                self.exact_port_binding_digest.as_str(),
            ),
            ("port_contract_digest", self.port_contract_digest.as_str()),
            ("composition_digest", self.composition_digest.as_str()),
            (
                "generation_cohort_digest",
                self.generation_cohort_digest.as_str(),
            ),
            ("commit_digest", self.commit_digest.as_str()),
        ] {
            if !is_digest(digest) {
                return Err(TargetCommitmentError::InvalidDigest(field));
            }
        }
        if self.target_generation != self.deployment_generation
            || self.target_generation != self.binding_generation
            || self.target_generation != self.composition_generation
        {
            return Err(TargetCommitmentError::MixedGeneration);
        }
        match self.state {
            TargetCommitState::Committed => {}
            TargetCommitState::Moving => return Err(TargetCommitmentError::Moving),
            TargetCommitState::Stale => return Err(TargetCommitmentError::Stale),
            TargetCommitState::Dirty => return Err(TargetCommitmentError::Dirty),
            TargetCommitState::MixedGeneration => {
                return Err(TargetCommitmentError::MixedGeneration);
            }
            TargetCommitState::Ambiguous => return Err(TargetCommitmentError::Ambiguous),
        }
        let expected_cohort_digest = self.expected_cohort_digest();
        // The commit digest is the outer seal over the canonical cohort. Check
        // it against the tuple-derived cohort first so a changed identity is
        // reported as a commitment tamper, preserving the public taxonomy.
        if self.expected_commit_digest_for(&expected_cohort_digest) != self.commit_digest {
            return Err(TargetCommitmentError::CommitDigestMismatch);
        }
        if expected_cohort_digest != self.generation_cohort_digest {
            return Err(TargetCommitmentError::CohortDigestMismatch);
        }
        Ok(())
    }

    /// Verify that this commitment is exactly the binding supplied by
    /// invocation admission.
    pub fn matches_resolved(
        &self,
        target: &ModelTargetRef,
        resolved: &ResolvedModelBinding,
    ) -> Result<(), TargetCommitmentError> {
        self.validate()?;
        if self.target_ref != target.0 || self.target_ref != resolved.model_target.reference.0 {
            return Err(TargetCommitmentError::TargetMismatch);
        }
        if self.target_digest != resolved.model_target.target_digest {
            return Err(TargetCommitmentError::TargetMismatch);
        }
        if self.model_deployment_ref != resolved.model_deployment_ref.0 {
            return Err(TargetCommitmentError::DeploymentMismatch);
        }
        if self.exact_port_binding_digest != resolved.binding_digest() {
            return Err(TargetCommitmentError::BindingMismatch);
        }
        if self.port_contract_digest != resolved.port_contract_digest() {
            return Err(TargetCommitmentError::PortContractMismatch);
        }
        if self.composition_digest != resolved.composition_digest {
            return Err(TargetCommitmentError::CompositionMismatch);
        }
        Ok(())
    }

    #[must_use]
    pub fn target(&self) -> ModelTargetRef {
        ModelTargetRef(self.target_ref.clone())
    }

    #[must_use]
    pub fn deployment(&self) -> ModelDeploymentRef {
        ModelDeploymentRef(self.model_deployment_ref.clone())
    }

    fn expected_cohort_digest(&self) -> String {
        let target_generation = self.target_generation.to_string();
        let deployment_generation = self.deployment_generation.to_string();
        let binding_generation = self.binding_generation.to_string();
        let composition_generation = self.composition_generation.to_string();
        digest_parts(
            b"cohort",
            &[
                self.target_ref.as_str(),
                self.target_digest.as_str(),
                self.model_deployment_ref.as_str(),
                self.exact_port_binding_digest.as_str(),
                self.port_contract_digest.as_str(),
                self.composition_digest.as_str(),
                target_generation.as_str(),
                deployment_generation.as_str(),
                binding_generation.as_str(),
                composition_generation.as_str(),
            ],
        )
    }

    fn expected_commit_digest(&self) -> String {
        self.expected_commit_digest_for(&self.generation_cohort_digest)
    }

    fn expected_commit_digest_for(&self, generation_cohort_digest: &str) -> String {
        digest_parts(b"commit", &[generation_cohort_digest, self.state.as_str()])
    }
}

impl TargetCommitState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Moving => "moving",
            Self::Stale => "stale",
            Self::Dirty => "dirty",
            Self::MixedGeneration => "mixed_generation",
            Self::Ambiguous => "ambiguous",
        }
    }
}

fn digest_parts(domain: &[u8], parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(INFERENCE_TARGET_COMMITMENT_SCHEMA.as_bytes());
    hasher.update(b"\0");
    hasher.update(domain);
    for part in parts {
        hasher.update(b"\0");
        hasher.update(part.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}
