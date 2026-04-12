//! Event system for APXM.
//!
//! Defines the universal [`ApxmEvent`] envelope, typed [`EventKind`] constants,
//! trait-based payloads, and pluggable [`EventEmitter`] sinks.

pub mod builder;
pub mod emitter;
pub mod event;
pub mod kind;
pub mod payload;
pub mod sinks;

#[cfg(test)]
mod tests;

pub use emitter::EventEmitter;
pub use event::{ApxmEvent, EventMeta, EventSource};
pub use kind::{EventCategory, EventKind};
pub use payload::EventPayload;
pub use sinks::{ChannelEmitter, FanOutEmitter, NoOpEmitter};
