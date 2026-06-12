//! Sandbox registry configuration for the runtime.
//!
//! The sandbox interface lives in `apxm_runtime::sandbox`. The driver registers
//! host-side implementations. On Linux we prefer a bubblewrap-backed backend
//! that enforces read-only-by-default filesystem access and optional network
//! isolation. The process backend remains as a portable degraded fallback.

#[cfg(target_os = "linux")]
#[path = "sandbox_linux.rs"]
mod sandbox_linux;

use std::sync::Arc;
use std::time::Instant;

use apxm_runtime::sandbox::constants::{backend_names, session_prefixes};
use apxm_runtime::sandbox::{
    ExecRequest, ExecResult, IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxContext,
    SandboxError, SandboxRegistry, ValidationResult,
};
use apxm_runtime::sandbox::{policy::SandboxPolicy, process::ProcessSandbox};
use async_trait::async_trait;

#[cfg(target_os = "linux")]
pub use sandbox_linux::BubblewrapSandboxBackend;

const WARN_PROCESS_NO_OS_ISOLATION: &str =
    "process fallback provides policy-only isolation and cannot enforce OS-level sandboxing";
const WARN_PROCESS_NO_FILESYSTEM_RESTRICTION: &str =
    "process fallback cannot enforce filesystem read/write restrictions";
const WARN_PROCESS_NO_NETWORK_RESTRICTION: &str =
    "process fallback cannot enforce network isolation";
const ERR_PROCESS_SANDBOX_PREFIX: &str = "process sandbox";

/// Default APXM sandbox backend backed by [`ProcessSandbox`].
///
/// This backend is intentionally weak and exists as a portability fallback.
/// It enforces timeouts, controlled environment propagation, and optional
/// command allowlists, but it does not provide namespace or syscall isolation.
pub struct ProcessSandboxBackend {
    policy: SandboxPolicy,
}

impl ProcessSandboxBackend {
    /// Create a new backend with the given sandbox policy.
    pub fn new(policy: SandboxPolicy) -> Self {
        Self { policy }
    }

    /// Create a new backend with the default policy.
    pub fn with_default_policy() -> Self {
        Self::new(SandboxPolicy::default())
    }
}

#[async_trait]
impl SandboxBackend for ProcessSandboxBackend {
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            isolation_level: IsolationLevel::PolicyOnly,
            supports_filesystem_restriction: false,
            supports_network_restriction: false,
            supports_syscall_filtering: false,
            supports_resource_limits: false,
            name: backend_names::PROCESS.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(&self, request: &ExecRequest) -> ValidationResult {
        let mut warnings = Vec::new();

        if request.min_isolation > IsolationLevel::PolicyOnly {
            warnings.push(WARN_PROCESS_NO_OS_ISOLATION.to_string());
        }
        if !request.read_paths.is_empty() || !request.write_paths.is_empty() {
            warnings.push(WARN_PROCESS_NO_FILESYSTEM_RESTRICTION.to_string());
        }
        if !request.needs_network {
            warnings.push(WARN_PROCESS_NO_NETWORK_RESTRICTION.to_string());
        }

        if warnings.is_empty() {
            ValidationResult::Ok
        } else {
            ValidationResult::Degraded { warnings }
        }
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        Ok(SandboxContext::new(
            session_id(session_prefixes::PROCESS),
            backend_names::PROCESS,
            IsolationLevel::PolicyOnly,
            (),
        ))
    }

    async fn execute(
        &self,
        _ctx: &SandboxContext,
        request: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        let policy = SandboxPolicy {
            timeout: request.timeout,
            max_output_bytes: request.max_output_bytes,
            working_dir: request.working_dir.clone(),
            env_overrides: request.env.clone(),
            ..self.policy.clone()
        };

        let sandbox = ProcessSandbox::new(policy);
        let args_owned: Vec<String> = request.args.clone();
        let args_refs: Vec<&str> = args_owned.iter().map(|s| s.as_str()).collect();

        let start = Instant::now();
        let result = sandbox
            .execute(&request.program, &args_refs, request.stdin_data.as_deref())
            .await
            .map_err(map_process_error)?;

        let duration = start.elapsed();

        if result.timed_out {
            return Ok(ExecResult {
                success: false,
                exit_code: None,
                stdout: result.stdout,
                stderr: result.stderr,
                duration,
                timed_out: true,
            });
        }

        Ok(ExecResult {
            success: result.exit_code == 0,
            exit_code: Some(result.exit_code),
            stdout: result.stdout,
            stderr: result.stderr,
            duration,
            timed_out: false,
        })
    }

    async fn destroy_session(&self, _ctx: SandboxContext) -> Result<(), SandboxError> {
        Ok(())
    }
}

/// Create and populate a [`SandboxRegistry`] with the best available host
/// backends for this platform.
pub fn configure_sandbox_registry() -> Arc<SandboxRegistry> {
    let mut registry = SandboxRegistry::new();

    #[cfg(target_os = "linux")]
    {
        let bubblewrap_backend = Arc::new(BubblewrapSandboxBackend::with_default_policy());
        if bubblewrap_backend.is_available() {
            registry.register(bubblewrap_backend);
        }
    }

    registry.register(Arc::new(ProcessSandboxBackend::with_default_policy()));
    Arc::new(registry)
}

fn map_process_error(error: std::io::Error) -> SandboxError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => SandboxError::PermissionDenied(error.to_string()),
        _ => SandboxError::ExecutionFailed(format!("{ERR_PROCESS_SANDBOX_PREFIX}: {error}")),
    }
}

fn session_id(prefix: &str) -> String {
    format!("{}-{}", prefix, monotonic_nanos())
}

fn monotonic_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

