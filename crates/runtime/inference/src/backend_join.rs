//! Join exact vLLM conformance evidence into the agents inference owner.
//!
//! Agents verifies digest membership of pinned backend vectors and keeps them
//! candidate-only until external release evidence exists. It does not become a
//! second owner of vLLM semantics or stub backend authority.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use apxm_program::grammar::is_digest;

/// Schema identity for the vLLM conformance join record.
pub const VLLM_CONFORMANCE_JOIN_SCHEMA: &str = "apxm.vllm-conformance-join.v1";

/// Current released-or-candidate vLLM port-contract digest published by the
/// vLLM owner descriptor. G4 joins against this exact digest; a future vLLM
/// release may advance it, at which point this constant is updated with the
/// released value (never silently substituted at runtime).
pub const PINNED_VLLM_PORT_CONTRACT_DIGEST: &str =
    "sha256:2106082c92a9dae2cd9e0c623315198f9ed8d736e8f5860ed043c58690428c01";

/// Pinned conformance vector digests from the vLLM owner vectors directory.
pub const PINNED_VLLM_VECTOR_DIGESTS: &[&str] = &[
    "sha256:7add76f8f7df341785ef45ff51b38e979a309299509b32e76b311268cc93ea32", // request
    "sha256:a60b2364bbbe1304e96defcf55a8600d501933e0fbae97c140f2bc4377d5e822", // result
    "sha256:96914ff1d56cc2d1e74fda3a063287615392cb0197bcc1c853cb1a18a5e7e8c3", // failure
    "sha256:5e7abcccf5c7f7276b9398ccd2c255671f23a6914eb23287b44518b30b8c7723", // stream-chunk
    "sha256:1e9a389b24f08411738594ca3646bbcbeb18a77c398c6b07933d04903bd9be39", // native-serving-binding
];

/// Join status relative to an immutable vLLM release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VllmJoinStatus {
    Joined,
    CandidateAwaitingVllmRelease,
}

/// Agents-owned join record for candidate or released vLLM conformance digests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VllmConformanceJoin {
    pub schema_version: String,
    pub agents_driver_contract_id: String,
    pub vllm_port_contract_id: String,
    pub vllm_port_contract_digest: String,
    pub joined_vector_digests: Vec<String>,
    pub join_status: VllmJoinStatus,
}

/// Why a join cannot be accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinError {
    InvalidDigest(&'static str),
    EmptyVectors,
    UnknownVectorDigest(String),
    PortContractMismatch { expected: String, actual: String },
    DuplicateVectorDigest(String),
    ReleaseEvidenceRequired,
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDigest(field) => write!(f, "join field {field} is not a sha256 digest"),
            Self::EmptyVectors => write!(f, "join requires at least one vector digest"),
            Self::UnknownVectorDigest(digest) => {
                write!(f, "vector digest {digest} is not in the pinned vLLM set")
            }
            Self::PortContractMismatch { expected, actual } => write!(
                f,
                "vLLM port-contract digest {actual} does not match pinned {expected}"
            ),
            Self::DuplicateVectorDigest(digest) => {
                write!(f, "vector digest {digest} occurs more than once")
            }
            Self::ReleaseEvidenceRequired => {
                write!(
                    f,
                    "joined status requires exact external vLLM release evidence"
                )
            }
        }
    }
}

impl std::error::Error for JoinError {}

impl VllmConformanceJoin {
    /// Build a join record from an exact vLLM port-contract digest and vector
    /// digests. Unknown digests fail closed; there is no silent substitution.
    /// The `released` flag is retained as an explicit call-site assertion, but
    /// it cannot manufacture the external release evidence required for a
    /// joined record.
    pub fn join(
        vllm_port_contract_digest: impl Into<String>,
        joined_vector_digests: Vec<String>,
        released: bool,
    ) -> Result<Self, JoinError> {
        let vllm_port_contract_digest = vllm_port_contract_digest.into();
        if !is_digest(&vllm_port_contract_digest) {
            return Err(JoinError::InvalidDigest("vllm_port_contract_digest"));
        }
        if vllm_port_contract_digest != PINNED_VLLM_PORT_CONTRACT_DIGEST {
            return Err(JoinError::PortContractMismatch {
                expected: PINNED_VLLM_PORT_CONTRACT_DIGEST.to_string(),
                actual: vllm_port_contract_digest,
            });
        }
        if joined_vector_digests.is_empty() {
            return Err(JoinError::EmptyVectors);
        }
        for digest in &joined_vector_digests {
            if !is_digest(digest) {
                return Err(JoinError::InvalidDigest("joined_vector_digests"));
            }
            if !PINNED_VLLM_VECTOR_DIGESTS.contains(&digest.as_str()) {
                return Err(JoinError::UnknownVectorDigest(digest.clone()));
            }
        }
        let mut sorted = joined_vector_digests.clone();
        sorted.sort_unstable();
        if let Some(duplicate) = sorted
            .windows(2)
            .find_map(|window| (window[0] == window[1]).then(|| window[0].clone()))
        {
            return Err(JoinError::DuplicateVectorDigest(duplicate));
        }
        if released {
            return Err(JoinError::ReleaseEvidenceRequired);
        }
        Ok(Self {
            schema_version: VLLM_CONFORMANCE_JOIN_SCHEMA.to_string(),
            agents_driver_contract_id: "apxm.inference-driver-binding.v1".to_string(),
            vllm_port_contract_id: "apxm.vllm-inference.v1".to_string(),
            vllm_port_contract_digest,
            joined_vector_digests,
            join_status: VllmJoinStatus::CandidateAwaitingVllmRelease,
        })
    }

    /// Candidate join against the currently pinned vLLM digests while P-011
    /// release evidence remains open. Does not invent backend behavior.
    pub fn candidate_from_pinned_vectors() -> Result<Self, JoinError> {
        Self::join(
            PINNED_VLLM_PORT_CONTRACT_DIGEST,
            PINNED_VLLM_VECTOR_DIGESTS
                .iter()
                .map(|digest| (*digest).to_string())
                .collect(),
            false,
        )
    }

    /// Validate a deserialized join record against the exact cross-owner pins.
    pub fn validate(&self) -> Result<(), JoinError> {
        if self.schema_version != VLLM_CONFORMANCE_JOIN_SCHEMA
            || self.agents_driver_contract_id != "apxm.inference-driver-binding.v1"
            || self.vllm_port_contract_id != "apxm.vllm-inference.v1"
        {
            return Err(JoinError::InvalidDigest("schema_or_contract_id"));
        }
        let rebuilt = Self::join(
            self.vllm_port_contract_digest.clone(),
            self.joined_vector_digests.clone(),
            matches!(self.join_status, VllmJoinStatus::Joined),
        )?;
        if rebuilt != *self {
            return Err(JoinError::UnknownVectorDigest(
                self.vllm_port_contract_digest.clone(),
            ));
        }
        Ok(())
    }
}

/// Digest file bytes with the same sha256:hex encoding used by APXM contracts.
#[must_use]
pub fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
