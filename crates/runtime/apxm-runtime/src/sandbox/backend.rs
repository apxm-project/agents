//! The core [`SandboxBackend`] trait.
//!
//! Host applications implement this trait to provide execution isolation.
//! APXM never implements OS-level sandboxing — it delegates entirely to
//! the registered backend.

use super::error::SandboxError;
use super::types::{ExecRequest, ExecResult, SandboxCapabilities, SandboxContext};
use async_trait::async_trait;
use std::path::Path;

const DEFAULT_SESSION_ID_PREFIX: &str = "sandbox";

/// Validation result returned by [`SandboxBackend::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationResult {
    /// Backend can fully satisfy all requirements.
    Ok,
    /// Backend can run but some requirements won't be enforced.
    Degraded { warnings: Vec<String> },
    /// Backend cannot satisfy minimum requirements.
    Unsupported { reason: String },
}

/// The core sandbox abstraction.
///
/// Host applications (Codex, Gemini CLI, Docker orchestrators, Wasm runtimes,
/// cloud sandbox services, etc.) implement this trait and register their
/// implementation with APXM's [`SandboxRegistry`](crate::SandboxRegistry) at
/// startup.
///
/// # Lifecycle
///
/// ```text
/// 1. Host creates backend impl
/// 2. Host registers it: registry.register(backend)
/// 3. APXM runtime calls create_session() once per graph execution
/// 4. For each INV/tool node: runtime calls execute(ctx, request)
/// 5. After graph completes: runtime calls destroy_session(ctx)
/// ```
///
/// # Contract
///
/// - `is_available()` must be cheap and synchronous.
/// - `create_session()` may be expensive (start container, boot VM).
/// - `execute()` runs a single command and returns its output.
/// - `destroy_session()` cleans up resources (stop container, kill VM).
/// - All methods must be safe to call from multiple threads (`Send + Sync`).
///
/// # What APXM guarantees
///
/// - APXM will never call `execute()` without a prior `create_session()`.
/// - APXM will always call `destroy_session()` (even on graph failure).
/// - The `ExecRequest` contains all information the backend needs.
/// - LLM calls (ASK, THINK, REASON) do NOT go through the sandbox —
///   only tool execution (INV, capability calls) does.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Report what this backend can do.
    fn capabilities(&self) -> SandboxCapabilities;

    /// Can this backend operate on the current system?
    ///
    /// Must be cheap and synchronous. Called during backend selection.
    fn is_available(&self) -> bool;

    /// Check whether this backend can satisfy the given requirements.
    ///
    /// Called before `create_session()` to verify compatibility.
    fn validate(&self, request: &ExecRequest) -> ValidationResult;

    /// Create an execution session for a graph run.
    ///
    /// Called once per `apxm execute` invocation. Backends may start
    /// containers, boot VMs, or establish connections here.
    ///
    /// The returned [`SandboxContext`] is passed to every subsequent
    /// `execute()` call and eventually to `destroy_session()`.
    async fn create_session(&self) -> Result<SandboxContext, SandboxError>;

    /// Execute a command inside the sandbox session.
    ///
    /// Called for each INV/tool node in the graph. The backend must:
    /// 1. Apply its isolation mechanism
    /// 2. Run the command described by `request`
    /// 3. Capture stdout/stderr
    /// 4. Enforce timeout
    /// 5. Return the result
    async fn execute(
        &self,
        ctx: &SandboxContext,
        request: ExecRequest,
    ) -> Result<ExecResult, SandboxError>;

    /// Tear down the sandbox session and release resources.
    ///
    /// Called after graph execution completes (success or failure).
    /// Backends should stop containers, kill VMs, clean up temp dirs, etc.
    async fn destroy_session(&self, ctx: SandboxContext) -> Result<(), SandboxError>;

    /// Rewrite a command so it launches under this backend's isolation.
    ///
    /// The one-shot [`execute()`](SandboxBackend::execute) path runs a command
    /// to completion and returns captured output. Long-running children that
    /// stay attached to live stdio for their whole lifetime — ACP coding agents
    /// speaking JSON-RPC over stdin/stdout, interactive terminals — cannot use
    /// it. For those, the caller spawns the process itself and must first ask
    /// the backend to rewrite `(program, args)` into a confined equivalent.
    ///
    /// Isolation wrappers (e.g. `bwrap`) forward stdin/stdout/stderr to the
    /// inner child transparently, so the caller's pipe handling is unaffected.
    ///
    /// `cwd` is bound writable, `needs_network` keeps the network namespace
    /// when true. The default implementation applies no isolation and returns
    /// the command unchanged — appropriate for backends (process-policy,
    /// remote) that cannot confine a caller-owned spawn.
    fn wrap_command(
        &self,
        program: &str,
        args: &[String],
        _cwd: &Path,
        _needs_network: bool,
    ) -> (String, Vec<String>) {
        (program.to_string(), args.to_vec())
    }
}

/// A minimal backend that delegates execution to a caller-supplied function.
///
/// This is NOT a platform-specific implementation — it's a building block
/// that lets host applications wrap their existing execution infrastructure
/// into a [`SandboxBackend`] with minimal boilerplate.
///
/// For hosts that only need to customize `execute()` while using default
/// session management, this avoids implementing the full trait.
pub struct DefaultBackend {
    caps: SandboxCapabilities,
    executor: Box<
        dyn Fn(ExecRequest) -> futures::future::BoxFuture<'static, Result<ExecResult, SandboxError>>
            + Send
            + Sync,
    >,
}

impl DefaultBackend {
    /// Create a default backend with custom capabilities and executor.
    pub fn new<F, Fut>(caps: SandboxCapabilities, executor: F) -> Self
    where
        F: Fn(ExecRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<ExecResult, SandboxError>> + Send + 'static,
    {
        Self {
            caps,
            executor: Box::new(move |req| Box::pin(executor(req))),
        }
    }
}

#[async_trait]
impl SandboxBackend for DefaultBackend {
    fn capabilities(&self) -> SandboxCapabilities {
        self.caps.clone()
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(&self, _request: &ExecRequest) -> ValidationResult {
        ValidationResult::Ok
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        Ok(SandboxContext::new(
            uuid_v7_string(),
            &self.caps.name,
            self.caps.isolation_level,
            (), // no internal state
        ))
    }

    async fn execute(
        &self,
        _ctx: &SandboxContext,
        request: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        (self.executor)(request).await
    }

    async fn destroy_session(&self, _ctx: SandboxContext) -> Result<(), SandboxError> {
        Ok(())
    }
}

fn uuid_v7_string() -> String {
    // Simple monotonic ID without pulling in uuid crate.
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{}-{}-{}",
        DEFAULT_SESSION_ID_PREFIX,
        ts.as_millis(),
        ts.subsec_nanos()
    )
}

impl std::fmt::Debug for DefaultBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultBackend")
            .field("capabilities", &self.caps)
            .finish_non_exhaustive()
    }
}
