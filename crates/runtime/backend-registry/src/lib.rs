//! Backend registration and API-key reference management for APXM.
//!
//! This crate owns the local backend roster stored at `$APXM_HOME/config.toml`.
//! It records backend protocol, endpoint, model metadata, and API-key/env-var
//! references. It does not own APXM credential custody; provider OAuth tokens
//! and sealed credentials belong to the auth plane.

pub mod backend;
pub mod mask;
pub mod validate;

pub use backend::{BackendError, BackendStore};
