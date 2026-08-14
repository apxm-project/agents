//! `apxm.external-agent-session` and `apxm.external-agent-evidence` —
//! closed consumer types and verification.
//!
//! An External Agent connection is reached over the agent client protocol and
//! exposed only as an opaque scoped session reference. One prompt effect is one
//! outer `capability.invoke` NodeExecution; the peer's private model and tool
//! cycles are recorded as ordered attributed nested evidence under that one
//! NodeExecution, never as native model NodeExecutions or authored loop
//! occurrences. Peer usage is third-party provenance preserved verbatim and
//! never enters native model accounting.

use serde::{Deserialize, Serialize};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::grammar::is_identifier;

/// The single accepted `schema_version` for an External Agent session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalAgentSessionVersion {
    #[serde(rename = "apxm.external-agent-session")]
    V1,
}

/// The closed connection lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycle {
    Uninitialized,
    Initialized,
    Authenticated,
    SessionOpen,
    Closed,
}

/// The closed terminal outcome of the one outer prompt effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutcome {
    Completed,
    Cancelled,
    Failed,
    OutcomeUnknown,
}

/// A decoded External Agent session reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAgentSession {
    pub schema_version: ExternalAgentSessionVersion,
    pub session_ref: String,
    pub profile_ref: String,
    pub lifecycle: SessionLifecycle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation_requested: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_outcome: Option<TerminalOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

impl ExternalAgentSession {
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();
        if !is_identifier(&self.session_ref) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::InvalidIdentifier,
                self.session_ref.clone(),
                "session_ref is not a contract identifier",
            ));
        }
        if !is_identifier(&self.profile_ref) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::InvalidIdentifier,
                self.profile_ref.clone(),
                "profile_ref is not a contract identifier",
            ));
        }
        verdict.finish()
    }
}

/// The single accepted `schema_version` for External Agent evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalAgentEvidenceVersion {
    #[serde(rename = "apxm.external-agent-evidence")]
    V1,
}

/// The closed attributed-event kinds observed during one prompt effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributedEventKind {
    Message,
    Plan,
    ToolCall,
    ReverseRequest,
}

/// The closed reverse-request authorization decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReverseDecision {
    Permit,
    DenyMissingAuthority,
    DenyCeilingExceeded,
    DenyPolicy,
}

/// One ordered attributed nested event. Reverse fields belong only to a
/// `reverse_request`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributedEvent {
    pub event_sequence: u64,
    pub kind: AttributedEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_decision: Option<ReverseDecision>,
}

/// One peer-reported usage measurement, attributed to the peer. The value is an
/// opaque string preserved verbatim; there is no native token field and no path
/// into native model accounting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerUsage {
    pub reported_by: String,
    pub metric_scope: String,
    pub reported_value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_partial: Option<bool>,
}

/// A decoded External Agent evidence record for one capability NodeExecution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAgentEvidence {
    pub schema_version: ExternalAgentEvidenceVersion,
    pub capability_node_execution_id: String,
    pub session_ref: String,
    pub attributed_events: Vec<AttributedEvent>,
    pub peer_usage: Vec<PeerUsage>,
}

impl ExternalAgentEvidence {
    /// Verify: events are strictly monotonic, reverse fields appear only on a
    /// `reverse_request` (and completely), and identifiers are well formed.
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();

        if !is_identifier(&self.capability_node_execution_id) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::InvalidIdentifier,
                self.capability_node_execution_id.clone(),
                "capability_node_execution_id is not a contract identifier",
            ));
        }

        let mut previous: Option<u64> = None;
        for event in &self.attributed_events {
            if let Some(prev) = previous
                && event.event_sequence <= prev
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::NonMonotonicEvidence,
                    event.event_sequence.to_string(),
                    "attributed event_sequence is not strictly increasing",
                ));
            }
            previous = Some(event.event_sequence);

            let has_reverse = event.reverse_operation.is_some()
                || event.reverse_target.is_some()
                || event.reverse_decision.is_some();
            match event.kind {
                AttributedEventKind::ReverseRequest => {
                    let complete = event.reverse_operation.is_some()
                        && event.reverse_target.is_some()
                        && event.reverse_decision.is_some();
                    if !complete {
                        verdict.push(Diagnostic::new(
                            DiagnosticCode::AttributedEventInconsistent,
                            event.event_sequence.to_string(),
                            "a reverse_request records operation, target, and decision",
                        ));
                    }
                }
                _ if has_reverse => {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::AttributedEventInconsistent,
                        event.event_sequence.to_string(),
                        "reverse fields belong only to a reverse_request",
                    ));
                }
                _ => {}
            }
        }

        for usage in &self.peer_usage {
            if !is_identifier(&usage.reported_by) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::InvalidIdentifier,
                    usage.reported_by.clone(),
                    "peer usage reported_by is not a contract identifier",
                ));
            }
        }

        verdict.finish()
    }
}

/// Verify an External Agent session presented as JSON, failing closed on decode.
#[must_use]
pub fn verify_external_agent_session_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<ExternalAgentSession>(value.clone()) {
        Ok(session) => session.verify(),
        Err(error) => schema_violation("external_agent_session", &error),
    }
}

/// Verify External Agent evidence presented as JSON, failing closed on decode.
#[must_use]
pub fn verify_external_agent_evidence_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<ExternalAgentEvidence>(value.clone()) {
        Ok(evidence) => evidence.verify(),
        Err(error) => schema_violation("external_agent_evidence", &error),
    }
}
