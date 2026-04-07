//! Standalone event system for APXM.
//!
//! This crate defines the universal [`ApxmEvent`] envelope, all 33
//! [`EventPayload`] variants (LLM, Runtime, Session layers), and the
//! [`EventEmitter`] trait.
//!
//! It is a leaf dependency — it does **not** depend on `apxm-runtime`,
//! `apxm-backends`, or any other APXM crate.

pub mod builder;
pub mod emitter;
pub mod event;
pub mod payload;

#[cfg(test)]
mod tests;

// ── Re-exports for convenience ──────────────────────────────────────────
pub use emitter::EventEmitter;
pub use event::{ApxmEvent, EventMeta, EventSource};
pub use payload::EventPayload;
