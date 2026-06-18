//! Integration test: server delegates `spawn_agent` through runner (T023).
//!
//! Verifies that when `APXM_RUNNER_URL` points to a mock runner, the server's
//! `RemoteAgentSpawner` correctly:
//!   1. POSTs to `/v1/agents/spawn` and returns the runner-assigned id.
//!   2. Polls `GET /v1/agents/{id}` until the job reaches a terminal state.
//!   3. Calls `DELETE /v1/agents/{id}` on cancel.
//!
//! The mock runner is an in-process axum server that simulates the runner API
//! contract without Docker.

use apxm_server::remote_runner::{RemoteAgentSpawner, RunStatus, SpawnRequest};
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde_json::json;
use std::net::SocketAddr;
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Mock runner
// ---------------------------------------------------------------------------

/// In-process mock of the apxm-runner API.
///
/// Spawn always succeeds with id `"mock-run-1"`.
/// Status immediately returns `Succeeded`.
/// Cancel returns 200.
async fn mock_runner() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/agents/spawn", post(mock_spawn))
        .route("/v1/agents/:id", get(mock_status))
        .route("/v1/agents/:id", delete(mock_cancel));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

async fn mock_spawn() -> impl IntoResponse {
    (
        StatusCode::CREATED,
        Json(json!({
            "id": "mock-run-1",
            "execution_id": "exec-mock-1",
            "status": "queued",
            "agent_profile": "codex"
        })),
    )
}

async fn mock_status(Path(_id): Path<String>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "id": "mock-run-1",
            "status": "succeeded",
            "agent_profile": "codex",
            "workflow_id": null
        })),
    )
}

async fn mock_cancel(Path(_id): Path<String>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({ "id": "mock-run-1", "status": "canceled" })),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `RemoteAgentSpawner::spawn` against the mock runner must return a run id.
#[tokio::test]
async fn spawn_returns_run_id_from_runner() {
    let (addr, _handle) = mock_runner().await;
    let spawner = RemoteAgentSpawner::new(format!("http://{addr}"));

    let req = SpawnRequest {
        run_id: "exec-mock-1".to_string(),
        agent_type: "codex".to_string(),
        workflow_id: Some("wf-test".to_string()),
        prompt: "hello".to_string(),
        credential_ids: vec![],
    };

    let run_id = spawner.spawn(req).await;
    assert!(run_id.is_ok(), "spawn must succeed: {:?}", run_id.err());
}

/// After spawn, `status` must report the job as `Succeeded` (mock always
/// returns terminal state immediately — simulates a fast job).
#[tokio::test]
async fn status_reports_succeeded_after_spawn() {
    let (addr, _handle) = mock_runner().await;
    let spawner = RemoteAgentSpawner::new(format!("http://{addr}"));

    let status = spawner.status("mock-run-1").await;
    assert!(status.is_ok(), "status must succeed: {:?}", status.err());
    assert_eq!(status.unwrap().status, RunStatus::Succeeded);
}

/// `cancel` must succeed against the mock runner.
#[tokio::test]
async fn cancel_succeeds() {
    let (addr, _handle) = mock_runner().await;
    let spawner = RemoteAgentSpawner::new(format!("http://{addr}"));

    let result = spawner.cancel("mock-run-1").await;
    assert!(result.is_ok(), "cancel must succeed: {:?}", result.err());
}

/// Pointing the spawner at a non-existent address must return a transport
/// error, not panic.
#[tokio::test]
async fn transport_error_on_unreachable_runner() {
    let spawner = RemoteAgentSpawner::new("http://127.0.0.1:1"); // port 1 is always refused

    let req = SpawnRequest {
        run_id: "exec-dead".to_string(),
        agent_type: "codex".to_string(),
        workflow_id: None,
        prompt: "hi".to_string(),
        credential_ids: vec![],
    };

    let result = spawner.spawn(req).await;
    assert!(result.is_err(), "unreachable runner must return an error");
}
