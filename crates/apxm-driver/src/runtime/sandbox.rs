//! Sandbox registry configuration for the runtime.
//!
//! Wraps the existing [`ProcessSandbox`] from `apxm-runtime` as a
//! [`SandboxBackend`] implementation and registers it as the default
//! backend.  This provides policy-level isolation (env-var filtering,
//! timeouts) without any OS-level sandboxing — hosts that need stronger
//! isolation register their own backend *after* this one.

use std::sync::Arc;
use std::time::Instant;

use apxm_runtime::sandbox::{policy::SandboxPolicy, process::ProcessSandbox};
use apxm_sandbox::{
    ExecRequest, ExecResult, IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxContext,
    SandboxError, SandboxRegistry, ValidationResult,
};
use async_trait::async_trait;

/// Default APXM sandbox backend backed by [`ProcessSandbox`].
///
/// Translates [`ExecRequest`] into [`ProcessSandbox::execute`] calls.
/// Reports [`IsolationLevel::PolicyOnly`] because `ProcessSandbox` only
/// performs environment-variable filtering and timeout enforcement — no
/// OS-level namespace/seccomp isolation.
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
            name: "apxm-process".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn is_available(&self) -> bool {
        true // ProcessSandbox works on any platform with tokio::process
    }

    fn validate(&self, _request: &ExecRequest) -> ValidationResult {
        // ProcessSandbox can run anything — it just doesn't provide
        // filesystem/network/syscall restrictions.
        ValidationResult::Degraded {
            warnings: vec![
                "ProcessSandbox provides policy-only isolation (env filtering + timeout); \
                 no OS-level sandboxing is applied."
                    .to_string(),
            ],
        }
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        // ProcessSandbox is stateless — no container to start, no VM to boot.
        // We store the policy in the context so execute() can reconstruct the
        // sandbox with the correct settings.
        let id = format!(
            "process-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        Ok(SandboxContext::new(
            id,
            "apxm-process",
            IsolationLevel::PolicyOnly,
            (), // no internal state needed
        ))
    }

    async fn execute(
        &self,
        _ctx: &SandboxContext,
        request: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        // Build a per-request policy from the ExecRequest fields.
        let policy = SandboxPolicy {
            timeout: request.timeout,
            max_output_bytes: request.max_output_bytes,
            working_dir: request.working_dir.clone(),
            ..self.policy.clone()
        };

        let sandbox = ProcessSandbox::new(policy);
        let args_owned: Vec<String> = request.args.clone();
        let args_refs: Vec<&str> = args_owned.iter().map(|s| s.as_str()).collect();

        let start = Instant::now();

        let result = sandbox
            .execute(&request.program, &args_refs, request.stdin_data.as_deref())
            .await
            .map_err(|e| SandboxError::ExecutionFailed(format!("process sandbox: {e}")))?;

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
        // ProcessSandbox is stateless — nothing to clean up.
        Ok(())
    }
}

/// Create and populate a [`SandboxRegistry`] with the default
/// [`ProcessSandboxBackend`].
///
/// This mirrors the pattern of [`configure_llm_registry`] and
/// [`configure_capability_registry`] — it creates the registry, registers
/// the built-in backend, and returns it wrapped in an [`Arc`].
pub fn configure_sandbox_registry() -> Arc<SandboxRegistry> {
    let mut registry = SandboxRegistry::new();
    let backend = Arc::new(ProcessSandboxBackend::with_default_policy());
    registry.register(backend);
    Arc::new(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_capabilities() {
        let backend = ProcessSandboxBackend::with_default_policy();
        let caps = backend.capabilities();
        assert_eq!(caps.isolation_level, IsolationLevel::PolicyOnly);
        assert_eq!(caps.name, "apxm-process");
        assert!(!caps.supports_filesystem_restriction);
        assert!(!caps.supports_network_restriction);
    }

    #[test]
    fn test_is_available() {
        let backend = ProcessSandboxBackend::with_default_policy();
        assert!(backend.is_available());
    }

    #[test]
    fn test_validate_returns_degraded() {
        let backend = ProcessSandboxBackend::with_default_policy();
        let result = backend.validate(&ExecRequest::default());
        assert!(matches!(result, ValidationResult::Degraded { .. }));
    }

    #[tokio::test]
    async fn test_session_lifecycle() {
        let backend = ProcessSandboxBackend::with_default_policy();
        let ctx = backend.create_session().await.unwrap();
        assert_eq!(ctx.backend_name, "apxm-process");
        assert_eq!(ctx.isolation_level, IsolationLevel::PolicyOnly);
        backend.destroy_session(ctx).await.unwrap();
    }

    #[tokio::test]
    async fn test_execute_echo() {
        let backend = ProcessSandboxBackend::with_default_policy();
        let ctx = backend.create_session().await.unwrap();

        let request = ExecRequest {
            program: "echo".to_string(),
            args: vec!["hello".to_string()],
            timeout: Duration::from_secs(5),
            ..ExecRequest::default()
        };

        let result = backend.execute(&ctx, request).await.unwrap();
        assert!(result.success);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout.trim(), "hello");
        assert!(!result.timed_out);

        backend.destroy_session(ctx).await.unwrap();
    }

    #[tokio::test]
    async fn test_execute_timeout() {
        let backend = ProcessSandboxBackend::with_default_policy();
        let ctx = backend.create_session().await.unwrap();

        let request = ExecRequest {
            program: "sleep".to_string(),
            args: vec!["10".to_string()],
            timeout: Duration::from_millis(500),
            ..ExecRequest::default()
        };

        let result = backend.execute(&ctx, request).await.unwrap();
        assert!(result.timed_out);
        assert!(!result.success);

        backend.destroy_session(ctx).await.unwrap();
    }

    #[test]
    fn test_configure_sandbox_registry() {
        let registry = configure_sandbox_registry();
        assert_eq!(registry.len(), 1);
        let caps = registry.list();
        assert_eq!(caps[0].name, "apxm-process");
        assert_eq!(caps[0].isolation_level, IsolationLevel::PolicyOnly);
    }
}
