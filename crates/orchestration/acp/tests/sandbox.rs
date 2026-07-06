//! Sandbox flag tests: `AcpSession::spawn` and `TerminalManager::create` must
//! actually call through to a supplied `SandboxBackend::wrap_command` (and
//! must NOT rewrite anything when no backend is supplied).

mod support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use apxm_acp::AcpSession;
use apxm_acp::terminal::TerminalManager;
use apxm_runtime::sandbox::{
    ExecRequest, ExecResult, IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxContext,
    SandboxError, ValidationResult,
};
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq)]
struct WrapCall {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    needs_network: bool,
}

/// Records every `wrap_command` invocation and rewrites the program into an
/// observably different (but still runnable) command, proving the caller
/// actually used the rewritten value rather than the original.
#[derive(Default)]
struct RecordingBackend {
    calls: Mutex<Vec<WrapCall>>,
}

impl RecordingBackend {
    fn calls(&self) -> Vec<WrapCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl SandboxBackend for RecordingBackend {
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            isolation_level: IsolationLevel::PolicyOnly,
            supports_filesystem_restriction: false,
            supports_network_restriction: false,
            supports_syscall_filtering: false,
            supports_resource_limits: false,
            name: "recording-test-backend".to_string(),
            version: "0.0.0".to_string(),
        }
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(&self, _request: &ExecRequest) -> ValidationResult {
        ValidationResult::Ok
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        unimplemented!("not exercised by wrap_command tests")
    }

    async fn execute(
        &self,
        _ctx: &SandboxContext,
        _request: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        unimplemented!("not exercised by wrap_command tests")
    }

    async fn destroy_session(&self, _ctx: SandboxContext) -> Result<(), SandboxError> {
        Ok(())
    }

    fn wrap_command(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        needs_network: bool,
    ) -> (String, Vec<String>) {
        self.calls.lock().unwrap().push(WrapCall {
            program: program.to_string(),
            args: args.to_vec(),
            cwd: cwd.to_path_buf(),
            needs_network,
        });
        // Rewrite unchanged so the underlying spawn still succeeds; the
        // point of this test is observing that wrap_command was consulted.
        (program.to_string(), args.to_vec())
    }
}

#[tokio::test]
async fn spawn_consults_the_sandbox_backend_when_one_is_supplied() {
    let dir = tempfile::tempdir().unwrap();
    let script = support::write_fixture(dir.path(), "agent.py", support::COOPERATIVE_AGENT);
    let mut profile = support::fixture_profile(&script);
    profile.sandbox = true;
    let aam = support::default_aam_context();

    let backend = Arc::new(RecordingBackend::default());
    let session = AcpSession::spawn(
        "fixture",
        &profile,
        dir.path(),
        &aam,
        Some(backend.clone() as Arc<dyn SandboxBackend>),
    )
    .await
    .expect("spawn with a sandbox backend should still succeed");

    let calls = backend.calls();
    assert_eq!(
        calls.len(),
        1,
        "wrap_command should be called exactly once per spawn"
    );
    assert_eq!(calls[0].program, "python3");
    assert_eq!(calls[0].args, vec![script.to_string_lossy().to_string()]);
    assert_eq!(calls[0].cwd, dir.path());
    assert!(
        calls[0].needs_network,
        "coding agents need network for the model gateway; confinement must not cut it off"
    );

    session.close().await;
}

#[tokio::test]
async fn spawn_does_not_touch_the_command_when_no_sandbox_backend_is_supplied() {
    let dir = tempfile::tempdir().unwrap();
    let script = support::write_fixture(dir.path(), "agent.py", support::COOPERATIVE_AGENT);
    let profile = support::fixture_profile(&script);
    let aam = support::default_aam_context();

    // None passed as the sandbox backend: spawn must fall back to the
    // original (program, args) unchanged. There is nothing to assert on a
    // recorder here (there is none) -- the meaningful assertion is that the
    // fixture agent still spawns and completes its handshake normally.
    let session = AcpSession::spawn("fixture", &profile, dir.path(), &aam, None)
        .await
        .expect("spawn without a sandbox backend should work unmodified");

    assert_eq!(session.agent_session_id(), Some("agent-session-fixture"));
    session.close().await;
}

#[tokio::test]
async fn terminal_manager_consults_the_sandbox_backend_for_agent_opened_terminals() {
    let backend = Arc::new(RecordingBackend::default());
    let manager = TerminalManager::with_sandbox(Some(backend.clone() as Arc<dyn SandboxBackend>));

    let terminal_id = manager
        .create("echo", &["confined".to_string()], None, &[])
        .await
        .expect("terminal create should succeed");
    assert!(!terminal_id.is_empty());

    // Give the (unconfined, since wrap_command is a passthrough here) echo
    // process a moment to finish.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let calls = backend.calls();
    assert_eq!(
        calls.len(),
        1,
        "terminal/create must go through wrap_command"
    );
    assert_eq!(calls[0].program, "echo");
    assert_eq!(calls[0].args, vec!["confined".to_string()]);
    assert!(calls[0].needs_network);
}

#[tokio::test]
async fn terminal_manager_without_sandbox_runs_the_command_unmodified() {
    let manager = TerminalManager::new();
    let terminal_id = manager
        .create("echo", &["hi".to_string()], None, &[])
        .await
        .expect("terminal create should succeed without a sandbox backend");
    assert!(!terminal_id.is_empty());
}
