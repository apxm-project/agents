//! Builder / convenience constructors for [`ApxmEvent`].

use chrono::Utc;

use super::event::{ApxmEvent, EventMeta, EventSource};
use super::payload::EventPayload;

impl ApxmEvent {
    /// Create a new event with the given payload, source, and trace ID.
    ///
    /// The timestamp is set to `Utc::now()` and the sequence number
    /// defaults to `0` (call [`with_seq`](Self::with_seq) to override).
    pub fn new(payload: EventPayload, source: EventSource, trace_id: impl Into<String>) -> Self {
        Self {
            meta: EventMeta {
                seq: 0,
                timestamp: Utc::now(),
                trace_id: trace_id.into(),
                source,
            },
            payload,
        }
    }

    /// Override the sequence number.
    pub fn with_seq(mut self, seq: u64) -> Self {
        self.meta.seq = seq;
        self
    }
}
