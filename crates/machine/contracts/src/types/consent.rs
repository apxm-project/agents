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

/// Policy risk vocabulary carried by a [`PermissionPrompt`].
///
/// This is the wider of the two risk vocabularies in the tree. The narrower
/// [`ApprovalRiskLevel`](crate::events::payload::ApprovalRiskLevel) is the
/// projection published on the approval-request event, whose variant list is
/// owned by the contracts repository and pinned by the generated public
/// clients; widening it has to land there first. Project across the boundary
/// with [`RiskLevel::to_approval`] rather than restating the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    /// Wire-shape (`snake_case`) string for this variant, so serde and
    /// non-serde contexts cannot disagree.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    /// Narrow this policy risk onto the event vocabulary. `Critical` saturates
    /// to `High` because the published event schema admits no fourth level.
    pub const fn to_approval(self) -> crate::events::payload::ApprovalRiskLevel {
        use crate::events::payload::ApprovalRiskLevel;
        match self {
            Self::Low => ApprovalRiskLevel::Low,
            Self::Medium => ApprovalRiskLevel::Medium,
            Self::High | Self::Critical => ApprovalRiskLevel::High,
        }
    }
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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

    /// The policy vocabulary is one level wider than the published event
    /// vocabulary, so the projection has to be total and `critical` has to
    /// land somewhere. It saturates to `high` rather than dropping the prompt.
    #[test]
    fn risk_level_projects_onto_the_published_event_vocabulary() {
        use crate::events::payload::ApprovalRiskLevel;

        for (policy, expected) in [
            (RiskLevel::Low, ApprovalRiskLevel::Low),
            (RiskLevel::Medium, ApprovalRiskLevel::Medium),
            (RiskLevel::High, ApprovalRiskLevel::High),
            (RiskLevel::Critical, ApprovalRiskLevel::High),
        ] {
            assert_eq!(policy.to_approval(), expected, "projecting {policy}");
        }
    }

    /// `as_str` and serde must not disagree; the label and the wire value are
    /// the same vocabulary.
    #[test]
    fn risk_level_as_str_matches_its_serde_wire_value() {
        for level in [
            RiskLevel::Low,
            RiskLevel::Medium,
            RiskLevel::High,
            RiskLevel::Critical,
        ] {
            assert_eq!(
                serde_json::to_value(level).expect("serialize risk level"),
                serde_json::Value::String(level.as_str().to_string())
            );
        }
    }
}
