//! APXM Sandbox — Platform-agnostic execution isolation interface.
//!
//! This crate defines the [`SandboxBackend`] trait and associated types that
//! allow APXM to delegate tool/command execution to an external isolation
//! mechanism. **APXM never implements OS-level sandboxing directly.**
//!
//! Host applications (Codex, Gemini CLI, custom integrations) implement
//! [`SandboxBackend`] using their own isolation technology (bubblewrap,
//! Seatbelt, Docker, Wasm, etc.) and register the implementation with the
//! APXM runtime at startup.
//!
//! # Design Principle
//!
//! APXM defines the *interface*, not the *implementation*. This crate is
//! intentionally free of any platform-specific code — no `Command::new`,
//! no syscall filters, no namespace manipulation. The only concrete
//! implementation shipped here is [`DefaultBackend`], which delegates to a
//! caller-supplied closure.
//!
//! # Analogy
//!
//! | APXM Concept        | Analogy                          |
//! |---------------------|----------------------------------|
//! | `SandboxBackend`    | LLVM `TargetMachine`             |
//! | `SandboxRegistry`   | Kubernetes CRI runtime selection |
//! | `SecurityManifest`  | Compiler security metadata       |
//! | Host implementations| containerd, CRI-O, Firecracker   |

mod backend;
mod error;
mod manifest;
mod registry;
mod types;

pub use backend::{DefaultBackend, SandboxBackend, ValidationResult};
pub use error::SandboxError;
pub use manifest::{NodeSandboxReq, SecurityManifest};
pub use registry::{SandboxRegistry, SandboxSelection};
pub use types::{ExecRequest, ExecResult, IsolationLevel, SandboxCapabilities, SandboxContext};
