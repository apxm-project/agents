//! Python tool worker bridge.
//!
//! Provides the Rust side of the subprocess-based Python tool execution
//! pipeline. The worker process communicates over NDJSON on stdin/stdout
//! and is lazily spawned on the first Python-backed `INV_TOOL` invocation.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐  NDJSON   ┌─────────────────────────┐
//! │ apxm-runtime│ ──stdin─→ │ python -m apxm.tool_worker │
//! │  (Rust)     │ ←stdout── │       (Python)             │
//! └─────────────┘           └─────────────────────────┘
//! ```
//!
//! - `protocol` — Wire types (`CallRequest`, `CallResponse`, etc.)
//! - `registry` — Tool manifest loaded from `tools.json` sidecar
//! - `worker`   — Subprocess management, request multiplexing, timeout

pub mod constants;
pub mod protocol;
pub mod registry;
pub mod worker;

pub use constants::{
    CAPABILITY_NAME, MANIFEST_TEMPFILE_PREFIX, MANIFEST_TEMPFILE_SUFFIX, PYTHON_BIN,
    PYTHON_MODULE_FLAG, TRACE_TARGET, WORKER_MODULE,
};
pub use protocol::{
    CallRequest, CallResponse, ErrorEnvelope, PROTOCOL_VERSION, WorkerRequest, WorkerResponse,
};
pub use registry::{PythonToolRegistry, ToolDescriptor};
pub use worker::PythonToolWorker;

use apxm_core::error::RuntimeError;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

/// Lazy handle to the Python tool worker.
///
/// Wraps `OnceCell` so the subprocess is spawned exactly once, on the
/// first `INV_TOOL` that targets a Python-backed capability.
pub struct PythonToolBridge {
    registry: PythonToolRegistry,
    worker: OnceCell<Arc<PythonToolWorker>>,
    /// Optional OS sandbox backend. When set + available, the python tool/hook
    /// worker is launched confined (bubblewrap): RO root, ephemeral /tmp, no
    /// network. None = run the worker directly (trusted/local).
    sandbox: Option<Arc<dyn crate::sandbox::SandboxBackend>>,
    /// When true, sandboxing is REQUIRED: if no OS-isolating backend is available
    /// the worker spawn fails closed instead of running unsandboxed (set on the
    /// trusted server-python path so its isolation guarantee actually holds).
    sandbox_required: bool,
}

impl PythonToolBridge {
    /// Create a new bridge from a tool registry.
    pub fn new(registry: PythonToolRegistry) -> Self {
        Self {
            registry,
            worker: OnceCell::new(),
            sandbox: None,
            sandbox_required: false,
        }
    }

    /// Attach an OS sandbox backend that confines the python worker process.
    /// `required` makes the spawn fail closed when no isolating backend exists.
    pub fn with_sandbox(
        mut self,
        sandbox: Option<Arc<dyn crate::sandbox::SandboxBackend>>,
        required: bool,
    ) -> Self {
        self.sandbox = sandbox;
        self.sandbox_required = required;
        self
    }

    /// Build from a `tools.json` file path.
    pub fn from_tools_json(path: &std::path::Path) -> Result<Self, RuntimeError> {
        let registry = PythonToolRegistry::from_file(path)?;
        Ok(Self::new(registry))
    }

    /// Check whether a capability name is backed by a Python tool.
    pub fn has_tool(&self, capability_name: &str) -> bool {
        self.registry.contains(capability_name)
    }

    /// Invoke a Python tool by capability name.
    ///
    /// Lazily spawns the worker on first call. Resolves the capability
    /// name to a handler_id via the registry, then dispatches to the worker.
    pub async fn call(
        &self,
        capability_name: &str,
        args: serde_json::Value,
        deadline: Duration,
    ) -> Result<serde_json::Value, RuntimeError> {
        let descriptor =
            self.registry
                .resolve(capability_name)
                .ok_or_else(|| RuntimeError::Capability {
                    capability: capability_name.into(),
                    message: format!(
                        "No Python tool registered for capability '{}'",
                        capability_name
                    ),
                })?;

        let handler_id = descriptor.handler_id.clone();

        let worker = self
            .worker
            .get_or_try_init(|| async {
                let manifest = self.registry.manifest_json()?;
                let w = PythonToolWorker::spawn_with_env(&manifest, &[], self.sandbox.as_ref(), self.sandbox_required).await?;
                Ok::<_, RuntimeError>(Arc::new(w))
            })
            .await?;

        worker.call(&handler_id, args, deadline).await
    }

    /// Invoke a Python lifecycle hook handler directly by its `handler_id`.
    ///
    /// Hooks are not capabilities (no capability-name mapping); they are
    /// dispatched by handler_id over the SAME worker as `@tool` (constitution
    /// #4). `payload` carries the hook event + call/result and the worker
    /// returns the hook's decision object.
    pub async fn call_hook(
        &self,
        handler_id: &str,
        payload: serde_json::Value,
        deadline: Duration,
    ) -> Result<serde_json::Value, RuntimeError> {
        let worker = self
            .worker
            .get_or_try_init(|| async {
                let manifest = self.registry.manifest_json()?;
                let w = PythonToolWorker::spawn_with_env(&manifest, &[], self.sandbox.as_ref(), self.sandbox_required).await?;
                Ok::<_, RuntimeError>(Arc::new(w))
            })
            .await?;
        worker.call(handler_id, payload, deadline).await
    }

    /// Access the underlying registry.
    pub fn registry(&self) -> &PythonToolRegistry {
        &self.registry
    }

    /// Return tool descriptors exposed by this bridge.
    pub fn descriptors(&self) -> impl Iterator<Item = &ToolDescriptor> {
        self.registry.descriptors()
    }
}
