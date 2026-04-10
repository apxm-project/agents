//! Event system for APXM.
//!
//! Defines the universal [`ApxmEvent`] envelope, all [`EventPayload`]
//! variants (LLM, Runtime, Session layers), and the [`EventEmitter`] trait.

pub mod builder;
pub mod emitter;
pub mod event;
pub mod payload;

#[cfg(test)]
mod tests;

pub use emitter::EventEmitter;
pub use event::{ApxmEvent, EventMeta, EventSource};
pub use payload::EventPayload;
