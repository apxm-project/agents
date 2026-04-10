//! Backend configuration and Docker lifecycle management for APXM.
//!
//! This module provides unified backend configuration management via the
//! [`backend`] module (reading from `~/.apxm/config.toml`), and Docker
//! container lifecycle management via the [`docker`] module for local backends.

pub mod backend;
pub mod docker;
pub mod mask;
pub mod validate;

// Re-export the primary types from backend module
pub use backend::{BackendError, BackendStore};
