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
    "sha256:118ad96d04eea06b729f8ad1ab898b072f7fab7b561f04427dd1c808e39f268c";

/// Exact external vLLM release identity independently verified by the release
/// owner. These values are admission pins, not caller-provided claims.
pub const PINNED_VLLM_RELEASE_ID: &str = "apxm-vllm-863e2bfcd2d3";
pub const PINNED_VLLM_OWNER_REVISION: &str = "863e2bfcd2d3ab52b2a93e830fcea239139764ad";
pub const PINNED_VLLM_RELEASE_MANIFEST_DIGEST: &str =
    "sha256:49c3085e7a971cba4529566ac640c86a3684c7fd7d4c037b210a9959b88e4e69";

/// Pinned conformance vector digests from the vLLM owner vectors directory.
pub const PINNED_VLLM_VECTOR_DIGESTS: &[&str] = &[
    "sha256:7add76f8f7df341785ef45ff51b38e979a309299509b32e76b311268cc93ea32", // request
    "sha256:00a8657980dfd3f49859d05f62955e430d15dd50125c91605d676830abf08bfa", // result
    "sha256:96914ff1d56cc2d1e74fda3a063287615392cb0197bcc1c853cb1a18a5e7e8c3", // failure
    "sha256:5e7abcccf5c7f7276b9398ccd2c255671f23a6914eb23287b44518b30b8c7723", // stream-chunk
    "sha256:1e9a389b24f08411738594ca3646bbcbeb18a77c398c6b07933d04903bd9be39", // native-serving-binding
];

fn pinned_vllm_vector_digests() -> Vec<String> {
    PINNED_VLLM_VECTOR_DIGESTS
        .iter()
        .map(|digest| (*digest).to_string())
        .collect()
}

/// Join status relative to an immutable vLLM release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VllmJoinStatus {
    Joined,
    CandidateAwaitingVllmRelease,
}

/// External release-owner evidence required before a join can become Joined.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VllmReleaseAttestation {
    pub release_id: String,
    pub owner_revision: String,
    pub manifest_digest: String,
    pub port_contract_digest: String,
    pub vector_digests: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_attestation: Option<VllmReleaseAttestation>,
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
    InvalidReleaseAttestation(&'static str),
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
            Self::InvalidReleaseAttestation(field) => {
                write!(
                    f,
                    "external vLLM release attestation field {field} is not verified"
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
            release_attestation: None,
        })
    }

    /// Admit a released join only from independently verified external owner
    /// evidence. The boolean `released` path above intentionally cannot do so.
    pub fn join_with_release_attestation(
        vllm_port_contract_digest: impl Into<String>,
        joined_vector_digests: Vec<String>,
        release_attestation: VllmReleaseAttestation,
    ) -> Result<Self, JoinError> {
        let candidate = Self::join(vllm_port_contract_digest, joined_vector_digests, false)?;
        release_attestation.verify()?;
        if candidate.joined_vector_digests != release_attestation.vector_digests {
            return Err(JoinError::InvalidReleaseAttestation("vector_digests"));
        }
        Ok(Self {
            join_status: VllmJoinStatus::Joined,
            release_attestation: Some(release_attestation),
            ..candidate
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
        let rebuilt = match self.join_status {
            VllmJoinStatus::Joined => Self::join_with_release_attestation(
                self.vllm_port_contract_digest.clone(),
                self.joined_vector_digests.clone(),
                self.release_attestation
                    .clone()
                    .ok_or(JoinError::ReleaseEvidenceRequired)?,
            )?,
            VllmJoinStatus::CandidateAwaitingVllmRelease => {
                if self.release_attestation.is_some() {
                    return Err(JoinError::InvalidReleaseAttestation("candidate_status"));
                }
                Self::join(
                    self.vllm_port_contract_digest.clone(),
                    self.joined_vector_digests.clone(),
                    false,
                )?
            }
        };
        if rebuilt != *self {
            return Err(JoinError::UnknownVectorDigest(
                self.vllm_port_contract_digest.clone(),
            ));
        }
        Ok(())
    }
}

impl VllmReleaseAttestation {
    /// Verify all release-owner coordinates and exact vector membership.
    pub fn verify(&self) -> Result<(), JoinError> {
        if self.release_id != PINNED_VLLM_RELEASE_ID {
            return Err(JoinError::InvalidReleaseAttestation("release_id"));
        }
        if self.owner_revision != PINNED_VLLM_OWNER_REVISION {
            return Err(JoinError::InvalidReleaseAttestation("owner_revision"));
        }
        if self.manifest_digest != PINNED_VLLM_RELEASE_MANIFEST_DIGEST
            || !is_digest(&self.manifest_digest)
        {
            return Err(JoinError::InvalidReleaseAttestation("manifest_digest"));
        }
        if self.port_contract_digest != PINNED_VLLM_PORT_CONTRACT_DIGEST {
            return Err(JoinError::InvalidReleaseAttestation("port_contract_digest"));
        }
        if self.vector_digests != pinned_vllm_vector_digests() {
            return Err(JoinError::InvalidReleaseAttestation("vector_digests"));
        }
        Ok(())
    }
}

/// Digest file bytes with the same sha256:hex encoding used by APXM contracts.
#[must_use]
pub fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
