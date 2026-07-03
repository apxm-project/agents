//! Kill / cleanup tests: a spawned agent that refuses to exit on EOF must
//! still be terminated — first via `AcpSession::close` (SIGTERM -> SIGKILL),
//! and, if the session is dropped without an explicit `close()`, via the
//! best-effort `Drop` impl. Either way, no zombie/orphan should remain.

mod support;

use std::time::Duration;

use apxm_acp::AcpSession;

#[tokio::test]
async fn close_terminates_a_stubborn_agent_that_ignores_eof() {
    let dir = tempfile::tempdir().unwrap();
    let script = support::write_fixture(dir.path(), "agent.py", support::STUBBORN_AGENT);
    let profile = support::fixture_profile(&script);
    let aam = support::default_aam_context();

    let session = AcpSession::spawn("stubborn", &profile, dir.path(), &aam, None)
        .await
        .expect("spawn");
    let pid = session.pid().expect("running child has a pid");
    assert!(support::process_alive(pid));

    // close() escalates stdin-close -> SIGTERM -> SIGKILL internally and
    // only returns once the child has been wait()'d (no zombie left behind).
    session.close().await;

    assert!(
        !support::process_alive(pid),
        "stubborn agent must be forcibly terminated by close()"
    );
}

#[tokio::test]
async fn drop_kills_the_child_without_an_explicit_close() {
    let dir = tempfile::tempdir().unwrap();
    let script = support::write_fixture(dir.path(), "agent.py", support::STUBBORN_AGENT);
    let profile = support::fixture_profile(&script);
    let aam = support::default_aam_context();

    let pid = {
        let session = AcpSession::spawn("stubborn", &profile, dir.path(), &aam, None)
            .await
            .expect("spawn");
        let pid = session.pid().expect("running child has a pid");
        assert!(support::process_alive(pid));
        pid
        // `session` drops here without calling close().
    };

    // Drop only issues a best-effort SIGKILL synchronously; give the OS /
    // tokio's SIGCHLD reaper a moment to actually reclaim the process.
    let died = support::wait_for_exit(pid, Duration::from_secs(5)).await;
    assert!(died, "Drop must SIGKILL the child even without close()");
}
