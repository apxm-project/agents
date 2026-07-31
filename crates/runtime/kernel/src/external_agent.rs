//! The External Agent Capability port and nested-evidence assembly.
//!
//! One External Agent prompt is exactly one outer `capability.invoke` effect
//! even when the peer runs many private model/tool cycles. Those cycles are
//! recorded as ordered attributed nested events under the one Capability
//! NodeExecution — never native model NodeExecutions or authored loop
//! occurrences. Peer usage is third-party provenance preserved verbatim; this
//! module offers no path from peer usage into native model accounting.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use apxm_program::external_agent::{
    AttributedEvent, ExternalAgentEvidence, ExternalAgentEvidenceVersion, PeerUsage,
};

/// The terminal state of the one outer effect an External Agent prompt produces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptEffectState {
    Completed {
        stop_reason: Option<String>,
    },
    Cancelled,
    Failed {
        message: String,
    },
    /// An ambiguous transport failure the adapter cannot resolve truthfully.
    OutcomeUnknown {
        message: String,
    },
}

/// A request to run one prompt against an admitted External Agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcpPromptRequest {
    pub effect_ref: String,
    pub session_ref: String,
    pub profile_ref: String,
    pub prompt: String,
}

/// The single outer Capability effect for one External Agent prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcpPromptOutcome {
    pub session_ref: String,
    pub state: PromptEffectState,
    pub nested_events: Vec<AttributedEvent>,
    pub peer_usage: Vec<PeerUsage>,
}

/// The External Agent Capability port. One `prompt` call is one outer
/// `capability.invoke` effect. Adapters implement it; the runtime never spawns a
/// peer process directly.
#[async_trait]
pub trait ExternalAgentCapabilityPort: Send + Sync {
    async fn prompt(&self, request: AcpPromptRequest) -> AcpPromptOutcome;
}

/// Assemble the nested attributed evidence for one Capability NodeExecution.
/// Peer usage is copied verbatim into `peer_usage` provenance; it is never
/// summed, parsed, or mapped onto a native model usage fact.
#[must_use]
pub fn assemble_evidence(
    capability_node_execution_id: impl Into<String>,
    outcome: &AcpPromptOutcome,
) -> ExternalAgentEvidence {
    ExternalAgentEvidence {
        schema_version: ExternalAgentEvidenceVersion::V1,
        capability_node_execution_id: capability_node_execution_id.into(),
        session_ref: outcome.session_ref.clone(),
        attributed_events: outcome.nested_events.clone(),
        peer_usage: outcome.peer_usage.clone(),
    }
}
