//! Reusable product-neutral canonical execution entry point.

pub mod tui;

#[cfg(feature = "dev")]
#[path = "commands/canonical_execute.rs"]
pub mod canonical_execute;
