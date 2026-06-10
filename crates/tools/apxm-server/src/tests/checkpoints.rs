use super::*;

// ── /v1/checkpoints (PAUSE/RESUME HITL) ───────────────────────────────────

#[tokio::test]
async fn checkpoint_create_returns_pending() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        routes::CHECKPOINTS,
        serde_json::json!({
            "checkpoint_id": "cp-test-001",
            "message": "Please review the generated workflow",
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
        routes::CHECKPOINTS,
        serde_json::json!({
            "checkpoint_id": "cp-get-001",
            "message": "Review needed"
        }),
    )
    .await;

    // Get it
    let (status, body) = get_json(app, &routes::checkpoint_detail_path("cp-get-001")).await;
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
        routes::CHECKPOINTS,
        serde_json::json!({
            "checkpoint_id": "cp-resume-001",
            "message": "Human decision required"
        }),
    )
    .await;

    // 2. Resume with human input
    let (status, body) = post_json(
        app,
        &routes::checkpoint_resume_path("cp-resume-001"),
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
    let (status, body) = get_json(app, &routes::checkpoint_detail_path("does-not-exist")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

#[tokio::test]
async fn checkpoint_resume_already_resumed_returns_400() {
    let state = test_state().await;
    let app = build_app(state);

    // Create + resume once
    post_json(
        app.clone(),
        routes::CHECKPOINTS,
        serde_json::json!({
            "checkpoint_id": "cp-double-001",
            "message": "Once only"
        }),
    )
    .await;
    post_json(
        app.clone(),
        &routes::checkpoint_resume_path("cp-double-001"),
        serde_json::json!({ "human_input": { "ok": true } }),
    )
    .await;

    // Attempt to resume again
    let (status, body) = post_json(
        app,
        &routes::checkpoint_resume_path("cp-double-001"),
        serde_json::json!({ "human_input": { "ok": false } }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "expected 400 on double-resume: {body}"
    );
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
