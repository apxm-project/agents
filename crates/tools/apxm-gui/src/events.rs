use apxm_core::events::kind;
use apxm_core::events::{EventCategory, EventKind, EventPayloadRegistry};
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

/// Build a payload decoder registry for GUI-local extension event kinds.
///
/// GUI payloads that intentionally reuse core event kind names stay outside
/// this registry because their schemas are not the core APXM payload schemas.
pub fn event_payload_registry() -> EventPayloadRegistry {
    let mut registry = EventPayloadRegistry::new();
    registry
        .register_payload::<AgentToolResultPayload>(TOOL_RESULT)
        .expect("GUI event kind should be a unique extension");
    registry
        .register_payload::<AgentDonePayload>(DONE)
        .expect("GUI event kind should be a unique extension");
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::events::kind as core_kind;
    use apxm_core::events::payload::{TokenPayload, UnknownEventPayload, WarningPayload};
    use apxm_core::events::{ApxmEvent, EventSource};

    const UNKNOWN_GUI_KIND: &str = "gui_unregistered_extension";

    fn raw_event_with_payload(payload: serde_json::Value) -> serde_json::Value {
        let event = ApxmEvent::root(
            WarningPayload {
                code: "W000".to_string(),
                message: "template".to_string(),
            },
            EventSource::Gui,
            "trace-gui-test",
        );
        let mut raw = serde_json::to_value(event).expect("serialize template event");
        raw["payload"] = payload;
        raw
    }

    #[test]
    fn gui_registry_decodes_local_extension_payloads() {
        let registry = event_payload_registry();

        let tool_result = ApxmEvent::from_json_with_registry(
            raw_event_with_payload(serde_json::json!({
                "kind": TOOL_RESULT.name(),
                "id": "tool-1",
                "success": true,
                "output": "ok",
            })),
            &registry,
        )
        .expect("tool result event");
        assert!(
            tool_result
                .payload
                .downcast_ref::<AgentToolResultPayload>()
                .is_some()
        );

        let done = ApxmEvent::from_json_with_registry(
            raw_event_with_payload(serde_json::json!({
                "kind": DONE.name(),
                "stopReason": "end_turn",
                "sessionId": "s1",
            })),
            &registry,
        )
        .expect("done event");
        assert!(done.payload.downcast_ref::<AgentDonePayload>().is_some());
    }

    #[test]
    fn gui_registry_preserves_unknown_payloads() {
        let event = ApxmEvent::from_json_with_registry(
            raw_event_with_payload(serde_json::json!({
                "kind": UNKNOWN_GUI_KIND,
                "payload": { "ok": true },
            })),
            &event_payload_registry(),
        )
        .expect("unknown GUI event");
        let payload = event
            .payload
            .downcast_ref::<UnknownEventPayload>()
            .expect("unknown payload");

        assert_eq!(payload.kind_name(), UNKNOWN_GUI_KIND);
    }

    #[test]
    fn gui_core_kind_payloads_follow_core_schemas() {
        let valid_core =
            serde_json::from_value::<ApxmEvent>(raw_event_with_payload(serde_json::json!({
                "kind": core_kind::TOKEN.name(),
                "text": "hello",
            })))
            .expect("core token event");
        assert_eq!(
            valid_core
                .payload
                .downcast_ref::<TokenPayload>()
                .expect("core token payload")
                .text,
            "hello"
        );

        let gui_shape =
            serde_json::from_value::<ApxmEvent>(raw_event_with_payload(serde_json::json!({
                "kind": core_kind::TOKEN.name(),
                "token": "hello",
            })));
        assert!(gui_shape.is_err());
    }
}
