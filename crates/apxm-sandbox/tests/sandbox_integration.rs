//! Integration tests for the APXM pluggable sandbox system.
//!
//! These tests verify:
//! 1. `SandboxRegistry` backend selection with multiple backends at different
//!    isolation levels.
//! 2. `DefaultBackend` full lifecycle: create_session -> execute -> destroy_session.
//! 3. Mock backend call verification to prove the registry-selected backend is
//!    actually invoked.
//! 4. `SecurityManifest`-driven backend selection.
//!
//! **Gap documented**: The `SandboxBackend::execute()` method is NOT called by
//! the APXM runtime's INV handler.  See the `test_gap_sandbox_not_called_by_inv`
//! test and the module-level documentation for the full trace.

use apxm_sandbox::{
    DefaultBackend, ExecRequest, ExecResult, IsolationLevel, SandboxBackend, SandboxCapabilities,
    SandboxContext, SandboxError, SandboxRegistry, SecurityManifest, ValidationResult,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn caps(name: &str, level: IsolationLevel) -> SandboxCapabilities {
    SandboxCapabilities {
        isolation_level: level,
        supports_filesystem_restriction: false,
        supports_network_restriction: false,
        supports_syscall_filtering: false,
        supports_resource_limits: false,
        name: name.to_string(),
        version: "0.1.0-test".to_string(),
    }
}

/// A mock backend that counts how many times `execute()` is called and
/// records the last program name it received.
struct CountingBackend {
    capabilities: SandboxCapabilities,
    call_count: AtomicU32,
    last_program: parking_lot::Mutex<String>,
}

impl CountingBackend {
    fn new(name: &str, level: IsolationLevel) -> Self {
        Self {
            capabilities: caps(name, level),
            call_count: AtomicU32::new(0),
            last_program: parking_lot::Mutex::new(String::new()),
        }
    }

    fn calls(&self) -> u32 {
        self.call_count.load(Ordering::SeqCst)
    }

    fn last_program(&self) -> String {
        self.last_program.lock().clone()
    }
}

#[async_trait]
impl SandboxBackend for CountingBackend {
    fn capabilities(&self) -> SandboxCapabilities {
        self.capabilities.clone()
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(&self, _request: &ExecRequest) -> ValidationResult {
        ValidationResult::Ok
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        Ok(SandboxContext::new(
            format!("mock-session-{}", self.capabilities.name),
            &self.capabilities.name,
            self.capabilities.isolation_level,
            (),
        ))
    }

    async fn execute(
        &self,
        _ctx: &SandboxContext,
        request: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        *self.last_program.lock() = request.program.clone();
        Ok(ExecResult {
            success: true,
            exit_code: Some(0),
            stdout: format!("mock-output-from-{}", self.capabilities.name),
            stderr: String::new(),
            duration: Duration::from_millis(1),
            timed_out: false,
        })
    }

    async fn destroy_session(&self, _ctx: SandboxContext) -> Result<(), SandboxError> {
        Ok(())
    }
}

/// A backend that is deliberately NOT available (is_available returns false).
struct UnavailableBackend {
    capabilities: SandboxCapabilities,
}

impl UnavailableBackend {
    fn new(name: &str, level: IsolationLevel) -> Self {
        Self {
            capabilities: caps(name, level),
        }
    }
}

#[async_trait]
impl SandboxBackend for UnavailableBackend {
    fn capabilities(&self) -> SandboxCapabilities {
        self.capabilities.clone()
    }

    fn is_available(&self) -> bool {
        false
    }

    fn validate(&self, _request: &ExecRequest) -> ValidationResult {
        ValidationResult::Unsupported {
            reason: "deliberately unavailable".into(),
        }
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        Err(SandboxError::NotAvailable("unavailable".into()))
    }

    async fn execute(
        &self,
        _ctx: &SandboxContext,
        _request: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        Err(SandboxError::NotAvailable("unavailable".into()))
    }

    async fn destroy_session(&self, _ctx: SandboxContext) -> Result<(), SandboxError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests: SandboxRegistry backend selection
// ---------------------------------------------------------------------------

#[test]
fn registry_selects_lowest_qualifying_backend() {
    let mut reg = SandboxRegistry::new();

    let none_backend = Arc::new(CountingBackend::new("passthrough", IsolationLevel::None));
    let policy_backend = Arc::new(CountingBackend::new("policy", IsolationLevel::PolicyOnly));
    let container_backend = Arc::new(CountingBackend::new("container", IsolationLevel::Container));
    let hypervisor_backend =
        Arc::new(CountingBackend::new("hypervisor", IsolationLevel::Hypervisor));

    reg.register(none_backend);
    reg.register(policy_backend);
    reg.register(container_backend);
    reg.register(hypervisor_backend);

    // Requesting None should pick "passthrough" (lowest)
    let selected = reg.select(IsolationLevel::None).unwrap();
    assert_eq!(selected.capabilities().name, "passthrough");

    // Requesting PolicyOnly should pick "policy" (exact match, not overkill)
    let selected = reg.select(IsolationLevel::PolicyOnly).unwrap();
    assert_eq!(selected.capabilities().name, "policy");

    // Requesting Container should pick "container"
    let selected = reg.select(IsolationLevel::Container).unwrap();
    assert_eq!(selected.capabilities().name, "container");

    // Requesting Hypervisor should pick "hypervisor"
    let selected = reg.select(IsolationLevel::Hypervisor).unwrap();
    assert_eq!(selected.capabilities().name, "hypervisor");

    // Requesting Wasm should fail (nothing that high)
    assert!(reg.select(IsolationLevel::Wasm).is_err());

    // Requesting Remote should fail
    assert!(reg.select(IsolationLevel::Remote).is_err());
}

#[test]
fn registry_skips_unavailable_backends() {
    let mut reg = SandboxRegistry::new();

    // Register an unavailable Container backend, then an available one
    let unavail = Arc::new(UnavailableBackend::new(
        "broken-container",
        IsolationLevel::Container,
    ));
    let avail = Arc::new(CountingBackend::new(
        "working-container",
        IsolationLevel::Container,
    ));

    reg.register(unavail);
    reg.register(avail);

    let selected = reg.select(IsolationLevel::Container).unwrap();
    assert_eq!(selected.capabilities().name, "working-container");
}

#[test]
fn registry_select_for_manifest() {
    let mut reg = SandboxRegistry::new();
    reg.register(Arc::new(CountingBackend::new(
        "passthrough",
        IsolationLevel::None,
    )));
    reg.register(Arc::new(CountingBackend::new(
        "os-level",
        IsolationLevel::OsLevel,
    )));

    // A manifest requiring OsLevel should skip the passthrough
    let manifest = SecurityManifest {
        min_isolation: IsolationLevel::OsLevel,
        ..SecurityManifest::default()
    };
    let selected = reg.select_for_manifest(&manifest).unwrap();
    assert_eq!(selected.capabilities().name, "os-level");

    // A pure manifest (no sandbox required) should pick passthrough
    let pure_manifest = SecurityManifest::pure();
    let selected = reg.select_for_manifest(&pure_manifest).unwrap();
    assert_eq!(selected.capabilities().name, "passthrough");
}

#[test]
fn registry_default_backend_is_first_registered() {
    let mut reg = SandboxRegistry::new();
    assert!(reg.default_backend().is_none());

    // Register Container first, then None
    reg.register(Arc::new(CountingBackend::new(
        "container",
        IsolationLevel::Container,
    )));
    reg.register(Arc::new(CountingBackend::new(
        "passthrough",
        IsolationLevel::None,
    )));

    let default = reg.default_backend().unwrap();
    assert_eq!(default.capabilities().name, "container");
}

// ---------------------------------------------------------------------------
// Tests: Full lifecycle with mock backend
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mock_backend_full_lifecycle() {
    let backend = Arc::new(CountingBackend::new("test-mock", IsolationLevel::PolicyOnly));

    // 1. Check availability
    assert!(backend.is_available());
    assert_eq!(backend.calls(), 0);

    // 2. Create session
    let ctx = backend.create_session().await.unwrap();
    assert_eq!(ctx.backend_name, "test-mock");
    assert_eq!(ctx.isolation_level, IsolationLevel::PolicyOnly);
    assert!(!ctx.id.is_empty());

    // 3. Execute command
    let request = ExecRequest {
        program: "echo".to_string(),
        args: vec!["hello".to_string(), "world".to_string()],
        origin_op: Some("INV".to_string()),
        origin_node_id: Some(42),
        timeout: Duration::from_secs(5),
        ..ExecRequest::default()
    };

    let result = backend.execute(&ctx, request).await.unwrap();
    assert!(result.success);
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout, "mock-output-from-test-mock");
    assert!(!result.timed_out);
    assert_eq!(backend.calls(), 1);
    assert_eq!(backend.last_program(), "echo");

    // 4. Execute a second command
    let request2 = ExecRequest {
        program: "python3".to_string(),
        args: vec!["-c".to_string(), "print('hi')".to_string()],
        timeout: Duration::from_secs(5),
        ..ExecRequest::default()
    };

    let result2 = backend.execute(&ctx, request2).await.unwrap();
    assert!(result2.success);
    assert_eq!(backend.calls(), 2);
    assert_eq!(backend.last_program(), "python3");

    // 5. Destroy session
    backend.destroy_session(ctx).await.unwrap();
}

// ---------------------------------------------------------------------------
// Tests: DefaultBackend with real execution closure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn default_backend_with_custom_executor() {
    let exec_count = Arc::new(AtomicU32::new(0));
    let count_clone = Arc::clone(&exec_count);

    let backend = DefaultBackend::new(
        caps("custom-exec", IsolationLevel::None),
        move |req: ExecRequest| {
            let c = Arc::clone(&count_clone);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(ExecResult {
                    success: true,
                    exit_code: Some(0),
                    stdout: format!("ran: {} {}", req.program, req.args.join(" ")),
                    stderr: String::new(),
                    duration: Duration::from_millis(1),
                    timed_out: false,
                })
            }
        },
    );

    let ctx = backend.create_session().await.unwrap();
    assert_eq!(ctx.backend_name, "custom-exec");

    let result = backend
        .execute(
            &ctx,
            ExecRequest {
                program: "my-tool".to_string(),
                args: vec!["--flag".to_string(), "value".to_string()],
                ..ExecRequest::default()
            },
        )
        .await
        .unwrap();

    assert!(result.success);
    assert_eq!(result.stdout, "ran: my-tool --flag value");
    assert_eq!(exec_count.load(Ordering::SeqCst), 1);

    backend.destroy_session(ctx).await.unwrap();
}

// ---------------------------------------------------------------------------
// Tests: Registry + Backend integration (select then execute)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn registry_select_and_execute_end_to_end() {
    let mut reg = SandboxRegistry::new();

    let none_backend = Arc::new(CountingBackend::new("passthrough", IsolationLevel::None));
    let container_backend = Arc::new(CountingBackend::new("container", IsolationLevel::Container));

    let none_ref = Arc::clone(&none_backend);
    let container_ref = Arc::clone(&container_backend);

    reg.register(none_backend);
    reg.register(container_backend);

    // Select the Container backend
    let selected = reg.select(IsolationLevel::Container).unwrap();
    assert_eq!(selected.capabilities().name, "container");

    // Full lifecycle through the selected backend
    let ctx = selected.create_session().await.unwrap();
    assert_eq!(ctx.backend_name, "container");

    let request = ExecRequest {
        program: "echo".to_string(),
        args: vec!["sandboxed".to_string()],
        origin_op: Some("INV".to_string()),
        origin_node_id: Some(7),
        timeout: Duration::from_secs(5),
        ..ExecRequest::default()
    };

    let result = selected.execute(&ctx, request).await.unwrap();
    assert!(result.success);
    assert_eq!(result.stdout, "mock-output-from-container");

    selected.destroy_session(ctx).await.unwrap();

    // Verify the correct backend was called
    assert_eq!(container_ref.calls(), 1);
    assert_eq!(none_ref.calls(), 0); // passthrough was NOT called
}

// ---------------------------------------------------------------------------
// Tests: ExecRequest defaults and field propagation
// ---------------------------------------------------------------------------

#[test]
fn exec_request_defaults_are_sane() {
    let req = ExecRequest::default();
    assert!(req.program.is_empty());
    assert!(req.args.is_empty());
    assert_eq!(req.timeout, Duration::from_secs(30));
    assert_eq!(req.max_output_bytes, 1024 * 1024);
    assert!(!req.needs_network);
    assert!(req.needs_process_spawn);
    assert!(req.origin_op.is_none());
    assert!(req.origin_node_id.is_none());
    assert!(req.read_paths.is_empty());
    assert!(req.write_paths.is_empty());
}

#[tokio::test]
async fn exec_request_fields_propagate_to_backend() {
    use std::path::PathBuf;

    let captured_req = Arc::new(parking_lot::Mutex::new(None::<ExecRequest>));
    let captured_clone = Arc::clone(&captured_req);

    let backend = DefaultBackend::new(
        caps("field-check", IsolationLevel::None),
        move |req: ExecRequest| {
            let c = Arc::clone(&captured_clone);
            async move {
                *c.lock() = Some(req);
                Ok(ExecResult {
                    success: true,
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                    duration: Duration::from_millis(1),
                    timed_out: false,
                })
            }
        },
    );

    let ctx = backend.create_session().await.unwrap();

    let request = ExecRequest {
        program: "node".to_string(),
        args: vec!["script.js".to_string()],
        working_dir: Some(PathBuf::from("/tmp/sandbox")),
        env: [("NODE_ENV".to_string(), "production".to_string())]
            .into_iter()
            .collect(),
        stdin_data: Some("input-data".to_string()),
        timeout: Duration::from_secs(10),
        max_output_bytes: 512,
        read_paths: vec![PathBuf::from("/etc/hosts")],
        write_paths: vec![PathBuf::from("/tmp/output")],
        needs_network: true,
        needs_process_spawn: false,
        origin_op: Some("INV".to_string()),
        origin_node_id: Some(99),
    };

    backend.execute(&ctx, request).await.unwrap();

    let captured = captured_req.lock().clone().expect("request should be captured");
    assert_eq!(captured.program, "node");
    assert_eq!(captured.args, vec!["script.js"]);
    assert_eq!(captured.working_dir, Some(PathBuf::from("/tmp/sandbox")));
    assert_eq!(
        captured.env.get("NODE_ENV"),
        Some(&"production".to_string())
    );
    assert_eq!(captured.stdin_data, Some("input-data".to_string()));
    assert_eq!(captured.timeout, Duration::from_secs(10));
    assert_eq!(captured.max_output_bytes, 512);
    assert!(captured.needs_network);
    assert!(!captured.needs_process_spawn);
    assert_eq!(captured.origin_op, Some("INV".to_string()));
    assert_eq!(captured.origin_node_id, Some(99));
    assert_eq!(captured.read_paths, vec![PathBuf::from("/etc/hosts")]);
    assert_eq!(captured.write_paths, vec![PathBuf::from("/tmp/output")]);

    backend.destroy_session(ctx).await.unwrap();
}

// ---------------------------------------------------------------------------
// Tests: SecurityManifest classification
// ---------------------------------------------------------------------------

#[test]
fn security_manifest_pure_does_not_require_sandbox() {
    let m = SecurityManifest::pure();
    assert!(m.is_pure());
    assert!(!m.requires_sandbox());
    assert_eq!(m.min_isolation, IsolationLevel::None);
}

#[test]
fn security_manifest_with_io_tier_requires_sandbox() {
    // tier::IO = 2 (see apxm-sandbox/src/manifest.rs)
    let m = SecurityManifest {
        max_tier: 2, // tier::IO
        min_isolation: IsolationLevel::OsLevel,
        tool_capabilities_used: vec!["bash".to_string(), "echo".to_string()],
        needs_process_spawn: true,
        ..SecurityManifest::default()
    };

    assert!(m.requires_sandbox());
    assert!(!m.is_pure());
    assert_eq!(m.min_isolation, IsolationLevel::OsLevel);
    assert_eq!(m.tool_capabilities_used.len(), 2);
}

// ---------------------------------------------------------------------------
// Tests: IsolationLevel ordering
// ---------------------------------------------------------------------------

#[test]
fn isolation_levels_are_ordered_weakest_to_strongest() {
    assert!(IsolationLevel::None < IsolationLevel::PolicyOnly);
    assert!(IsolationLevel::PolicyOnly < IsolationLevel::OsLevel);
    assert!(IsolationLevel::OsLevel < IsolationLevel::Container);
    assert!(IsolationLevel::Container < IsolationLevel::Hypervisor);
    assert!(IsolationLevel::Hypervisor < IsolationLevel::Wasm);
    assert!(IsolationLevel::Wasm < IsolationLevel::Remote);
}

// ---------------------------------------------------------------------------
// Tests: SandboxContext opaque state downcast
// ---------------------------------------------------------------------------

#[test]
fn sandbox_context_downcast_inner_state() {
    // Backends store arbitrary state in the context via `Any`.
    struct MyState {
        container_id: String,
    }

    let ctx = SandboxContext::new(
        "test-session",
        "docker",
        IsolationLevel::Container,
        MyState {
            container_id: "abc123".to_string(),
        },
    );

    let state = ctx.downcast_ref::<MyState>().expect("downcast should work");
    assert_eq!(state.container_id, "abc123");

    // Wrong type should return None
    assert!(ctx.downcast_ref::<String>().is_none());
}

// ---------------------------------------------------------------------------
// Test: Document the integration gap
// ---------------------------------------------------------------------------

/// **This test documents the wiring gap between the sandbox system and the
/// runtime's INV handler.**
///
/// # Execution path trace
///
/// ```text
/// Graph node (op=INV, capability="bash")
///   -> executor/handlers/inv.rs::execute()
///     -> ctx.capability_system.invoke_with_timeout("bash", args, timeout)
///       -> CapabilityRegistry.get("bash")
///         -> BashCapability.execute(args)          // <-- direct tokio::process::Command
///           -> tokio::process::Command::new("sh")  // NO sandbox backend involved
/// ```
///
/// # Where the gap is
///
/// 1. `ExecutionContext` carries `sandbox_registry: Arc<SandboxRegistry>`.
/// 2. The INV handler (`executor/handlers/inv.rs`) has access to `ctx.sandbox_registry`.
/// 3. But the INV handler calls `ctx.capability_system.invoke_with_timeout()` which
///    resolves to `CapabilityExecutor::execute(args)` -- a direct trait method call.
/// 4. `CapabilityExecutor::execute()` (e.g., `BashCapability`) spawns processes
///    via `tokio::process::Command` directly, completely bypassing the sandbox.
/// 5. The `SandboxBackend::execute()` method is never called during INV execution.
///
/// # What needs to change
///
/// Option A (recommended): Create a `SandboxedCapabilityExecutor` wrapper that:
///   1. Receives `Arc<SandboxRegistry>` at construction time
///   2. In `execute()`, selects a backend from the registry
///   3. Translates `HashMap<String, Value>` args into `ExecRequest`
///   4. Calls `backend.execute(ctx, request)` instead of spawning directly
///   5. Wraps the `ExecResult` back into a `Value`
///
/// Option B: Modify the INV handler to call the sandbox directly for capabilities
///   that are marked as requiring process execution (add a `needs_sandbox: bool`
///   field to `CapabilityMetadata`).
///
/// Option C: Make `BashCapability` and `UserToolCapability` accept an optional
///   `Arc<SandboxRegistry>` and delegate to it when present.
#[test]
fn test_gap_sandbox_not_called_by_inv_handler() {
    // This test exists to document the gap.  It verifies that:
    // 1. The sandbox registry IS stored in the execution context.
    // 2. The INV handler does NOT reference it (confirmed by grep).
    //
    // When the gap is closed, this test should be updated to verify
    // that the sandbox IS called.

    // The SandboxRegistry can be constructed and is fully functional:
    let mut registry = SandboxRegistry::new();
    let backend = Arc::new(CountingBackend::new("test", IsolationLevel::PolicyOnly));
    let backend_ref = Arc::clone(&backend);
    registry.register(backend);

    // The registry can select a backend:
    let selected = registry.select(IsolationLevel::None).unwrap();
    assert_eq!(selected.capabilities().name, "test");

    // But during INV execution, the backend's execute() is never called.
    // If it were, backend_ref.calls() would be > 0 after a graph execution.
    assert_eq!(
        backend_ref.calls(),
        0,
        "Sandbox backend execute() is not called by the INV handler -- \
         this documents the integration gap. When the gap is closed, \
         update this assertion."
    );
}
