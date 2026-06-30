//! APXM Principal and host identity types.
//!
//! A Principal is derived ONLY from a verified Host Certificate.
//! It is never self-asserted. owner= input parameters are removed as inputs.

use serde::{Deserialize, Serialize};

use crate::types::host::HostTier;

/// Attested principal derived from a verified Host Certificate.
///
/// Never constructed from user input — only from certificate verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPrincipal {
    /// Operator who enrolled the host (from the Host Certificate).
    pub owner: String,
    /// Stable host identifier (from the Host Certificate).
    pub host_id: String,
    /// For subject-bound hosts: the verified end-user subject.
    /// Set only from a verified SubjectDelegationJwt, never from input.
    pub subject: Option<String>,
    /// Execution tier ceiling from the Host Certificate.
    pub tier: HostTier,
    /// Scope ceiling intersection from the Host Certificate.
    pub scopes: Vec<String>,
}

impl HostPrincipal {
    pub fn new(
        owner: impl Into<String>,
        host_id: impl Into<String>,
        tier: HostTier,
        scopes: Vec<String>,
    ) -> Self {
        Self {
            owner: owner.into(),
            host_id: host_id.into(),
            subject: None,
            tier,
            scopes,
        }
    }

    /// Bind a verified end-user subject to this principal.
    /// Only called after subject_proof_jwt is verified by auth middleware.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Returns true if this principal is acting for a specific end-user.
    pub fn is_subject_bound(&self) -> bool {
        self.subject.is_some()
    }

    /// Returns true if this principal has the requested scope.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope || s == "*")
    }
}

/// Host Certificate issued at enrollment.
///
/// Signed by the enrollment CA Ed25519 key.
/// The host's Ed25519 private key never leaves the host keychain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCertificate {
    pub host_id: String,
    pub owner: String,
    /// Ed25519 public key, base64url encoded.
    pub pubkey: String,
    /// Maximum tier this host may operate at.
    pub tier_ceiling: HostTier,
    /// Maximum scope set this host may request.
    pub scopes_ceiling: Vec<String>,
    pub not_before: i64,
    /// Max 90 days from enrollment (Unix timestamp).
    pub not_after: i64,
}

impl HostCertificate {
    pub const MAX_VALIDITY_SECS: i64 = 90 * 24 * 3600; // 90 days

    pub fn is_expired(&self, now_unix: i64) -> bool {
        now_unix > self.not_after || now_unix < self.not_before
    }
}

/// Subject delegation proof for subject-bound hosts.
///
/// Issued by auth at end-user login. Binds {host_id, subject}.
/// Verified by identity middleware before setting HostPrincipal.subject.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectDelegation {
    pub host_id: String,
    /// Verified end-user subject (e.g. email or opaque subject ID).
    pub subject: String,
    /// Short-lived JWT issued and signed by the enrollment CA key.
    pub subject_proof_jwt: String,
}

/// Proof-of-possession token envelope for host credential requests.
///
/// APXM-minted JWT; audience = host_id, cnf = host-cert pubkey thumbprint.
/// Short TTL (60s max). Never raw secret material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenEnvelope {
    pub host_id: String,
    pub jwt: String,
    pub expires_at: i64,
    /// Intersection of connection scopes, cert ceiling, and grant.
    pub scopes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_principal_has_scope() {
        let p = HostPrincipal::new(
            "owner",
            "host1",
            HostTier::LinkTools,
            vec!["read".into(), "write".into()],
        );
        assert!(p.has_scope("read"));
        assert!(p.has_scope("write"));
        assert!(!p.has_scope("admin"));
    }

    #[test]
    fn host_principal_wildcard_scope() {
        let p = HostPrincipal::new("owner", "host1", HostTier::Direct, vec!["*".into()]);
        assert!(p.has_scope("anything"));
    }

    #[test]
    fn host_principal_subject_bound() {
        let p = HostPrincipal::new("owner", "host1", HostTier::LinkTools, vec![])
            .with_subject("user@example.com");
        assert!(p.is_subject_bound());
        assert_eq!(p.subject.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn host_cert_expiry() {
        let cert = HostCertificate {
            host_id: "h1".into(),
            owner: "o1".into(),
            pubkey: "deadbeef".into(),
            tier_ceiling: HostTier::LinkTools,
            scopes_ceiling: vec![],
            not_before: 1000,
            not_after: 2000,
        };
        assert!(!cert.is_expired(1500));
        assert!(cert.is_expired(2001));
        assert!(cert.is_expired(999));
    }
}
