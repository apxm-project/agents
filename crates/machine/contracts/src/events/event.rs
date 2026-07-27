//! Core event envelope types.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::payload::{
    EventPayload, boxed_payload_from_json, boxed_payload_from_json_with_registry,
};
use super::registry::EventPayloadRegistry;

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

    /// Decode an event from JSON using an explicit extension registry.
    ///
    /// Core event payloads are decoded first, then the provided registry is
    /// checked, then unregistered event kinds are preserved as
    /// [`UnknownEventPayload`].
    pub fn from_json_with_registry(
        value: serde_json::Value,
        registry: &EventPayloadRegistry,
    ) -> serde_json::Result<Self> {
        let raw = RawEvent::deserialize(value).map_err(serde_json::Error::custom)?;
        let kind_name = raw
            .kind_name()
            .map_err(serde_json::Error::custom)?
            .to_owned();
        let payload =
            boxed_payload_from_json_with_registry(&kind_name, raw.payload.clone(), Some(registry))?;

        Ok(Self {
            meta: raw.meta,
            payload: Arc::from(payload),
        })
    }
}

#[derive(Deserialize)]
struct RawEvent {
    meta: EventMeta,
    payload: serde_json::Value,
}

impl RawEvent {
    fn kind_name(&self) -> Result<&str, serde_json::Error> {
        self.payload
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                serde_json::Error::custom("event payload is missing string field `kind`")
            })
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
    /// Unique span identifier for this event.
    pub span_id: String,
    /// Parent span ID for hierarchical span nesting. `None` for root spans.
    pub parent_span_id: Option<String>,
    /// Scope identifier for session isolation. Events within the same scope
    /// share checkpoint state. `None` for the global (root) scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    /// Optional ProgramPackage provenance for events emitted while running a
    /// server-owned ProgramPackage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_package: Option<ProgramPackageEventProvenance>,
}

/// ProgramPackage identity and nesting metadata attached to runtime events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramPackageEventProvenance {
    /// Immutable ProgramPackage id.
    pub program_package_id: String,
    /// Immutable ProgramPackage digest.
    pub program_package_digest: String,
    /// Optional parent ProgramPackage id for nested ProgramPackage execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_program_package_id: Option<String>,
    /// Optional parent execution id for nested ProgramPackage execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,
    /// Entry flow inside the ProgramPackage, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_name: Option<String>,
}

/// Where the event originated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    /// An LLM or inference backend (name stored as the string).
    Backend(String),
    /// The APXM runtime / executor.
    Runtime,
    /// A session-level component (context manager, Invocation tracker).
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
        let raw = RawEvent::deserialize(deserializer)?;
        let kind_name = raw.kind_name().map_err(D::Error::custom)?.to_owned();
        let payload = boxed_payload_from_json(&kind_name, raw.payload)
            .map(Arc::from)
            .map_err(D::Error::custom)?;

        Ok(Self {
            meta: raw.meta,
            payload,
        })
    }
}
