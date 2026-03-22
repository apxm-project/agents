//! The [`EventEmitter`] trait for components that produce events.

use crate::payload::EventPayload;

/// Trait for components that emit APXM events.
///
/// Implementors decide how to wrap the payload in an [`ApxmEvent`](crate::ApxmEvent)
/// envelope (attaching metadata such as sequence numbers and trace IDs)
/// and where to send it (e.g., to an [`EventBus`](crate::EventBus)).
pub trait EventEmitter: Send + Sync {
    /// Emit an event with an auto-generated trace ID.
    fn emit(&self, payload: EventPayload);

    /// Emit an event correlated with an existing trace.
    fn emit_with_trace(&self, payload: EventPayload, trace_id: &str);
}
