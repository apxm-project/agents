//! The Confinement port.
//!
//! Confinement binds an execution to one exact admitted sandbox before any
//! peer/handler/process runs. An unadmitted sandbox digest fails closed with a
//! typed error — there is no unsandboxed or "best effort" degraded mode. The
//! port is defined here and implemented by injected adapters.

use async_trait::async_trait;

/// Closed confinement technology set mirroring the owner attestation schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfinementType {
    Wasm,
    Docker,
    Gvisor,
    Firecracker,
    NativeSandbox,
}

impl ConfinementType {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wasm => "WASM",
            Self::Docker => "DOCKER",
            Self::Gvisor => "GVISOR",
            Self::Firecracker => "FIRECRACKER",
            Self::NativeSandbox => "NATIVE-SANDBOX",
        }
    }
}

/// A request to confine one execution to an exact sandbox descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfinementRequest {
    pub host_id: String,
    pub execution_id: String,
    pub confinement_type: ConfinementType,
    pub sandbox_digest: String,
}

/// The attestation that an execution runs within an admitted sandbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfinementAttestation {
    pub attestation_id: String,
    pub host_id: String,
    pub execution_id: String,
    pub confinement_type: ConfinementType,
    pub sandbox_digest: String,
    pub attested_at: String,
    pub signature: String,
}

/// Why confinement was refused. Closed set; there is no degraded fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfinementError {
    UnadmittedSandbox {
        confinement_type: &'static str,
        sandbox_digest: String,
    },
    MalformedSandboxDigest,
}

impl std::fmt::Display for ConfinementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnadmittedSandbox {
                confinement_type,
                sandbox_digest,
            } => write!(f, "no admitted {confinement_type} sandbox matches {sandbox_digest}"),
            Self::MalformedSandboxDigest => write!(f, "malformed sandbox digest"),
        }
    }
}

impl std::error::Error for ConfinementError {}

/// The Confinement port: attest that an execution is confined to an exact
/// admitted sandbox, or fail closed.
#[async_trait]
pub trait ConfinementPort: Send + Sync {
    async fn attest(
        &self,
        request: ConfinementRequest,
    ) -> Result<ConfinementAttestation, ConfinementError>;
}
