//! End-to-end integration tests for the APXM sandbox system.
//!
//! These tests exercise the full stack:
//!   `ProcessSandboxBackend` -> `SandboxBackend` trait -> actual process execution
//!
//! They also verify the driver's `configure_sandbox_registry()` wiring and
//! demonstrate a full graph execution with INV nodes.
//!
//! # Architecture note
//!
//! The `ProcessSandboxBackend` in `apxm-driver/src/runtime/sandbox.rs` wraps
//! `apxm-runtime`'s `ProcessSandbox` into a `SandboxBackend` implementation.
//! The driver registers it via `configure_sandbox_registry()` and injects it
//! into the `Runtime` via `set_sandbox_registry()`.
//!
//! The capability/INV path routes any capability that declares an `ExecRequest`
//! (via `to_exec_request`) through the selected `SandboxBackend`, so process
//! tools run confined or fail closed -- never unsandboxed.

use apxm_runtime::sandbox::constants::backend_names;
use apxm_runtime::sandbox::{
    DefaultBackend, ExecRequest, ExecResult, IsolationLevel, SandboxBackend, SandboxCapabilities,
    SandboxRegistry,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Tests: ProcessSandboxBackend (real process execution)
// ---------------------------------------------------------------------------

/// Test that the driver's ProcessSandboxBackend can actually execute commands.
/// This verifies the full path: ExecRequest -> ProcessSandbox -> tokio::process.
#[tokio::test]
async fn process_sandbox_backend_executes_echo() {
    use apxm_driver::runtime::sandbox::ProcessSandboxBackend;

    let backend = ProcessSandboxBackend::with_default_policy();

    // Verify capabilities
    let caps = backend.capabilities();
    assert_eq!(caps.name, backend_names::PROCESS);
    assert_eq!(caps.isolation_level, IsolationLevel::PolicyOnly);
    assert!(backend.is_available());

    // Full lifecycle
    let ctx = backend.create_session().await.unwrap();
    assert_eq!(ctx.backend_name, backend_names::PROCESS);

    let request = ExecRequest {
        program: "echo".to_string(),
        args: vec!["sandbox-test".to_string()],
        timeout: Duration::from_secs(5),
        origin_op: Some("INV".to_string()),
        origin_node_id: Some(1),
        ..ExecRequest::default()
    };

    let result = backend.execute(&ctx, request).await.unwrap();
    assert!(result.success);
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout.trim(), "sandbox-test");
    assert!(!result.timed_out);

    backend.destroy_session(ctx).await.unwrap();
}

/// Test that ProcessSandboxBackend properly enforces timeouts.
#[tokio::test]
async fn process_sandbox_backend_timeout() {
    use apxm_driver::runtime::sandbox::ProcessSandboxBackend;

    let backend = ProcessSandboxBackend::with_default_policy();
    let ctx = backend.create_session().await.unwrap();

    let request = ExecRequest {
        program: "sleep".to_string(),
        args: vec!["10".to_string()],
        timeout: Duration::from_millis(300),
        ..ExecRequest::default()
    };

    let result = backend.execute(&ctx, request).await.unwrap();
    assert!(result.timed_out);
    assert!(!result.success);

    backend.destroy_session(ctx).await.unwrap();
}

/// Test that ProcessSandboxBackend captures stderr and non-zero exit codes.
#[tokio::test]
async fn process_sandbox_backend_nonzero_exit() {
    use apxm_driver::runtime::sandbox::ProcessSandboxBackend;

    let backend = ProcessSandboxBackend::with_default_policy();
    let ctx = backend.create_session().await.unwrap();

    let request = ExecRequest {
        program: "sh".to_string(),
        args: vec!["-c".to_string(), "echo err >&2; exit 42".to_string()],
        timeout: Duration::from_secs(5),
        ..ExecRequest::default()
    };

    let result = backend.execute(&ctx, request).await.unwrap();
    assert!(!result.success);
    assert_eq!(result.exit_code, Some(42));
    assert!(result.stderr.contains("err"));
    assert!(!result.timed_out);

    backend.destroy_session(ctx).await.unwrap();
}

/// Test the driver's configure_sandbox_registry() helper.
#[test]
fn configure_sandbox_registry_returns_populated_registry() {
    use apxm_driver::runtime::sandbox::configure_sandbox_registry;

    let registry = configure_sandbox_registry();
    let caps = registry.list();
    assert!(!caps.is_empty());
    assert!(caps.iter().any(|cap| cap.name == backend_names::PROCESS));

    // Can select the backend for PolicyOnly
    let backend = registry.select(IsolationLevel::PolicyOnly).unwrap();
    assert_eq!(backend.capabilities().name, backend_names::PROCESS);

    // Can also select for None (PolicyOnly >= None)
    let backend = registry.select(IsolationLevel::None).unwrap();
    assert_eq!(backend.capabilities().name, backend_names::PROCESS);
}

// ---------------------------------------------------------------------------
// Tests: Multiple backends with different isolation levels
// ---------------------------------------------------------------------------

/// Helper to build a counting mock backend for the registry tests.
fn counting_caps(name: &str, level: IsolationLevel) -> SandboxCapabilities {
    SandboxCapabilities {
        isolation_level: level,
        supports_filesystem_restriction: level >= IsolationLevel::Container,
        supports_network_restriction: level >= IsolationLevel::Container,
        supports_syscall_filtering: level >= IsolationLevel::OsLevel,
        supports_resource_limits: level >= IsolationLevel::Container,
        name: name.to_string(),
        version: "test".to_string(),
    }
}

#[tokio::test]
async fn multi_backend_registry_selects_correctly() {
    use apxm_driver::runtime::sandbox::ProcessSandboxBackend;

    let mut reg = SandboxRegistry::new();

    // Register the real ProcessSandboxBackend as the lowest-level option
    let process_backend = Arc::new(ProcessSandboxBackend::with_default_policy());
    reg.register(process_backend);

    // Register a mock "container" backend
    let container_count = Arc::new(AtomicU32::new(0));
    let cc = Arc::clone(&container_count);
    let container_backend = DefaultBackend::new(
        counting_caps("mock-container", IsolationLevel::Container),
        move |_req: ExecRequest| {
            let c = Arc::clone(&cc);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(ExecResult {
                    success: true,
                    exit_code: Some(0),
                    stdout: "from-container".to_string(),
                    stderr: String::new(),
                    duration: Duration::from_millis(1),
                    timed_out: false,
                })
            }
        },
    );
    reg.register(Arc::new(container_backend));

    // Register a mock "hypervisor" backend
    let hv_count = Arc::new(AtomicU32::new(0));
    let hc = Arc::clone(&hv_count);
    let hv_backend = DefaultBackend::new(
        counting_caps("mock-hypervisor", IsolationLevel::Hypervisor),
        move |_req: ExecRequest| {
            let c = Arc::clone(&hc);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(ExecResult {
                    success: true,
                    exit_code: Some(0),
                    stdout: "from-hypervisor".to_string(),
                    stderr: String::new(),
                    duration: Duration::from_millis(1),
                    timed_out: false,
                })
            }
        },
    );
    reg.register(Arc::new(hv_backend));

    // Test selection at each level
    // PolicyOnly -> apxm-process (real backend, lowest qualifying)
    let selected = reg.select(IsolationLevel::PolicyOnly).unwrap();
    assert_eq!(selected.capabilities().name, backend_names::PROCESS);

    // Container -> mock-container (exact match)
    let selected = reg.select(IsolationLevel::Container).unwrap();
    assert_eq!(selected.capabilities().name, "mock-container");

    // OsLevel -> mock-container (lowest that meets >= OsLevel)
    let selected = reg.select(IsolationLevel::OsLevel).unwrap();
    assert_eq!(selected.capabilities().name, "mock-container");

    // Hypervisor -> mock-hypervisor
    let selected = reg.select(IsolationLevel::Hypervisor).unwrap();
    assert_eq!(selected.capabilities().name, "mock-hypervisor");

    // Execute through the container backend to verify the mock works
    let container = reg.select(IsolationLevel::Container).unwrap();
    let ctx = container.create_session().await.unwrap();
    let result = container
        .execute(
            &ctx,
            ExecRequest {
                program: "test".to_string(),
                timeout: Duration::from_secs(1),
                ..ExecRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.stdout, "from-container");
    assert_eq!(container_count.load(Ordering::SeqCst), 1);
    assert_eq!(hv_count.load(Ordering::SeqCst), 0);
    container.destroy_session(ctx).await.unwrap();

    // Execute through the real ProcessSandboxBackend
    let process = reg.select(IsolationLevel::PolicyOnly).unwrap();
    let ctx = process.create_session().await.unwrap();
    let result = process
        .execute(
            &ctx,
            ExecRequest {
                program: "echo".to_string(),
                args: vec!["real-process".to_string()],
                timeout: Duration::from_secs(5),
                ..ExecRequest::default()
            },
        )
        .await
        .unwrap();
    assert!(result.success);
    assert_eq!(result.stdout.trim(), "real-process");
    process.destroy_session(ctx).await.unwrap();
}

// ---------------------------------------------------------------------------
// Tests: Runtime integration (graph execution with INV node)
// ---------------------------------------------------------------------------

/// Test that a runtime with INV nodes and a sandbox registry correctly routes
/// process-spawning capabilities through the sandbox backend.
///
/// This verifies the full integration: CapabilitySystem detects that
/// BashCapability returns an ExecRequest from `to_exec_request()` and routes
/// execution through the registered SandboxBackend instead of calling
/// `execute()` directly.
#[tokio::test]
async fn runtime_inv_node_with_sandbox_registry_configured() {
    use apxm_core::types::execution::{ExecutionDag, Node, NodeMetadata};
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::values::Value;
    use apxm_runtime::capability::builtins::BashCapability;
    use apxm_runtime::{Runtime, RuntimeConfig};
    use std::collections::HashMap;

    // Create runtime
    let config = RuntimeConfig::in_memory();
    let mut runtime = Runtime::new(config).await.unwrap();

    // Register BashCapability from apxm-tools (the production capability)
    runtime
        .capability_system()
        .register(Arc::new(BashCapability::new()))
        .unwrap();

    // Set up a sandbox registry with a counting backend to detect usage
    let call_count = Arc::new(AtomicU32::new(0));
    let cc = Arc::clone(&call_count);
    let mut registry = SandboxRegistry::new();
    registry.register(Arc::new(DefaultBackend::new(
        SandboxCapabilities {
            isolation_level: IsolationLevel::OsLevel,
            supports_filesystem_restriction: false,
            supports_network_restriction: false,
            supports_syscall_filtering: false,
            supports_resource_limits: false,
            name: "tracking-backend".to_string(),
            version: "test".to_string(),
        },
        move |_req: ExecRequest| {
            let c = Arc::clone(&cc);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(ExecResult {
                    success: true,
                    exit_code: Some(0),
                    stdout: "from-tracking-backend".to_string(),
                    stderr: String::new(),
                    duration: Duration::from_millis(1),
                    timed_out: false,
                })
            }
        },
    )));
    runtime.set_sandbox_registry(Arc::new(registry));

    // Create a DAG with an INV node that calls the bash capability
    let mut inv_node = Node {
        id: 1,
        op_type: AISOperationType::InvTool,
        attributes: HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![100],
        metadata: NodeMetadata::default(),
    };
    inv_node
        .attributes
        .insert("capability".to_string(), Value::String("bash".to_string()));
    inv_node.attributes.insert(
        "params_json".to_string(),
        Value::String(r#"{"command": "echo hello from sandbox test"}"#.to_string()),
    );

    let dag = ExecutionDag {
        nodes: vec![inv_node],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![1],
        metadata: Default::default(),
    };

    // Execute the graph
    let result = runtime.execute(dag).await.unwrap();

    // The graph should execute successfully
    assert_eq!(result.stats.executed_nodes, 1);
    assert_eq!(result.stats.failed_nodes, 0);

    // The INV node should produce the tracking backend's mock output
    // (routed through sandbox, NOT direct execution)
    let output = result
        .results
        .get(&100)
        .expect("output token 100 should exist");
    assert_eq!(
        output.as_string().map(|s| s.as_str()),
        Some("from-tracking-backend")
    );

    // The tracking backend SHOULD have been called — the integration gap
    // is now closed: BashCapability.to_exec_request() returns an ExecRequest,
    // and CapabilitySystem routes it through the sandbox backend.
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "The sandbox backend's execute() should have been called exactly once \
         during INV execution. BashCapability returns an ExecRequest from \
         to_exec_request(), which CapabilitySystem routes through the sandbox."
    );
}

/// Test that the sandbox registry is properly propagated through the
/// Runtime -> ExecutionContext -> child ExecutionContext chain.
#[tokio::test]
async fn sandbox_registry_propagates_to_child_contexts() {
    use apxm_backends::LLMRegistry;
    use apxm_runtime::aam::Aam;
    use apxm_runtime::capability::CapabilitySystem;
    use apxm_runtime::executor::ExecutionContext;
    use apxm_runtime::memory::{MemoryConfig, MemorySystem};

    let memory = Arc::new(
        MemorySystem::new(MemoryConfig::in_memory_ltm())
            .await
            .unwrap(),
    );
    let llm_registry = Arc::new(LLMRegistry::new());
    let capability_system = Arc::new(CapabilitySystem::new());

    // Create a registry with a uniquely named backend
    let mut registry = SandboxRegistry::new();
    registry.register(Arc::new(DefaultBackend::new(
        SandboxCapabilities {
            isolation_level: IsolationLevel::Container,
            supports_filesystem_restriction: true,
            supports_network_restriction: true,
            supports_syscall_filtering: false,
            supports_resource_limits: false,
            name: "propagation-test-backend".to_string(),
            version: "test".to_string(),
        },
        |_req| async {
            Ok(ExecResult {
                success: true,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_millis(1),
                timed_out: false,
            })
        },
    )));

    let registry = Arc::new(registry);

    let ctx = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
        .with_sandbox_registry(Arc::clone(&registry));

    // Verify the parent context has the registry
    assert_eq!(ctx.sandbox_registry.len(), 1);
    assert_eq!(
        ctx.sandbox_registry.list()[0].name,
        "propagation-test-backend"
    );

    // Create a child context and verify registry propagation
    let child = ctx.child();
    assert_eq!(child.sandbox_registry.len(), 1);
    assert_eq!(
        child.sandbox_registry.list()[0].name,
        "propagation-test-backend"
    );

    // Verify both point to the same Arc
    assert!(Arc::ptr_eq(&ctx.sandbox_registry, &child.sandbox_registry));
}

/// Real bubblewrap confinement: the host root is readable (over-satisfying the
/// read grant), writes are confined to the working dir, and writes elsewhere
/// (read-only root) are blocked. Skipped when `bwrap` is not installed.
#[tokio::test]
async fn bubblewrap_confines_writes_to_workdir_only() {
    use apxm_driver::runtime::sandbox::configure_sandbox_registry;
    let registry = configure_sandbox_registry();
    let backend = match registry.select(IsolationLevel::OsLevel) {
        Ok(b) if b.capabilities().name == backend_names::BUBBLEWRAP => b,
        _ => {
            eprintln!("SKIP: bubblewrap backend unavailable (bwrap not installed)");
            return;
        }
    };

    let workdir = std::env::temp_dir().join(format!("apxm-bwrap-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).unwrap();

    let session = backend.create_session().await.unwrap();
    let script = "echo confined; \
                  cat /etc/hostname >/dev/null 2>&1 && echo readok; \
                  (echo x > ./in_workdir.txt) && echo wrote_workdir; \
                  (echo x > /etc/apxm_should_fail) 2>/dev/null && echo WROTE_ETC || echo etc_blocked";
    let request = ExecRequest {
        min_isolation: IsolationLevel::OsLevel,
        program: "sh".to_string(),
        args: vec!["-c".to_string(), script.to_string()],
        working_dir: Some(workdir.clone()),
        read_paths: vec![workdir.clone()],
        write_paths: vec![workdir.clone()],
        needs_network: false,
        ..ExecRequest::default()
    };
    let result = backend.execute(&session, request).await.unwrap();
    backend.destroy_session(session).await.unwrap();
    let _ = std::fs::remove_dir_all(&workdir);

    assert!(
        result.success,
        "sandboxed command failed: stdout={} stderr={}",
        result.stdout, result.stderr
    );
    assert!(result.stdout.contains("confined"), "command did not run");
    assert!(result.stdout.contains("readok"), "host root should be readable");
    assert!(
        result.stdout.contains("wrote_workdir"),
        "working dir should be writable"
    );
    assert!(
        result.stdout.contains("etc_blocked") && !result.stdout.contains("WROTE_ETC"),
        "writes outside the working dir must be blocked (read-only root)"
    );
}

/// The ACP path: `wrap_command` must yield a confined process that still has
/// working live stdio (bwrap forwards stdin/stdout to the inner child). Mirrors
/// how AcpSession attaches pipes. Skipped when `bwrap` is not installed.
#[tokio::test]
async fn wrap_command_runs_confined_with_live_stdio() {
    use apxm_driver::runtime::sandbox::configure_sandbox_registry;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let registry = configure_sandbox_registry();
    let backend = match registry.select(IsolationLevel::OsLevel) {
        Ok(b) if b.capabilities().name == backend_names::BUBBLEWRAP => b,
        _ => {
            eprintln!("SKIP: bubblewrap backend unavailable (bwrap not installed)");
            return;
        }
    };

    let workdir = std::env::temp_dir().join(format!("apxm-wrap-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).unwrap();

    let (program, args) = backend.wrap_command(
        "sh",
        &[
            "-c".to_string(),
            "read line; echo \"got:$line\"; \
             (echo x > /etc/apxm_nope) 2>/dev/null && echo WROTE || echo blocked"
                .to_string(),
        ],
        &workdir,
        false,
    );
    assert_eq!(program, "bwrap", "wrap_command should route through bwrap");

    let mut child = Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"hello\n")
        .await
        .unwrap();
    let output = child.wait_with_output().await.unwrap();
    let _ = std::fs::remove_dir_all(&workdir);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("got:hello"), "stdio not forwarded: {stdout}");
    assert!(
        stdout.contains("blocked") && !stdout.contains("WROTE"),
        "wrapped process not confined: {stdout}"
    );
}
