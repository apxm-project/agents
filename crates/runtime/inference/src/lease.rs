//! Short-lived target- and purpose-bound inference credential leases.
//!
//! Secret material is redeemed only into adapter memory for one exact purpose
//! and target binding. It is never cloned, logged, serialized, checkpointed, or
//! retained after the lease scope ends. APXM does not own secret custody.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::identity::ModelTargetRef;
use apxm_program::grammar::is_digest;

/// Schema identity for the public lease identity contract.
pub const INFERENCE_CREDENTIAL_LEASE_SCHEMA: &str = "apxm.inference-credential-lease.v1";

/// Closed lease purpose vocabulary for inference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeasePurpose {
    ModelInference,
}

/// Public lease identity. Contains no secret material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceCredentialLeaseIdentity {
    pub schema_version: String,
    pub lease_id: String,
    pub purpose: LeasePurpose,
    pub model_target_ref: String,
    pub exact_port_binding_digest: String,
    pub expires_at_unix_ms: u64,
    pub lease_digest: String,
}

/// Why a lease cannot be redeemed. Every variant fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseError {
    EmptyField(&'static str),
    InvalidDigest(&'static str),
    PurposeMismatch,
    TargetMismatch {
        expected: String,
        actual: String,
    },
    BindingMismatch,
    Expired {
        now_unix_ms: u64,
        expires_at_unix_ms: u64,
    },
    DigestMismatch,
    AlreadyActivated,
    Revoked,
    NotActivated,
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "lease field {field} is empty"),
            Self::InvalidDigest(field) => write!(f, "lease field {field} is not a sha256 digest"),
            Self::PurposeMismatch => write!(f, "lease purpose is not model_inference"),
            Self::TargetMismatch { expected, actual } => {
                write!(f, "lease target {actual} does not match {expected}")
            }
            Self::BindingMismatch => {
                write!(f, "lease binding digest does not match admitted binding")
            }
            Self::Expired {
                now_unix_ms,
                expires_at_unix_ms,
            } => write!(
                f,
                "lease expired at {expires_at_unix_ms}; observed {now_unix_ms}"
            ),
            Self::DigestMismatch => write!(f, "lease digest does not match identity"),
            Self::AlreadyActivated => {
                write!(f, "lease was already activated for one adapter scope")
            }
            Self::Revoked => write!(f, "lease was revoked"),
            Self::NotActivated => write!(f, "lease material was not activated for this scope"),
        }
    }
}

impl std::error::Error for LeaseError {}

impl InferenceCredentialLeaseIdentity {
    /// Build one public lease identity and its content digest over non-secret fields.
    pub fn mint(
        lease_id: impl Into<String>,
        model_target_ref: impl Into<String>,
        exact_port_binding_digest: impl Into<String>,
        expires_at_unix_ms: u64,
    ) -> Result<Self, LeaseError> {
        let lease_id = lease_id.into();
        let model_target_ref = model_target_ref.into();
        let exact_port_binding_digest = exact_port_binding_digest.into();
        if lease_id.trim().is_empty() {
            return Err(LeaseError::EmptyField("lease_id"));
        }
        if model_target_ref.trim().is_empty() {
            return Err(LeaseError::EmptyField("model_target_ref"));
        }
        if !is_digest(&exact_port_binding_digest) {
            return Err(LeaseError::InvalidDigest("exact_port_binding_digest"));
        }
        let lease_digest = digest_identity(
            &lease_id,
            &model_target_ref,
            &exact_port_binding_digest,
            expires_at_unix_ms,
        );
        Ok(Self {
            schema_version: INFERENCE_CREDENTIAL_LEASE_SCHEMA.to_string(),
            lease_id,
            purpose: LeasePurpose::ModelInference,
            model_target_ref,
            exact_port_binding_digest,
            expires_at_unix_ms,
            lease_digest,
        })
    }

    /// Validate public identity shape and digest membership.
    pub fn validate(&self) -> Result<(), LeaseError> {
        if self.schema_version != INFERENCE_CREDENTIAL_LEASE_SCHEMA {
            return Err(LeaseError::EmptyField("schema_version"));
        }
        if self.lease_id.trim().is_empty() {
            return Err(LeaseError::EmptyField("lease_id"));
        }
        if !matches!(self.purpose, LeasePurpose::ModelInference) {
            return Err(LeaseError::PurposeMismatch);
        }
        if self.model_target_ref.trim().is_empty() {
            return Err(LeaseError::EmptyField("model_target_ref"));
        }
        if !is_digest(&self.exact_port_binding_digest) {
            return Err(LeaseError::InvalidDigest("exact_port_binding_digest"));
        }
        if !is_digest(&self.lease_digest) {
            return Err(LeaseError::InvalidDigest("lease_digest"));
        }
        let expected = digest_identity(
            &self.lease_id,
            &self.model_target_ref,
            &self.exact_port_binding_digest,
            self.expires_at_unix_ms,
        );
        if expected != self.lease_digest {
            return Err(LeaseError::DigestMismatch);
        }
        Ok(())
    }
}

/// Opaque credential material held only in adapter memory.
///
/// Intentionally implements neither `Clone`, `Debug`, nor `Serialize`. Dropping
/// the value ends the lease scope and discards material.
pub struct InferenceCredentialLease {
    identity: InferenceCredentialLeaseIdentity,
    material: String,
    activated: bool,
    revoked: bool,
}

impl InferenceCredentialLease {
    /// Pair public identity with secret material. Material is never part of the
    /// public contract.
    pub fn issue(
        identity: InferenceCredentialLeaseIdentity,
        material: impl Into<String>,
    ) -> Result<Self, LeaseError> {
        identity.validate()?;
        let material = material.into();
        if material.is_empty() {
            return Err(LeaseError::EmptyField("material"));
        }
        Ok(Self {
            identity,
            material,
            activated: false,
            revoked: false,
        })
    }

    #[must_use]
    pub fn identity(&self) -> &InferenceCredentialLeaseIdentity {
        &self.identity
    }

    /// Activate the lease for one exact target, binding, and purpose while it is
    /// unexpired. Activation may happen once; retries within the same adapter
    /// scope reuse the in-memory material through [`expose`].
    pub fn activate(
        &mut self,
        authored_target: &ModelTargetRef,
        exact_port_binding_digest: &str,
        now_unix_ms: u64,
    ) -> Result<(), LeaseError> {
        self.identity.validate()?;
        if self.revoked {
            return Err(LeaseError::Revoked);
        }
        if self.activated {
            return Err(LeaseError::AlreadyActivated);
        }
        if self.identity.model_target_ref != authored_target.0 {
            return Err(LeaseError::TargetMismatch {
                expected: authored_target.0.clone(),
                actual: self.identity.model_target_ref.clone(),
            });
        }
        if self.identity.exact_port_binding_digest != exact_port_binding_digest {
            return Err(LeaseError::BindingMismatch);
        }
        if now_unix_ms >= self.identity.expires_at_unix_ms {
            return Err(LeaseError::Expired {
                now_unix_ms,
                expires_at_unix_ms: self.identity.expires_at_unix_ms,
            });
        }
        self.activated = true;
        Ok(())
    }

    /// Borrow activated material from adapter memory. Fails if the lease was
    /// never activated for this scope.
    pub fn expose(&self) -> Result<&str, LeaseError> {
        if self.revoked {
            return Err(LeaseError::Revoked);
        }
        if !self.activated {
            return Err(LeaseError::NotActivated);
        }
        Ok(self.material.as_str())
    }

    /// Revoke this lease and discard the adapter-memory material immediately.
    pub fn revoke(&mut self) {
        self.revoked = true;
        self.material.clear();
        self.activated = false;
    }

    #[must_use]
    pub fn is_revoked(&self) -> bool {
        self.revoked
    }
}

fn digest_identity(
    lease_id: &str,
    model_target_ref: &str,
    exact_port_binding_digest: &str,
    expires_at_unix_ms: u64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(INFERENCE_CREDENTIAL_LEASE_SCHEMA.as_bytes());
    hasher.update(b"\0");
    hasher.update(lease_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(b"model_inference");
    hasher.update(b"\0");
    hasher.update(model_target_ref.as_bytes());
    hasher.update(b"\0");
    hasher.update(exact_port_binding_digest.as_bytes());
    hasher.update(b"\0");
    hasher.update(expires_at_unix_ms.to_string().as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Redact any candidate diagnostic/log payload. Prompts, completions,
/// credentials, and private payloads are replaced with a stable token.
#[must_use]
pub fn redact_diagnostic_value(key: &str, value: &str) -> String {
    let key_l = key.to_ascii_lowercase();
    let forbidden = [
        "prompt",
        "completion",
        "credential",
        "secret",
        "token",
        "authorization",
        "lease_material",
        "private",
        "password",
        "api_key",
    ];
    if forbidden.iter().any(|needle| key_l.contains(needle)) {
        return "[redacted]".to_string();
    }
    if value.len() > 240 {
        return "[redacted:bounded]".to_string();
    }
    value.to_string()
}
