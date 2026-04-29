use super::*;

#[tokio::test]
async fn execute_invalid_air_returns_400() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/execute",
        serde_json::json!({
            "air": "not valid AIR"
        }),
    )
    .await;
    // Invalid AIR -> compiler rejects -> 400 Bad Request
    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(body["error"].is_string(), "expected error message: {body}");
}

#[tokio::test]
async fn execute_empty_air_returns_error() {
    let app = build_app(test_state().await);
    let (status, _body) = post_json(app, "/v1/execute", serde_json::json!({})).await;
    // Missing air field -> 400 or 422
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400/422 for empty request, got {status}"
    );
}

#[tokio::test]
async fn server_runtime_configures_sandbox_registry() {
    let runtime = build_server_runtime().await.expect("server runtime");

    assert!(
        !runtime.sandbox_registry().is_empty(),
        "server runtime should install host sandbox backends"
    );
}

#[test]
fn prepare_request_rejects_client_session_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_root = temp.path().join("sessions");
    let request = ExecuteRequest {
        air: const_only_air(),
        args: vec![],
        session_id: Some("explicit-session".to_string()),
        session_root: Some(session_root.to_string_lossy().to_string()),
    };

    let error = prepare_request(request).expect_err("client session_root should be rejected");
    assert!(
        error.message.contains("session_root is server-controlled"),
        "unexpected error: {error:?}"
    );
    assert!(
        !session_root.exists(),
        "client-provided session root should not be created"
    );
}

#[test]
fn prepare_request_rejects_unsafe_session_id() {
    for session_id in [
        INVALID_SESSION_ID_WITH_SEPARATOR,
        INVALID_SESSION_ID_WITH_BACKSLASH,
        INVALID_SESSION_ID_WITH_SPACE,
        INVALID_SESSION_ID_WITH_SEMICOLON,
        INVALID_SESSION_ID_CURRENT,
        INVALID_SESSION_ID_PARENT,
    ] {
        let request = ExecuteRequest {
            air: const_only_air(),
            args: vec![],
            session_id: Some(session_id.to_string()),
            session_root: None,
        };

        let error = prepare_request(request).expect_err("unsafe session id should be rejected");
        assert!(
            error.message.contains(ERROR_INVALID_SESSION_ID),
            "expected invalid session id error for {session_id:?}: {error:?}"
        );
    }
}

#[test]
fn prepare_request_rejects_session_root_without_session_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_root = temp.path().join("sessions");
    let request = ExecuteRequest {
        air: const_only_air(),
        args: vec![],
        session_id: None,
        session_root: Some(session_root.to_string_lossy().to_string()),
    };

    let error = prepare_request(request).expect_err("client session_root should be rejected");
    assert!(
        error.message.contains("session_root is server-controlled"),
        "unexpected error: {error:?}"
    );
    assert!(
        !session_root.exists(),
        "client-provided session root should not be created"
    );
}

#[tokio::test]
async fn execute_rejects_client_session_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_root = temp.path().join("sessions");
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/execute",
        serde_json::json!({
            "air": const_only_air(),
            "session_id": "server-session",
            "session_root": session_root.to_string_lossy().to_string()
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "expected rejection: {body}"
    );
    assert!(
        body.to_string()
            .contains("session_root is server-controlled"),
        "unexpected response: {body}"
    );
    assert!(
        !session_root.exists(),
        "client-provided session root should not be created"
    );
}
