use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A per-call consent request sent to the host's prompt surface.
///
/// The consent broker must receive this and return a SignedApproval within
/// the timeout before the operation proceeds. Fail-closed: timeout → deny.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPrompt {
    /// Stable identifier for this specific consent request.
    pub prompt_id: String,
    /// The capability being requested.
    pub capability_id: String,
    /// The host that owns the capability.
    pub host_id: String,
    /// Subject bound to this session (None for owner-only calls).
    pub subject: Option<String>,
    /// Deterministic digest of the call args (SHA-256 hex or simplified stub).
    pub args_digest: String,
    /// Unix timestamp (seconds) after which this prompt expires.
    pub expires_at: i64,
    /// ACP channel id for relay-connected hosts (None for DIRECT hosts).
    pub channel_id: Option<String>,
    /// Human-readable description shown on the approval surface.
    pub description: Option<String>,
}

/// Signed approval returned by the consent broker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedApproval {
    pub prompt_id: String,
    pub approved: bool,
    pub approver: String,
    /// Unix timestamp when the approval was issued.
    pub issued_at: i64,
    /// Ed25519 signature over (prompt_id || approved || issued_at || scope_hash).
    pub signature: String,
    /// Hash of the capability scope granted by this approval.
    pub scope_hash: Option<String>,
}

/// Result of a consent check.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsentDecision {
    /// Approved with a signed approval record.
    Approved(SignedApproval),
    /// Denied (host or user explicitly rejected).
    Denied { reason: String },
    /// Approval not received within the timeout window. Fail-closed → deny.
    TimedOut,
    /// No consent broker is configured. Caller decides whether to allow.
    NoBroker,
}

/// Async interface for per-call consent routing.
///
/// The `os` repo implements the real broker (routes to host prompt surface or
/// Studio approval UI). Tests and non-host contexts use `NoOpConsentBroker`.
#[async_trait::async_trait]
pub trait ConsentBroker: Send + Sync + 'static {
    async fn request_consent(&self, prompt: PermissionPrompt, timeout: Duration)
    -> ConsentDecision;
}

/// Broker that immediately returns NoBroker for every request.
///
/// Used when no host prompt surface is configured. Callers that treat NoBroker
/// as allow must be explicit about that decision in their enforcement logic.
pub struct NoOpConsentBroker;

#[async_trait::async_trait]
impl ConsentBroker for NoOpConsentBroker {
    async fn request_consent(
        &self,
        _prompt: PermissionPrompt,
        _timeout: Duration,
    ) -> ConsentDecision {
        ConsentDecision::NoBroker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_op_broker_returns_no_broker() {
        let broker = NoOpConsentBroker;
        let prompt = PermissionPrompt {
            prompt_id: "p1".into(),
            capability_id: "provider.write".into(),
            host_id: "host-1".into(),
            subject: None,
            args_digest: "sha256-stub-3".into(),
            expires_at: 9999999999,
            channel_id: None,
            description: None,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let decision = rt.block_on(broker.request_consent(prompt, Duration::from_secs(5)));
        assert_eq!(decision, ConsentDecision::NoBroker);
    }
}
