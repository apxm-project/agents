use apxm_core::events::kind;
use apxm_core::events::{EventCategory, EventKind};
use apxm_core::impl_event_payload;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TOOL_RESULT: EventKind = EventKind::new("tool_result", EventCategory::Lifecycle, false);
pub const DONE: EventKind = EventKind::new("done", EventCategory::Lifecycle, true);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTokenPayload {
    pub token: String,
}
impl_event_payload!(AgentTokenPayload, kind::TOKEN);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolCallPayload {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
impl_event_payload!(AgentToolCallPayload, kind::TOOL_CALL);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolResultPayload {
    pub id: String,
    pub success: bool,
    pub output: String,
}
impl_event_payload!(AgentToolResultPayload, TOOL_RESULT);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUsagePayload {
    #[serde(rename = "inputTokens")]
    pub input_tokens: u64,
    #[serde(rename = "outputTokens")]
    pub output_tokens: u64,
}
impl_event_payload!(AgentUsagePayload, kind::USAGE);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDonePayload {
    #[serde(rename = "stopReason")]
    pub stop_reason: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
}
impl_event_payload!(AgentDonePayload, DONE);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentErrorPayload {
    pub error: String,
}
impl_event_payload!(AgentErrorPayload, kind::ERROR);
