use std::collections::BTreeMap;
use std::sync::Arc;

use apxm_acp::{AcpSession, AgentProfile, PermissionMode};
use apxm_core::types::aam::AamContext;

fn mock_profile() -> AgentProfile {
    let mock_script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("mock_agent.sh");

    AgentProfile {
        command: format!("bash {}", mock_script.display()),
        close_grace_ms: 100,
        session_create_timeout_ms: 5_000,
        permission_mode: PermissionMode::ApproveAll,
        env: BTreeMap::new(),
        default_mode: None,
        default_model: None,
        system_prompt: None,
        skip_preamble: false,
        capabilities: Vec::new(),
    }
}

/// Check that jq is available (required by mock_agent.sh).
fn jq_available() -> bool {
    std::process::Command::new("jq")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn full_session_lifecycle() {
    if !jq_available() {
        eprintln!("SKIP: jq not installed");
        return;
    }

    let profile = mock_profile();
    let cwd = std::env::current_dir().unwrap();

    // Spawn session
    let aam_ctx = AamContext::default();
    let mut session = AcpSession::spawn("mock", &profile, &cwd, &aam_ctx)
        .await
        .expect("should spawn mock agent");

    assert!(!session.session_id().is_empty());
    assert_eq!(session.agent_session_id(), Some("mock-session-1"));
    assert_eq!(session.turn_count(), 0);

    // Create a reverse handler (approve-all, uses a fresh CapabilitySystem)
    let cap_sys = Arc::new(apxm_runtime::CapabilitySystem::new());
    let handler =
        apxm_acp::reverse::CapabilityReverseHandler::new(cap_sys, PermissionMode::ApproveAll);

    // Send a prompt
    let result = session
        .prompt("What is 2+2?", &handler)
        .await
        .expect("prompt should succeed");

    assert_eq!(result.text, "Hello from mock");
    assert_eq!(result.stop_reason, "end_turn");
    assert_eq!(session.turn_count(), 1);

    // Close gracefully
    session.close().await;
}


#[tokio::test]
async fn multi_turn_on_same_session() {
    if !jq_available() {
        eprintln!("SKIP: jq not installed");
        return;
    }

    let profile = mock_profile();
    let cwd = std::env::current_dir().unwrap();

    let aam_ctx = AamContext::default();
    let mut session = AcpSession::spawn("mock", &profile, &cwd, &aam_ctx)
        .await
        .expect("should spawn");

    let cap_sys = Arc::new(apxm_runtime::CapabilitySystem::new());

    // Turn 1
    let handler = apxm_acp::reverse::CapabilityReverseHandler::new(
        Arc::clone(&cap_sys),
        PermissionMode::ApproveAll,
    );
    let r1 = session.prompt("Turn 1", &handler).await.expect("turn 1");
    assert_eq!(r1.text, "Hello from mock");
    assert_eq!(session.turn_count(), 1);

    // Turn 2
    let handler = apxm_acp::reverse::CapabilityReverseHandler::new(
        Arc::clone(&cap_sys),
        PermissionMode::ApproveAll,
    );
    let r2 = session.prompt("Turn 2", &handler).await.expect("turn 2");
    assert_eq!(r2.text, "Hello from mock");
    assert_eq!(session.turn_count(), 2);

    session.close().await;
}
