use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Stable denial reason returned when no approval surface is configured.
pub const APPROVAL_BROKER_UNAVAILABLE_REASON: &str = "approval_broker_unavailable";

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

/// A per-call approval request sent to the host's prompt surface.
///
/// Runtime admission waits for signed approvals within the timeout before the
/// operation proceeds. Fail-closed: timeout → deny.
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

/// Signed approval returned by an approval surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedApproval {
    pub signer_subject: String,
    pub signer_display: Option<String>,
    /// Ed25519 signature over canonical apxm.prompt-approval.
    pub signature: String,
    /// RFC3339 timestamp when the approval was issued.
    pub signed_at: String,
}

/// Closed public resolution vocabulary for approval lifecycle events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalResolution {
    Approved,
    Denied,
    Expired,
}

impl ApprovalResolution {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Expired => "expired",
        }
    }
}

/// Non-empty set of approvals whose signatures were verified by the broker.
#[derive(Debug, Clone, PartialEq)]
pub struct SignedApprovalSet {
    approvals: Vec<SignedApproval>,
}

impl SignedApprovalSet {
    pub fn new(first: SignedApproval, additional: Vec<SignedApproval>) -> Self {
        let mut approvals = Vec::with_capacity(1 + additional.len());
        approvals.push(first);
        approvals.extend(additional);
        Self { approvals }
    }

    pub fn as_slice(&self) -> &[SignedApproval] {
        &self.approvals
    }
}

/// Evidence recorded for an approval made through an authenticated interactive
/// surface that does not produce a cryptographic signature.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractiveApproval {
    pub decided_at: String,
    pub responder_subject: Option<String>,
}

/// Trusted evidence attached to an approved consent decision.
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalEvidence {
    Signed(SignedApprovalSet),
    Interactive(InteractiveApproval),
}

/// Result of a consent check.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsentDecision {
    /// Approved with evidence from the surface that made the decision.
    Approved { evidence: ApprovalEvidence },
    /// Denied (host or user explicitly rejected).
    Denied { reason: String },
    /// Approval not received within the timeout window. Fail-closed → deny.
    Expired,
}

impl ConsentDecision {
    pub const fn resolution(&self) -> ApprovalResolution {
        match self {
            Self::Approved { .. } => ApprovalResolution::Approved,
            Self::Denied { .. } => ApprovalResolution::Denied,
            Self::Expired => ApprovalResolution::Expired,
        }
    }
}

/// Async interface for per-call approval routing.
#[async_trait::async_trait]
pub trait ConsentBroker: Send + Sync + 'static {
    async fn request_consent(&self, prompt: PermissionPrompt, timeout: Duration)
    -> ConsentDecision;
}

/// Approval router used when no approval surface is configured.
pub struct UnavailableConsentBroker;

#[async_trait::async_trait]
impl ConsentBroker for UnavailableConsentBroker {
    async fn request_consent(
        &self,
        _prompt: PermissionPrompt,
        _timeout: Duration,
    ) -> ConsentDecision {
        ConsentDecision::Denied {
            reason: APPROVAL_BROKER_UNAVAILABLE_REASON.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_broker_returns_typed_denial() {
        let broker = UnavailableConsentBroker;
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
        assert_eq!(
            decision,
            ConsentDecision::Denied {
                reason: APPROVAL_BROKER_UNAVAILABLE_REASON.to_string()
            }
        );
    }
}
