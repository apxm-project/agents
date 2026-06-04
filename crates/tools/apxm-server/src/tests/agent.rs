use super::*;

// ── Agent Registry ────────────────────────────────────────────────────────

#[tokio::test]
async fn agent_registry_register_returns_ok() {
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app,
        routes::AGENTS_REGISTER,
        serde_json::json!({
            "name": "test-agent",
            "url": "https://example.com/agent",
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
    let (status, body) = get_json(app, routes::AGENTS).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array(), "expected array: {body}");
}

#[tokio::test]
async fn agent_registry_missing_name_returns_400() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        routes::AGENTS_REGISTER,
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
    let (status, body) = get_json(app, routes::AGENT_CARD).await;
    assert_eq!(status, StatusCode::OK, "agent card failed: {body}");
    assert!(body["name"].is_string(), "missing 'name': {body}");
    assert!(body["url"].is_string(), "missing 'url': {body}");
    assert!(body["version"].is_string(), "missing 'version': {body}");
    assert!(
        body["capabilities"].is_object(),
        "missing 'capabilities': {body}"
    );
}
