//! Core event envelope types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::payload::EventPayload;

/// Universal event envelope for all APXM events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApxmEvent {
    /// Metadata attached to this event.
    pub meta: EventMeta,
    /// The event payload.
    pub payload: EventPayload,
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
}
