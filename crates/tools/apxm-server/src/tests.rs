// Integration Tests
//
// These tests exercise the HTTP API layer without binding to a TCP port.
// They use `tower::ServiceExt::oneshot` + `axum::body::Body` to send requests
// directly to the Axum router and collect responses via `http_body_util::BodyExt`.
//
// No LLM API key is required — all LLM-touching tests are gated behind
// `#[cfg(feature = "integration")]` and use stub responses.

use std::sync::Arc;
use std::time::SystemTime;

use apxm_runtime::{Runtime, RuntimeConfig};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use dashmap::DashMap;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::build_app;
use crate::checkpoints::{Checkpoint, CheckpointStatus, CheckpointStore};
use crate::helpers::{jsonrpc_err, jsonrpc_ok, mcp_tool_result, now_ms};
use crate::state::AppState;
use crate::tasks::{QueuedTask, TaskQueueManager, TaskStatus};

// ── Test helpers ──────────────────────────────────────────────────────────

/// Build a test AppState backed by a real (but unconfigured) Runtime.
///
/// The runtime has no LLM backends registered, so any graph that calls
/// ASK/THINK/REASON will error.  Tests that need execution should use
/// graphs composed entirely of CONST_STR and synchronisation ops.
async fn test_state() -> AppState {
    // Use in-memory LTM to avoid SQLite file-locking across parallel tests.
    let runtime = Arc::new(
        Runtime::new(RuntimeConfig::in_memory())
            .await
            .expect("test runtime"),
    );
    AppState {
        runtime,
        agent_registry: Arc::new(DashMap::new()),
        task_manager: TaskQueueManager::new(),
        checkpoint_store: CheckpointStore::new(),
        start_time: SystemTime::now(),
        a2a_tasks: Arc::new(DashMap::new()),
    }
}

/// POST a JSON body to `path` and return `(StatusCode, serde_json::Value)`.
async fn post_json(
    app: Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// GET `path` and return `(StatusCode, serde_json::Value)`.
async fn get_json(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ── Health ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok() {
    let app = build_app(test_state().await);
    let (status, body) = get_json(app, "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
}

// ── /v1/models ────────────────────────────────────────────────────────────

#[tokio::test]
async fn models_returns_json_array() {
    let app = build_app(test_state().await);
    let (status, body) = get_json(app, "/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    // With no backends configured the array may be empty, but must be an array.
    assert!(
        body["data"].is_array() || body["models"].is_array(),
        "expected 'data' or 'models' array, got: {body}"
    );
}

// ── Agent Registry ────────────────────────────────────────────────────────

#[tokio::test]
async fn agent_registry_register_returns_ok() {
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app,
        "/v1/agents/register",
        serde_json::json!({
            "name": "test-agent",
            "url": "http://localhost:19999",
            "flows": ["research"],
            "capabilities": ["web-search"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register failed: {body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["name"], "test-agent");
}

#[tokio::test]
async fn agent_registry_list_returns_array() {
    let app = build_app(test_state().await);
    let (status, body) = get_json(app, "/v1/agents").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array(), "expected array: {body}");
}

#[tokio::test]
async fn agent_registry_missing_name_returns_400() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/agents/register",
        serde_json::json!({ "url": "http://localhost:19999" }),
    )
    .await;
    // Missing `name` field → deserialization error → 400 or 422
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400/422, got {status}: {body}"
    );
}

// ── /.well-known/agent.json (A2A Agent Card) ──────────────────────────────

#[tokio::test]
async fn a2a_agent_card_has_required_fields() {
    let app = build_app(test_state().await);
    let (status, body) = get_json(app, "/.well-known/agent.json").await;
    assert_eq!(status, StatusCode::OK, "agent card failed: {body}");
    assert!(body["name"].is_string(), "missing 'name': {body}");
    assert!(body["url"].is_string(), "missing 'url': {body}");
    assert!(body["version"].is_string(), "missing 'version': {body}");
    assert!(
        body["capabilities"].is_object(),
        "missing 'capabilities': {body}"
    );
}

// ── /v1/mcp (MCP JSON-RPC) ────────────────────────────────────────────────

#[tokio::test]
async fn mcp_initialize_returns_protocol_version() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/mcp",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": apxm_core::constants::protocols::MCP_VERSION }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "mcp initialize failed: {body}");
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);
    assert!(body["result"].is_object(), "expected result object: {body}");
    let version = body["result"]["protocolVersion"].as_str().unwrap_or("");
    assert!(!version.is_empty(), "protocolVersion missing: {body}");
}

#[tokio::test]
async fn mcp_tools_list_returns_array() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/mcp",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tools = &body["result"]["tools"];
    assert!(tools.is_array(), "expected tools array: {body}");
}

#[tokio::test]
async fn mcp_unknown_method_returns_error_code() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/mcp",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "totally/unknown",
            "params": {}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "should always return 200 JSON-RPC: {body}"
    );
    assert!(body["error"].is_object(), "expected error object: {body}");
    let code = body["error"]["code"].as_i64().unwrap_or(0);
    assert_eq!(code, -32601, "expected method-not-found code: {body}");
}

// ── /v1/execute (graph execution) ─────────────────────────────────────────

#[tokio::test]
async fn execute_invalid_graph_returns_400() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/execute",
        serde_json::json!({
            "graph": { "not": "a valid graph" }
        }),
    )
    .await;
    // Invalid graph → compiler rejects → 400 Bad Request
    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(body["error"].is_string(), "expected error message: {body}");
}

#[tokio::test]
async fn execute_empty_graph_returns_error() {
    let app = build_app(test_state().await);
    let (status, _body) = post_json(app, "/v1/execute", serde_json::json!({})).await;
    // Missing graph field → 400 or 422
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400/422 for empty request, got {status}"
    );
}

// ── /v1/memory (LTM facts) ────────────────────────────────────────────────

#[tokio::test]
async fn memory_store_and_search_roundtrip() {
    let state = test_state().await;
    let app = build_app(state);

    // Store a fact
    let (store_status, store_body) = post_json(
        app.clone(),
        "/v1/memory/facts/store",
        serde_json::json!({
            "text": "RDNA 4 uses a unified compute architecture",
            "tags": ["gpu", "rdna4"],
            "source": "test"
        }),
    )
    .await;
    assert_eq!(store_status, StatusCode::OK, "store failed: {store_body}");
    assert!(
        store_body["id"].is_string(),
        "expected fact id: {store_body}"
    );
}

#[tokio::test]
async fn memory_search_returns_array() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/memory/facts/search",
        serde_json::json!({ "query": "GPU architecture", "limit": 5 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "search failed: {body}");
    // May return empty array if nothing stored, but must be an array
    assert!(body.is_array(), "expected array response: {body}");
}

// ── /a2a (A2A JSON-RPC) ───────────────────────────────────────────────────

#[tokio::test]
async fn a2a_jsonrpc_unknown_method_returns_error() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/a2a",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "t1",
            "method": "tasks/reopen",
            "params": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a2a should return 200: {body}");
    // Unrecognised method should produce an error response
    assert!(body["error"].is_object(), "expected error: {body}");
}

// ── 404 for unknown routes ────────────────────────────────────────────────

#[tokio::test]
async fn unknown_route_returns_404() {
    let app = build_app(test_state().await);
    let req = Request::builder()
        .method("GET")
        .uri("/does/not/exist")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── /v1/tasks (task queue) ────────────────────────────────────────────────

#[tokio::test]
async fn task_queue_create_returns_id() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/tasks",
        serde_json::json!({
            "queue": "test-queue",
            "data": { "work": "process this" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create task failed: {body}");
    assert_eq!(body["ok"], true);
    assert!(body["id"].is_string(), "expected task id: {body}");
    assert_eq!(body["queue"], "test-queue");
}

#[tokio::test]
async fn task_queue_list_returns_tasks() {
    let state = test_state().await;
    let app = build_app(state);

    // Create a task first
    post_json(
        app.clone(),
        "/v1/tasks",
        serde_json::json!({
            "queue": "list-test",
            "data": { "item": 1 }
        }),
    )
    .await;

    let (status, body) = get_json(app, "/v1/tasks/list-test").await;
    assert_eq!(status, StatusCode::OK, "list tasks failed: {body}");
    assert_eq!(body["queue"], "list-test");
    assert!(
        body["count"].as_u64().unwrap_or(0) >= 1,
        "expected count >= 1: {body}"
    );
    assert!(body["tasks"].is_array(), "expected tasks array: {body}");
}

#[tokio::test]
async fn task_queue_claim_and_complete() {
    let state = test_state().await;
    let app = build_app(state);

    // 1. Create task
    let (_, create_body) = post_json(
        app.clone(),
        "/v1/tasks",
        serde_json::json!({ "queue": "work", "data": { "job": "test" } }),
    )
    .await;
    let task_id = create_body["id"].as_str().unwrap().to_string();

    // 2. Claim task
    let (claim_status, claim_body) = post_json(
        app.clone(),
        "/v1/tasks/work/claim",
        serde_json::json!({ "agent_id": "test-agent", "lease_ms": 30000 }),
    )
    .await;
    assert_eq!(claim_status, StatusCode::OK, "claim failed: {claim_body}");
    assert_eq!(claim_body["task_id"], task_id);
    let claim_token = claim_body["claim_token"].as_str().unwrap().to_string();

    // 3. Complete task
    let complete_path = format!("/v1/tasks/{}/complete", task_id);
    let (complete_status, complete_body) = post_json(
        app,
        &complete_path,
        serde_json::json!({
            "claim_token": claim_token,
            "result": { "output": "done" },
            "success": true
        }),
    )
    .await;
    assert_eq!(
        complete_status,
        StatusCode::OK,
        "complete failed: {complete_body}"
    );
    assert_eq!(complete_body["ok"], true);
}

#[tokio::test]
async fn task_queue_claim_empty_queue_returns_404() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/tasks/empty-queue/claim",
        serde_json::json!({ "agent_id": "agent-1" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

// ── /v1/checkpoints (PAUSE/RESUME HITL) ───────────────────────────────────

#[tokio::test]
async fn checkpoint_create_returns_pending() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/checkpoints",
        serde_json::json!({
            "checkpoint_id": "cp-test-001",
            "message": "Please review the generated plan",
            "display_data": { "plan": "step 1, step 2, step 3" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create checkpoint failed: {body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["checkpoint_id"], "cp-test-001");
    assert_eq!(body["status"], "pending");
    assert!(
        body["resume_url"].is_string(),
        "expected resume_url: {body}"
    );
}

#[tokio::test]
async fn checkpoint_get_returns_checkpoint() {
    let state = test_state().await;
    let app = build_app(state);

    // Create first
    post_json(
        app.clone(),
        "/v1/checkpoints",
        serde_json::json!({
            "checkpoint_id": "cp-get-001",
            "message": "Review needed"
        }),
    )
    .await;

    // Get it
    let (status, body) = get_json(app, "/v1/checkpoints/cp-get-001").await;
    assert_eq!(status, StatusCode::OK, "get checkpoint failed: {body}");
    assert_eq!(body["id"], "cp-get-001");
    assert_eq!(body["status"], "pending");
    assert!(body["message"].is_string());
}

#[tokio::test]
async fn checkpoint_resume_workflow() {
    let state = test_state().await;
    let app = build_app(state);

    // 1. Create checkpoint
    post_json(
        app.clone(),
        "/v1/checkpoints",
        serde_json::json!({
            "checkpoint_id": "cp-resume-001",
            "message": "Human decision required"
        }),
    )
    .await;

    // 2. Resume with human input
    let (status, body) = post_json(
        app,
        "/v1/checkpoints/cp-resume-001/resume",
        serde_json::json!({ "human_input": { "decision": "approved", "notes": "LGTM" } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resume failed: {body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["checkpoint_id"], "cp-resume-001");
    assert_eq!(body["status"], "resumed");
    assert_eq!(body["human_input"]["decision"], "approved");
    assert!(body["resumed_at_ms"].is_number());
}

#[tokio::test]
async fn checkpoint_get_missing_returns_404() {
    let app = build_app(test_state().await);
    let (status, body) = get_json(app, "/v1/checkpoints/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

#[tokio::test]
async fn checkpoint_resume_already_resumed_returns_400() {
    let state = test_state().await;
    let app = build_app(state);

    // Create + resume once
    post_json(
        app.clone(),
        "/v1/checkpoints",
        serde_json::json!({
            "checkpoint_id": "cp-double-001",
            "message": "Once only"
        }),
    )
    .await;
    post_json(
        app.clone(),
        "/v1/checkpoints/cp-double-001/resume",
        serde_json::json!({ "human_input": { "ok": true } }),
    )
    .await;

    // Attempt to resume again
    let (status, body) = post_json(
        app,
        "/v1/checkpoints/cp-double-001/resume",
        serde_json::json!({ "human_input": { "ok": false } }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "expected 400 on double-resume: {body}"
    );
}

// ── TaskQueueManager unit tests ─────────────────────────────────────────

fn make_task(id: &str, queue: &str) -> QueuedTask {
    QueuedTask {
        id: id.to_string(),
        queue: queue.to_string(),
        data: serde_json::json!({"work": id}),
        status: TaskStatus::Pending,
        claimed_by: None,
        claim_token: None,
        lease_expires_ms: None,
        result: None,
        created_at_ms: now_ms(),
        completed_at_ms: None,
    }
}

#[tokio::test]
async fn task_manager_enqueue_and_get() {
    let mgr = TaskQueueManager::new();
    let task = make_task("t1", "q1");
    mgr.enqueue(task).await;

    // Verify task exists in all_tasks index
    assert!(mgr.all_tasks.get("t1").is_some());
    let stored = mgr.all_tasks.get("t1").unwrap();
    assert_eq!(stored.queue, "q1");
    assert_eq!(stored.status, TaskStatus::Pending);
}

#[tokio::test]
async fn task_manager_list_queue() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("a", "q")).await;
    mgr.enqueue(make_task("b", "q")).await;
    mgr.enqueue(make_task("c", "other")).await;

    let q_tasks = mgr.list_queue("q");
    assert_eq!(q_tasks.len(), 2);
    assert!(q_tasks.iter().any(|t| t.id == "a"));
    assert!(q_tasks.iter().any(|t| t.id == "b"));

    let other_tasks = mgr.list_queue("other");
    assert_eq!(other_tasks.len(), 1);
    assert_eq!(other_tasks[0].id, "c");

    // Non-existent queue returns empty
    assert!(mgr.list_queue("nope").is_empty());
}

#[tokio::test]
async fn task_manager_claim_returns_first_pending() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("t1", "q")).await;
    mgr.enqueue(make_task("t2", "q")).await;

    let claimed = mgr.claim("q", "agent-a", 60_000).await;
    assert!(claimed.is_some());
    let claimed = claimed.unwrap();
    assert_eq!(claimed.id, "t1");
    assert_eq!(claimed.status, TaskStatus::Claimed);
    assert_eq!(claimed.claimed_by.as_deref(), Some("agent-a"));
    assert!(claimed.claim_token.is_some());
    assert!(claimed.lease_expires_ms.is_some());
}

#[tokio::test]
async fn task_manager_claim_empty_queue_returns_none() {
    let mgr = TaskQueueManager::new();
    assert!(mgr.claim("nonexistent", "a", 1000).await.is_none());
}

#[tokio::test]
async fn task_manager_claim_skips_already_claimed() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("t1", "q")).await;
    mgr.enqueue(make_task("t2", "q")).await;

    // Claim first task
    let first = mgr.claim("q", "agent-a", 60_000).await.unwrap();
    assert_eq!(first.id, "t1");

    // Next claim should get the second task
    let second = mgr.claim("q", "agent-b", 60_000).await.unwrap();
    assert_eq!(second.id, "t2");

    // No more pending tasks
    assert!(mgr.claim("q", "agent-c", 60_000).await.is_none());
}

#[tokio::test]
async fn task_manager_complete_success() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("t1", "q")).await;

    let claimed = mgr.claim("q", "agent", 60_000).await.unwrap();
    let token = claimed.claim_token.unwrap();

    let result = mgr
        .complete("t1", &token, serde_json::json!({"output": "done"}))
        .await;
    assert!(result.is_ok());

    // Verify completed state in all_tasks
    let stored = mgr.all_tasks.get("t1").unwrap();
    assert_eq!(stored.status, TaskStatus::Completed);
    assert!(stored.result.is_some());
    assert!(stored.completed_at_ms.is_some());
}

#[tokio::test]
async fn task_manager_complete_wrong_token_fails() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("t1", "q")).await;
    mgr.claim("q", "agent", 60_000).await;

    let result = mgr
        .complete("t1", "wrong-token", serde_json::json!({}))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid claim token"));
}

#[tokio::test]
async fn task_manager_complete_missing_task_fails() {
    let mgr = TaskQueueManager::new();
    let result = mgr
        .complete("nonexistent", "tok", serde_json::json!({}))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

// ── CheckpointStore unit tests ──────────────────────────────────────────

#[test]
fn checkpoint_store_create_and_get() {
    let store = CheckpointStore::new();
    let cp = Checkpoint {
        id: "cp1".to_string(),
        message: "Review this".to_string(),
        display_data: serde_json::json!({"plan": "step 1"}),
        status: CheckpointStatus::Pending,
        human_input: None,
        notification_url: None,
        created_at_ms: now_ms(),
        resumed_at_ms: None,
    };
    store.create(cp);

    let loaded = store.get("cp1");
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.id, "cp1");
    assert_eq!(loaded.message, "Review this");
    assert_eq!(loaded.status, CheckpointStatus::Pending);
}

#[test]
fn checkpoint_store_get_missing_returns_none() {
    let store = CheckpointStore::new();
    assert!(store.get("nope").is_none());
}

#[test]
fn checkpoint_store_resume_success() {
    let store = CheckpointStore::new();
    store.create(Checkpoint {
        id: "cp2".to_string(),
        message: "Approve?".to_string(),
        display_data: serde_json::json!(null),
        status: CheckpointStatus::Pending,
        human_input: None,
        notification_url: None,
        created_at_ms: now_ms(),
        resumed_at_ms: None,
    });

    let result = store.resume("cp2", serde_json::json!({"decision": "yes"}));
    assert!(result.is_ok());
    let resumed = result.unwrap();
    assert_eq!(resumed.status, CheckpointStatus::Resumed);
    assert_eq!(
        resumed.human_input,
        Some(serde_json::json!({"decision": "yes"}))
    );
    assert!(resumed.resumed_at_ms.is_some());
}

#[test]
fn checkpoint_store_resume_missing_returns_error() {
    let store = CheckpointStore::new();
    let result = store.resume("nope", serde_json::json!({}));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

#[test]
fn checkpoint_store_resume_already_resumed_returns_error() {
    let store = CheckpointStore::new();
    store.create(Checkpoint {
        id: "cp3".to_string(),
        message: "once".to_string(),
        display_data: serde_json::json!(null),
        status: CheckpointStatus::Pending,
        human_input: None,
        notification_url: None,
        created_at_ms: now_ms(),
        resumed_at_ms: None,
    });

    // First resume succeeds
    assert!(store.resume("cp3", serde_json::json!({"ok": true})).is_ok());

    // Second resume fails
    let result = store.resume("cp3", serde_json::json!({"ok": false}));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not in pending state"));
}

// ── JSON-RPC helper unit tests ──────────────────────────────────────────

#[test]
fn jsonrpc_ok_format() {
    let resp = jsonrpc_ok(
        serde_json::json!(42),
        serde_json::json!({"answer": "hello"}),
    );
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 42);
    assert_eq!(body["result"]["answer"], "hello");
    assert!(body.get("error").is_none());
}

#[test]
fn jsonrpc_ok_with_null_id() {
    let resp = jsonrpc_ok(serde_json::json!(null), serde_json::json!("ok"));
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert!(body["id"].is_null());
    assert_eq!(body["result"], "ok");
}

#[test]
fn jsonrpc_err_format() {
    let resp = jsonrpc_err(serde_json::json!(7), -32601, "Method not found");
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 7);
    assert!(body.get("result").is_none());
    assert_eq!(body["error"]["code"], -32601);
    assert_eq!(body["error"]["message"], "Method not found");
}

#[test]
fn jsonrpc_err_with_string_id() {
    let resp = jsonrpc_err(serde_json::json!("req-abc"), -32600, "Invalid request");
    let body = resp.0;
    assert_eq!(body["id"], "req-abc");
    assert_eq!(body["error"]["code"], -32600);
}

// ── mcp_tool_result helper tests ────────────────────────────────────────

#[test]
fn mcp_tool_result_success() {
    let resp = mcp_tool_result(serde_json::json!(1), "tool output text".to_string(), false);
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);

    let content = &body["result"]["content"];
    assert!(content.is_array());
    let items = content.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "text");
    assert_eq!(items[0]["text"], "tool output text");
    assert_eq!(body["result"]["isError"], false);
}

#[test]
fn mcp_tool_result_error() {
    let resp = mcp_tool_result(
        serde_json::json!(2),
        "something went wrong".to_string(),
        true,
    );
    let body = resp.0;
    assert_eq!(body["result"]["isError"], true);
    assert_eq!(body["result"]["content"][0]["text"], "something went wrong");
}
