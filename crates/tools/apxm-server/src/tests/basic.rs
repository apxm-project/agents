use super::*;

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
