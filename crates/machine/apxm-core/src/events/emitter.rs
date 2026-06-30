//! The [`EventEmitter`] trait for components that produce events.

use super::event::ApxmEvent;

/// Trait for components that emit APXM events.
///
/// Implementors decide where to send fully-formed [`ApxmEvent`] envelopes.
pub trait EventEmitter: Send + Sync {
    /// Emit an event.
    fn emit(&self, event: ApxmEvent);
}
