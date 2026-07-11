//! APXM Sandbox — Platform-agnostic execution isolation interface.
//!
//! Defines the [`SandboxBackend`] trait and associated types that allow APXM
//! to delegate tool/command execution to an external isolation mechanism.

mod backend;
pub mod constants;
mod error;
mod manifest;
pub mod policy;
pub mod process;
mod registry;
mod types;

pub use backend::{
    DefaultBackend, SandboxBackend, ValidationResult, WrappedChild, WrappedCommand,
    WrappedCommandGuard,
};
pub use error::SandboxError;
pub use manifest::{NodeSandboxReq, SecurityManifest};
pub use registry::{SandboxRegistry, SandboxSelection};
pub use types::{ExecRequest, ExecResult, IsolationLevel, SandboxCapabilities, SandboxContext};
