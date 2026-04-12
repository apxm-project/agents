//! Core event envelope types.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::payload::{EventPayload, boxed_payload_from_json};

/// Universal event envelope for all APXM events.
#[derive(Debug, Clone)]
pub struct ApxmEvent {
    /// Metadata attached to this event.
    pub meta: EventMeta,
    /// The event payload.
    pub payload: Arc<dyn EventPayload>,
}

impl ApxmEvent {
    /// Access the typed event kind without downcasting.
    pub fn kind(&self) -> super::kind::EventKind {
        self.payload.event_kind()
    }
}

/// Metadata attached to every event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMeta {
    /// Monotonically increasing sequence number within a bus.
    pub seq: u64,
    /// When the event was created.
    pub timestamp: DateTime<Utc>,
    /// Trace/correlation ID for distributed tracing.
    pub trace_id: String,
    /// Where the event originated.
    pub source: EventSource,
}

/// Where the event originated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    /// An LLM or inference backend (name stored as the string).
    Backend(String),
    /// The APXM runtime / executor.
    Runtime,
    /// A session-level component (context manager, turn tracker).
    Session,
    /// The APXM server / API layer.
    Server,
    /// ACP session-scoped events.
    Acp(String),
    /// User-initiated GUI events.
    Gui,
}

impl Serialize for ApxmEvent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut payload_json = self.payload.to_json();
        if let Some(obj) = payload_json.as_object_mut() {
            obj.insert("kind".into(), self.payload.event_kind().name().into());
        } else {
            payload_json = serde_json::json!({
                "kind": self.payload.event_kind().name(),
                "value": payload_json,
            });
        }

        serde_json::json!({
            "meta": self.meta,
            "payload": payload_json,
        })
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ApxmEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct RawEvent {
            meta: EventMeta,
            payload: serde_json::Value,
        }

        let raw = RawEvent::deserialize(deserializer)?;
        let kind_name = raw
            .payload
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| D::Error::custom("event payload is missing string field `kind`"))?
            .to_owned();
        let payload = boxed_payload_from_json(&kind_name, raw.payload)
            .map(Arc::from)
            .map_err(D::Error::custom)?;

        Ok(Self {
            meta: raw.meta,
            payload,
        })
    }
}
