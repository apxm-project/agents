//! Shared template-placeholder parsing.
//!
//! The actual parser lives in `apxm_core::utils::template` so the runtime
//! handlers can share it without taking a dependency on the compiler crate.
//! This module re-exports the canonical helpers.

pub use apxm_core::utils::template::{is_numeric_placeholder, parse_placeholder_names};
