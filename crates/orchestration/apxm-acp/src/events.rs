use apxm_core::events::{EventCategory, EventKind};
use apxm_core::impl_event_payload;
use serde::{Deserialize, Serialize};

pub const ACP_SESSION_SPAWNED: EventKind =
    EventKind::new("acp_session_spawned", EventCategory::Lifecycle, false);
pub const ACP_SESSION_PROMPT_START: EventKind =
    EventKind::new("acp_session_prompt_start", EventCategory::Lifecycle, false);
pub const ACP_SESSION_CHUNK: EventKind =
    EventKind::new("acp_session_chunk", EventCategory::Stream, false);
pub const ACP_SESSION_PROMPT_END: EventKind =
    EventKind::new("acp_session_prompt_end", EventCategory::Lifecycle, true);
pub const ACP_SESSION_CLOSED: EventKind =
    EventKind::new("acp_session_closed", EventCategory::Lifecycle, true);
pub const ACP_REVERSE_REQUEST: EventKind =
    EventKind::new("acp_reverse_request", EventCategory::Lifecycle, false);
pub const ACP_SESSION_ERROR: EventKind =
    EventKind::new("acp_session_error", EventCategory::Error, true);
pub const ACP_TOOL_RESULT: EventKind =
    EventKind::new("acp_tool_result", EventCategory::Lifecycle, false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSessionSpawnedPayload {
    pub agent: String,
    pub session_id: String,
    pub cwd: String,
}
impl_event_payload!(AcpSessionSpawnedPayload, ACP_SESSION_SPAWNED);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSessionPromptStartPayload {
    pub session_id: String,
    pub prompt_len: usize,
}
impl_event_payload!(AcpSessionPromptStartPayload, ACP_SESSION_PROMPT_START);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSessionChunkPayload {
    pub session_id: String,
    pub chunk_len: usize,
    pub total_len: usize,
}
impl_event_payload!(AcpSessionChunkPayload, ACP_SESSION_CHUNK);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSessionPromptEndPayload {
    pub session_id: String,
    pub stop_reason: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
impl_event_payload!(AcpSessionPromptEndPayload, ACP_SESSION_PROMPT_END);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSessionClosedPayload {
    pub session_id: String,
    pub total_turns: u32,
    pub close_reason: String,
}
impl_event_payload!(AcpSessionClosedPayload, ACP_SESSION_CLOSED);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpReverseRequestPayload {
    pub session_id: String,
    pub method: String,
    pub approved: bool,
}
impl_event_payload!(AcpReverseRequestPayload, ACP_REVERSE_REQUEST);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpSessionErrorPayload {
    pub session_id: String,
    pub error: String,
}
impl_event_payload!(AcpSessionErrorPayload, ACP_SESSION_ERROR);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpToolResultPayload {
    pub id: String,
    pub success: bool,
    pub output: String,
}
impl_event_payload!(AcpToolResultPayload, ACP_TOOL_RESULT);
