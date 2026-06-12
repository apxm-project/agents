//! Event system for APXM.
//!
//! Defines the universal [`ApxmEvent`] envelope, typed [`EventKind`] constants,
//! trait-based payloads, and pluggable [`EventEmitter`] sinks.

pub mod builder;
pub mod emitter;
pub mod event;
pub mod kind;
pub mod payload;
pub mod registry;
pub mod sinks;


pub use emitter::EventEmitter;
pub use event::{ApxmEvent, EventMeta, EventSource, SkillEventProvenance};
pub use kind::{EventCategory, EventKind, core_event_kind};
pub use payload::{EventPayload, UnknownEventPayload};
pub use registry::{
    EventPayloadDecoder, EventPayloadRegistration, EventPayloadRegistry, EventRegistryError,
    register_event_payload, register_event_payload_decoder, registered_event_kind,
    registered_event_kinds,
};
pub use sinks::{ChannelEmitter, FanOutEmitter, NoOpEmitter};
