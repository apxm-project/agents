//! Core types for the sandbox interface.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Isolation strength, ordered from weakest to strongest.
///
/// The compiler's [`SecurityManifest`](crate::SecurityManifest) specifies a
/// *minimum* level; the [`SandboxRegistry`](crate::SandboxRegistry) picks the
/// first available backend that meets or exceeds it.
///
/// Host applications report their isolation level via
/// [`SandboxCapabilities::isolation_level`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationLevel {
    /// No isolation — trust the process. Dev/testing only.
    None = 0,
    /// Application-level policy checks (blocklists, allowlists).
    PolicyOnly = 1,
    /// OS-level syscall/namespace isolation (Landlock, seccomp, Seatbelt).
    OsLevel = 2,
    /// Container-level isolation (Docker, Podman, bubblewrap).
    Container = 3,
    /// Hypervisor-level isolation (Firecracker, gVisor).
    Hypervisor = 4,
    /// WebAssembly capability-based sandbox (Wasmtime, Wasmer).
    Wasm = 5,
    /// Remote/air-gapped execution (cloud sandbox, remote VM).
    Remote = 6,
}

impl std::fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::PolicyOnly => write!(f, "policy-only"),
            Self::OsLevel => write!(f, "os-level"),
            Self::Container => write!(f, "container"),
            Self::Hypervisor => write!(f, "hypervisor"),
            Self::Wasm => write!(f, "wasm"),
            Self::Remote => write!(f, "remote"),
        }
    }
}

/// What a backend can provide — reported by each [`SandboxBackend`](crate::SandboxBackend).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCapabilities {
    /// The isolation level this backend provides.
    pub isolation_level: IsolationLevel,
    /// Whether the backend can restrict filesystem access per-path.
    pub supports_filesystem_restriction: bool,
    /// Whether the backend can restrict network access.
    pub supports_network_restriction: bool,
    /// Whether the backend filters syscalls (seccomp, WASI caps, etc.).
    pub supports_syscall_filtering: bool,
    /// Whether the backend enforces CPU/memory limits.
    pub supports_resource_limits: bool,
    /// Human-readable backend name (e.g. "codex-bwrap", "docker", "wasmtime").
    pub name: String,
    /// Backend version string.
    pub version: String,
}

/// A request to execute a command inside the sandbox.
///
/// This is the data that flows from the APXM runtime to the sandbox backend.
/// It is intentionally platform-agnostic — no PID, no file descriptors, no
/// namespace flags. The backend translates this into platform-specific calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    /// Minimum isolation level required for this execution.
    pub min_isolation: IsolationLevel,
    /// The program to execute (e.g. "sh", "python3", "node").
    pub program: String,
    /// Arguments to pass to the program.
    pub args: Vec<String>,
    /// Working directory for the command.
    pub working_dir: Option<PathBuf>,
    /// Environment variables to set.
    pub env: HashMap<String, String>,
    /// Optional data to write to the process's stdin.
    pub stdin_data: Option<String>,
    /// Maximum execution time before the backend should kill the process.
    pub timeout: Duration,
    /// Maximum bytes to capture from stdout+stderr combined.
    pub max_output_bytes: usize,
    /// Paths the command needs to read from (informational — backend decides enforcement).
    pub read_paths: Vec<PathBuf>,
    /// Paths the command needs to write to (informational — backend decides enforcement).
    pub write_paths: Vec<PathBuf>,
    /// Whether the command requires network access.
    pub needs_network: bool,
    /// Whether the command spawns child processes.
    pub needs_process_spawn: bool,
    /// The AIS operation that triggered this execution (for audit/logging).
    pub origin_op: Option<String>,
    /// The node ID in the graph that triggered this execution.
    pub origin_node_id: Option<u64>,
}

impl Default for ExecRequest {
    fn default() -> Self {
        Self {
            min_isolation: IsolationLevel::PolicyOnly,
            program: String::new(),
            args: Vec::new(),
            working_dir: None,
            env: HashMap::new(),
            stdin_data: None,
            timeout: Duration::from_secs(30),
            max_output_bytes: 1024 * 1024, // 1 MB
            read_paths: Vec::new(),
            write_paths: Vec::new(),
            needs_network: false,
            needs_process_spawn: true,
            origin_op: None,
            origin_node_id: None,
        }
    }
}

/// Result of executing a command inside the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    /// Whether the command exited successfully (exit code 0).
    pub success: bool,
    /// The process exit code (None if killed/signaled).
    pub exit_code: Option<i32>,
    /// Captured stdout (may be truncated to `max_output_bytes`).
    pub stdout: String,
    /// Captured stderr (may be truncated to `max_output_bytes`).
    pub stderr: String,
    /// Wall-clock duration of the execution.
    pub duration: Duration,
    /// Whether the process was killed due to timeout.
    pub timed_out: bool,
}

/// Opaque handle for a sandbox execution session.
///
/// Created by [`SandboxBackend::create_session`] and held for the lifetime
/// of a single graph execution. Backends store their internal state
/// (container ID, bwrap PID, VM handle, etc.) behind this handle.
pub struct SandboxContext {
    /// Unique session identifier.
    pub id: String,
    /// Which backend created this context.
    pub backend_name: String,
    /// The isolation level in effect.
    pub isolation_level: IsolationLevel,
    /// Backend-specific opaque state.
    pub(crate) inner: Box<dyn std::any::Any + Send + Sync>,
}

impl SandboxContext {
    /// Create a new sandbox context.
    pub fn new(
        id: impl Into<String>,
        backend_name: impl Into<String>,
        isolation_level: IsolationLevel,
        inner: impl std::any::Any + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            backend_name: backend_name.into(),
            isolation_level,
            inner: Box::new(inner),
        }
    }

    /// Downcast the inner state to a concrete type.
    ///
    /// Used by backend implementations to recover their own state.
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.inner.downcast_ref()
    }
}

impl std::fmt::Debug for SandboxContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxContext")
            .field("id", &self.id)
            .field("backend_name", &self.backend_name)
            .field("isolation_level", &self.isolation_level)
            .finish_non_exhaustive()
    }
}
