//! Builder / convenience constructors for [`ApxmEvent`].

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use super::event::{ApxmEvent, EventMeta, EventSource};
use super::payload::EventPayload;

impl ApxmEvent {
    /// Create a root event (no parent span).
    ///
    /// The span_id is auto-generated and parent_span_id is `None`.
    /// Timestamp is set to `Utc::now()` and sequence defaults to `0`.
    pub fn root(
        payload: impl EventPayload,
        source: EventSource,
        trace_id: impl Into<String>,
    ) -> Self {
        Self::root_shared(Arc::new(payload), source, trace_id)
    }

    /// Create a root event from a shared payload.
    pub fn root_shared(
        payload: Arc<dyn EventPayload>,
        source: EventSource,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            meta: EventMeta {
                seq: 0,
                timestamp: Utc::now(),
                trace_id: trace_id.into(),
                source,
                span_id: Uuid::new_v4().to_string(),
                parent_span_id: None,
                scope_id: None,
            },
            payload,
        }
    }

    /// Create a child event nested under the given parent span.
    ///
    /// A new span_id is auto-generated and parent_span_id is set to the
    /// provided value.
    pub fn child_of(
        payload: impl EventPayload,
        source: EventSource,
        trace_id: impl Into<String>,
        parent_span_id: impl Into<String>,
    ) -> Self {
        Self::child_of_shared(Arc::new(payload), source, trace_id, parent_span_id)
    }

    /// Create a child event from a shared payload.
    pub fn child_of_shared(
        payload: Arc<dyn EventPayload>,
        source: EventSource,
        trace_id: impl Into<String>,
        parent_span_id: impl Into<String>,
    ) -> Self {
        Self {
            meta: EventMeta {
                seq: 0,
                timestamp: Utc::now(),
                trace_id: trace_id.into(),
                source,
                span_id: Uuid::new_v4().to_string(),
                parent_span_id: Some(parent_span_id.into()),
                scope_id: None,
            },
            payload,
        }
    }

    /// Override the sequence number.
    pub fn with_seq(mut self, seq: u64) -> Self {
        self.meta.seq = seq;
        self
    }

    /// Override the span_id (useful when the caller controls span allocation).
    pub fn with_span_id(mut self, span_id: impl Into<String>) -> Self {
        self.meta.span_id = span_id.into();
        self
    }

    /// Set the scope_id for session isolation.
    pub fn with_scope_id(mut self, scope_id: Option<String>) -> Self {
        self.meta.scope_id = scope_id;
        self
    }
}
