use apxm_core::events::{EventCategory, EventKind, EventPayloadRegistry};
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

/// Build a payload decoder registry for ACP extension event kinds.
pub fn event_payload_registry() -> EventPayloadRegistry {
    let mut registry = EventPayloadRegistry::new();
    registry
        .register_payload::<AcpSessionSpawnedPayload>(ACP_SESSION_SPAWNED)
        .expect("ACP event kind should be a unique extension");
    registry
        .register_payload::<AcpSessionPromptStartPayload>(ACP_SESSION_PROMPT_START)
        .expect("ACP event kind should be a unique extension");
    registry
        .register_payload::<AcpSessionChunkPayload>(ACP_SESSION_CHUNK)
        .expect("ACP event kind should be a unique extension");
    registry
        .register_payload::<AcpSessionPromptEndPayload>(ACP_SESSION_PROMPT_END)
        .expect("ACP event kind should be a unique extension");
    registry
        .register_payload::<AcpSessionClosedPayload>(ACP_SESSION_CLOSED)
        .expect("ACP event kind should be a unique extension");
    registry
        .register_payload::<AcpReverseRequestPayload>(ACP_REVERSE_REQUEST)
        .expect("ACP event kind should be a unique extension");
    registry
        .register_payload::<AcpSessionErrorPayload>(ACP_SESSION_ERROR)
        .expect("ACP event kind should be a unique extension");
    registry
        .register_payload::<AcpToolResultPayload>(ACP_TOOL_RESULT)
        .expect("ACP event kind should be a unique extension");
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::events::payload::{UnknownEventPayload, WarningPayload};
    use apxm_core::events::{ApxmEvent, EventSource};

    const UNKNOWN_ACP_KIND: &str = "acp_unregistered_extension";

    fn raw_event_with_payload(payload: serde_json::Value) -> serde_json::Value {
        let event = ApxmEvent::root(
            WarningPayload {
                code: "W000".to_string(),
                message: "template".to_string(),
            },
            EventSource::Acp("test".to_string()),
            "trace-acp-test",
        );
        let mut raw = serde_json::to_value(event).expect("serialize template event");
        raw["payload"] = payload;
        raw
    }

    fn decode(payload: serde_json::Value) -> ApxmEvent {
        ApxmEvent::from_json_with_registry(
            raw_event_with_payload(payload),
            &event_payload_registry(),
        )
        .expect("decode ACP event")
    }

    #[test]
    fn acp_registry_decodes_all_extension_payloads() {
        assert!(
            decode(serde_json::json!({
                "kind": ACP_SESSION_SPAWNED.name(),
                "agent": "codex",
                "session_id": "s1",
                "cwd": "/tmp",
            }))
            .payload
            .downcast_ref::<AcpSessionSpawnedPayload>()
            .is_some()
        );
        assert!(
            decode(serde_json::json!({
                "kind": ACP_SESSION_PROMPT_START.name(),
                "session_id": "s1",
                "prompt_len": 10,
            }))
            .payload
            .downcast_ref::<AcpSessionPromptStartPayload>()
            .is_some()
        );
        assert!(
            decode(serde_json::json!({
                "kind": ACP_SESSION_CHUNK.name(),
                "session_id": "s1",
                "chunk_len": 2,
                "total_len": 12,
            }))
            .payload
            .downcast_ref::<AcpSessionChunkPayload>()
            .is_some()
        );
        assert!(
            decode(serde_json::json!({
                "kind": ACP_SESSION_PROMPT_END.name(),
                "session_id": "s1",
                "stop_reason": "end_turn",
                "input_tokens": 7,
                "output_tokens": 9,
            }))
            .payload
            .downcast_ref::<AcpSessionPromptEndPayload>()
            .is_some()
        );
        assert!(
            decode(serde_json::json!({
                "kind": ACP_SESSION_CLOSED.name(),
                "session_id": "s1",
                "total_turns": 1,
                "close_reason": "done",
            }))
            .payload
            .downcast_ref::<AcpSessionClosedPayload>()
            .is_some()
        );
        assert!(
            decode(serde_json::json!({
                "kind": ACP_REVERSE_REQUEST.name(),
                "session_id": "s1",
                "method": "fs/read",
                "approved": true,
            }))
            .payload
            .downcast_ref::<AcpReverseRequestPayload>()
            .is_some()
        );
        assert!(
            decode(serde_json::json!({
                "kind": ACP_SESSION_ERROR.name(),
                "session_id": "s1",
                "error": "failed",
            }))
            .payload
            .downcast_ref::<AcpSessionErrorPayload>()
            .is_some()
        );
        assert!(
            decode(serde_json::json!({
                "kind": ACP_TOOL_RESULT.name(),
                "id": "tool-1",
                "success": true,
                "output": "ok",
            }))
            .payload
            .downcast_ref::<AcpToolResultPayload>()
            .is_some()
        );
    }

    #[test]
    fn acp_registry_rejects_malformed_registered_payload() {
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": ACP_SESSION_CHUNK.name(),
            "session_id": "s1",
            "chunk_len": "not a number",
            "total_len": 12,
        }));

        let result = ApxmEvent::from_json_with_registry(raw, &event_payload_registry());

        assert!(result.is_err());
    }

    #[test]
    fn acp_registry_preserves_unknown_payloads() {
        let event = decode(serde_json::json!({
            "kind": UNKNOWN_ACP_KIND,
            "session_id": "s1",
            "vendor_payload": { "ok": true },
        }));
        let payload = event
            .payload
            .downcast_ref::<UnknownEventPayload>()
            .expect("unknown event payload");

        assert_eq!(payload.kind_name(), UNKNOWN_ACP_KIND);
        assert_eq!(payload.payload_json()["session_id"], "s1");
    }
}
