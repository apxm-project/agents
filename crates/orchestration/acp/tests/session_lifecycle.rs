//! Session lifecycle tests: spawn -> (prompt) -> clean exit.
//!
//! `apxm-acp` is the live `SPAWN_AGENT` backend (see
//! `docs/plans/runtime-simplification.md` RT-4) but previously shipped with
//! no tests at all. These exercise the real subprocess spawn / JSON-RPC
//! handshake / close path end to end against a fixture agent.

mod support;

use std::sync::Arc;

use apxm_acp::AcpSession;
use apxm_acp::registry::PermissionMode;
use apxm_acp::reverse::CapabilityReverseHandler;
use apxm_runtime::CapabilitySystem;

#[tokio::test]
async fn spawn_establishes_session_and_agent_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let script = support::write_fixture(dir.path(), "agent.py", support::COOPERATIVE_AGENT);
    let profile = support::fixture_profile(&script);
    let aam = support::default_aam_context();

    let session = AcpSession::spawn("fixture", &profile, dir.path(), &aam, None)
        .await
        .expect("spawn should succeed against a cooperative fixture agent");

    assert!(!session.session_id().is_empty());
    assert_eq!(session.agent_session_id(), Some("agent-session-fixture"));
    assert_eq!(session.profile_name(), "fixture");
    assert_eq!(session.turn_count(), 0);

    session.close().await;
}

#[tokio::test]
async fn prompt_round_trip_collects_streamed_text_and_increments_turn_count() {
    let dir = tempfile::tempdir().unwrap();
    let script = support::write_fixture(dir.path(), "agent.py", support::COOPERATIVE_AGENT);
    let profile = support::fixture_profile(&script);
    let aam = support::default_aam_context();

    let mut session = AcpSession::spawn("fixture", &profile, dir.path(), &aam, None)
        .await
        .expect("spawn");

    let capability_system = Arc::new(CapabilitySystem::new());
    let handler = CapabilityReverseHandler::new(capability_system, PermissionMode::DenyAll);

    let result = session
        .prompt("hi there", &handler)
        .await
        .expect("prompt should round-trip through the fixture");

    assert_eq!(result.text, "hello from fixture");
    assert_eq!(result.stop_reason, "end_turn");
    assert_eq!(session.turn_count(), 1);

    session.close().await;
}

#[tokio::test]
async fn close_lets_a_cooperative_agent_exit_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let script = support::write_fixture(dir.path(), "agent.py", support::COOPERATIVE_AGENT);
    let profile = support::fixture_profile(&script);
    let aam = support::default_aam_context();

    let session = AcpSession::spawn("fixture", &profile, dir.path(), &aam, None)
        .await
        .expect("spawn");
    let pid = session.pid().expect("child should have a pid while running");
    assert!(support::process_alive(pid), "child should be running before close");

    session.close().await;

    assert!(
        !support::process_alive(pid),
        "cooperative agent should have exited (via EOF) by the time close() returns"
    );
}

#[tokio::test]
async fn spawn_fails_cleanly_for_a_nonexistent_program() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = support::fixture_profile(&dir.path().join("agent.py"));
    profile.command = "apxm-acp-test-definitely-not-a-real-binary".to_string();
    let aam = support::default_aam_context();

    match AcpSession::spawn("missing", &profile, dir.path(), &aam, None).await {
        Err(apxm_acp::AcpError::Spawn { agent, .. }) => assert_eq!(agent, "missing"),
        Err(other) => panic!("expected AcpError::Spawn, got {other:?}"),
        Ok(_) => panic!("spawning a nonexistent program must fail, not hang"),
    }
}
