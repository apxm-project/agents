use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptMode {
    #[serde(rename = "confirm")]
    Confirm,
    #[serde(rename = "dual_control")]
    DualControl,
    #[serde(rename = "external_signoff")]
    ExternalSignoff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "critical")]
    Critical,
}

/// A per-call consent request sent to the host's prompt surface.
///
/// The consent broker must receive this and return signed approvals within
/// the timeout before the operation proceeds. Fail-closed: timeout → deny.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPrompt {
    /// Stable identifier for this specific consent request.
    pub prompt_id: String,
    /// Stable tool-call identifier this prompt guards.
    pub call_id: String,
    /// Runtime grant that admitted the call before consent was required.
    pub grant_id: String,
    /// The capability being requested.
    pub capability_id: String,
    /// Callable implementation binding checked by the runtime gate.
    pub capability_binding: String,
    /// The host that owns the capability, when the prompt is host-scoped.
    pub host_id: Option<String>,
    /// Operation being attempted under the capability.
    pub operation: String,
    /// Prompt policy selected for this call.
    pub mode: PromptMode,
    /// Subject bound to this session (None for owner-only calls).
    pub subject: Option<serde_json::Value>,
    /// Deterministic digest of the call args (SHA-256 hex or simplified stub).
    pub args_digest: String,
    /// Human-visible preview of the arguments being approved.
    pub args_preview: serde_json::Value,
    /// Risk level selected by policy.
    pub risk_level: RiskLevel,
    /// RFC3339 timestamp after which this prompt expires.
    pub expires_at: String,
    /// ACP channel id for relay-connected hosts (None for DIRECT hosts).
    pub channel_id: Option<String>,
    /// Human-readable description shown on the approval surface.
    pub description: Option<String>,
    /// Optional stable target reference shown on the approval surface.
    pub target_ref: Option<String>,
    /// Optional structured resource metadata shown on the approval surface.
    pub resource: Option<serde_json::Value>,
    /// Optional diff/blob reference for high-risk changes.
    pub diff_ref: Option<serde_json::Value>,
}

/// Signed approval returned by the consent broker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedApproval {
    pub signer_subject: String,
    pub signer_display: Option<String>,
    /// Ed25519 signature over canonical apxm.prompt-approval.v1.
    pub signature: String,
    /// RFC3339 timestamp when the approval was issued.
    pub signed_at: String,
}

/// Result of a consent check.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsentDecision {
    /// Approved with a signed approval record.
    Approved(Vec<SignedApproval>),
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
            call_id: "c1".into(),
            grant_id: "g1".into(),
            capability_id: "provider.write".into(),
            capability_binding: "provider.write".into(),
            host_id: Some("host-1".into()),
            operation: "write".into(),
            mode: PromptMode::Confirm,
            subject: None,
            args_digest: "sha256-stub-3".into(),
            args_preview: serde_json::json!({}),
            risk_level: RiskLevel::High,
            expires_at: "2026-06-30T00:00:00Z".into(),
            channel_id: None,
            description: None,
            target_ref: None,
            resource: None,
            diff_ref: None,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let decision = rt.block_on(broker.request_consent(prompt, Duration::from_secs(5)));
        assert_eq!(decision, ConsentDecision::NoBroker);
    }
}
