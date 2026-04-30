use super::*;

// ── /v1/tasks (task queue) ────────────────────────────────────────────────

#[tokio::test]
async fn task_queue_create_returns_id() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        routes::TASKS,
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
        routes::TASKS,
        serde_json::json!({
            "queue": "list-test",
            "data": { "item": 1 }
        }),
    )
    .await;

    let (status, body) = get_json(app, &routes::task_queue_path("list-test")).await;
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
        routes::TASKS,
        serde_json::json!({ "queue": "work", "data": { "job": "test" } }),
    )
    .await;
    let task_id = create_body["id"].as_str().unwrap().to_string();

    // 2. Claim task
    let (claim_status, claim_body) = post_json(
        app.clone(),
        &routes::task_claim_path("work"),
        serde_json::json!({ "agent_id": "test-agent", "lease_ms": 30000 }),
    )
    .await;
    assert_eq!(claim_status, StatusCode::OK, "claim failed: {claim_body}");
    assert_eq!(claim_body["task_id"], task_id);
    let claim_token = claim_body["claim_token"].as_str().unwrap().to_string();

    // 3. Complete task
    let complete_path = routes::task_complete_path(&task_id);
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
        &routes::task_claim_path("empty-queue"),
        serde_json::json!({ "agent_id": "agent-1" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
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
