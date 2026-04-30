use super::*;

#[tokio::test]
async fn skill_execute_rejects_missing_required_capability_before_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_CAPABILITY_NOT_REGISTERED),
        "expected capability policy rejection: {body}"
    );
}

#[tokio::test]
async fn skill_execute_allows_declared_read_only_capability() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = inv_tool_artifact_bytes(FIXTURE_TOOL, None);
    write_policy_skill_with_artifact(temp.path(), &artifact, FIXTURE_TOOL, FIXTURE_TOOL);
    let state = test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "skill execute failed: {body}");
    assert_eq!(body["content"], FIXTURE_OUTPUT);
}

#[tokio::test]
async fn skill_execute_allows_sandboxed_side_effectful_capability_after_preflight() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = inv_tool_artifact_bytes(FIXTURE_TOOL, None);
    write_sandboxed_policy_skill_with_artifact(temp.path(), &artifact, FIXTURE_TOOL, FIXTURE_TOOL);

    let mut runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    runtime.set_sandbox_registry(Arc::new(fixture_sandbox_registry()));
    runtime
        .capability_system()
        .register(Arc::new(FixtureSandboxedCapability::new(FIXTURE_TOOL)))
        .expect("register fixture sandboxed capability");
    let app = build_app(
        test_state_with_runtime_and_skill_roots(runtime, vec![temp.path().to_path_buf()]).await,
    );

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "skill execute failed: {body}");
    assert_eq!(body["content"], FIXTURE_OUTPUT);
}

#[tokio::test]
async fn skill_execute_rejects_sandboxed_policy_without_backend_preflight() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = inv_tool_artifact_bytes(FIXTURE_TOOL, None);
    write_sandboxed_policy_skill_with_artifact(temp.path(), &artifact, FIXTURE_TOOL, FIXTURE_TOOL);
    let state = test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSandboxedCapability::new(FIXTURE_TOOL)))
        .expect("register fixture sandboxed capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_SANDBOX_PREFLIGHT),
        "expected sandbox preflight rejection: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_degraded_sandbox_preflight() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = inv_tool_artifact_bytes(FIXTURE_TOOL, None);
    write_sandboxed_policy_skill_with_artifact(temp.path(), &artifact, FIXTURE_TOOL, FIXTURE_TOOL);

    let mut runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    runtime.set_sandbox_registry(Arc::new(fixture_degraded_sandbox_registry()));
    runtime
        .capability_system()
        .register(Arc::new(FixtureSandboxedCapability::new(FIXTURE_TOOL)))
        .expect("register fixture sandboxed capability");
    let app = build_app(
        test_state_with_runtime_and_skill_roots(runtime, vec![temp.path().to_path_buf()]).await,
    );

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_SANDBOX_DEGRADED),
        "expected degraded sandbox preflight rejection: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_inv_tool_not_in_allowed_tools() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = inv_tool_artifact_bytes(FIXTURE_TOOL, None);
    write_policy_skill_with_artifact(temp.path(), &artifact, FIXTURE_TOOL, "other_tool");
    let state = test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new("other_tool")))
        .expect("register other fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_UNDECLARED_CAPABILITY),
        "expected undeclared capability rejection: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_python_handler_inv_tool() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = inv_tool_artifact_bytes(FIXTURE_TOOL, Some("sha256:fixture"));
    write_policy_skill_with_artifact(temp.path(), &artifact, FIXTURE_TOOL, FIXTURE_TOOL);
    let state = test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_PYTHON_INV_TOOL_UNSUPPORTED),
        "expected python-backed INV_TOOL rejection: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_non_read_only_side_effect_policies() {
    for policy in ["write_files", "network", "requires_approval"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact = skill_artifact_bytes(AISOperationType::ConstStr);
        write_side_effect_policy_skill_with_artifact(temp.path(), &artifact, policy);
        let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

        let (status, body) = post_json(
            app,
            &skill_execute_route(FIXTURE_SKILL_ID),
            serde_json::json!({}),
        )
        .await;

        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "expected 400 for side_effect_policy={policy}: {body}"
        );
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains(ERROR_SIDE_EFFECT_POLICY_UNSUPPORTED),
            "expected side-effect policy rejection for {policy}: {body}"
        );
    }
}

#[tokio::test]
async fn skill_execute_rejects_client_session_root_field() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let mut request = serde_json::Map::new();
    request.insert(
        REQUEST_SESSION_ROOT_FIELD.to_string(),
        serde_json::Value::String(REQUEST_ROOT_INJECTION.to_string()),
    );

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::Value::Object(request),
    )
    .await;

    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected unknown-field rejection, got {status}: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_raw_air_field() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let mut request = serde_json::Map::new();
    request.insert(
        REQUEST_AIR_FIELD.to_string(),
        serde_json::Value::String(const_only_air()),
    );

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::Value::Object(request),
    )
    .await;

    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected unknown-field rejection, got {status}: {body}"
    );
}

#[tokio::test]
async fn skill_execute_query_root_cannot_change_server_owned_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let route = format!(
        "{}?{QUERY_ROOT_INJECTION}",
        skill_execute_route(FIXTURE_SKILL_ID)
    );

    let (status, body) = post_json(
        app,
        &route,
        serde_json::json!({ "session_id": FIXTURE_SESSION_ID }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "skill execute failed: {body}");
    let session_dir = body["session_dir"].as_str().expect("session dir");
    assert!(
        session_dir.contains("/skills/"),
        "session dir: {session_dir}"
    );
    assert!(
        !session_dir.contains(REQUEST_ROOT_INJECTION),
        "session dir: {session_dir}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_missing_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_valid_skill(temp.path(), FIXTURE_PACKAGE_DIR);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_SKILL_NOT_COMPILED),
        "expected not compiled error: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_missing_manifest_artifact_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let skill_dir = temp.path().join(FIXTURE_PACKAGE_DIR);
    let artifact = skill_artifact_bytes(AISOperationType::ConstStr);
    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{FIXTURE_SKILL_ID}"
version = "{FIXTURE_SKILL_VERSION}"
entry_flow = "{FIXTURE_ENTRY_FLOW}"
"#
        ),
    );
    std::fs::write(skill_dir.join(FILE_SKILL_ARTIFACT), artifact).expect(FILE_SKILL_ARTIFACT);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_ARTIFACT_HASH_REQUIRED),
        "expected missing artifact hash error: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_artifact_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let skill_dir = temp.path().join(FIXTURE_PACKAGE_DIR);
    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{FIXTURE_SKILL_ID}"
version = "{FIXTURE_SKILL_VERSION}"
entry_flow = "{FIXTURE_ENTRY_FLOW}"
artifact_hash = "{BAD_ARTIFACT_HASH}"
"#
        ),
    );
    std::fs::write(
        skill_dir.join(FILE_SKILL_ARTIFACT),
        skill_artifact_bytes(AISOperationType::ConstStr),
    )
    .expect(FILE_SKILL_ARTIFACT);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_ARTIFACT_HASH_MISMATCH),
        "expected hash mismatch error: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_malformed_artifact_even_with_matching_file_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = corrupted_skill_artifact_bytes();
    write_executable_skill_with_artifact(
        temp.path(),
        FIXTURE_PACKAGE_DIR,
        FIXTURE_SKILL_VERSION,
        &artifact,
        FIXTURE_SKILL_ID,
        FIXTURE_ENTRY_FLOW,
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_INVALID_ARTIFACT),
        "expected invalid artifact error: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_entry_flow_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact =
        skill_artifact_bytes_with_entry(AISOperationType::ConstStr, FIXTURE_ALT_ENTRY_FLOW);
    write_executable_skill_with_artifact(
        temp.path(),
        FIXTURE_PACKAGE_DIR,
        FIXTURE_SKILL_VERSION,
        &artifact,
        FIXTURE_SKILL_ID,
        FIXTURE_ENTRY_FLOW,
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_ENTRY_FLOW_MISMATCH),
        "expected entry flow mismatch: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_embedded_manifest_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = skill_artifact_bytes_with_embedded_manifest(&format!(
        r#"
skill_id = "wrong-skill"
version = "{FIXTURE_SKILL_VERSION}"
entry_flow = "{FIXTURE_ENTRY_FLOW}"
"#
    ));
    write_executable_skill_with_artifact(
        temp.path(),
        FIXTURE_PACKAGE_DIR,
        FIXTURE_SKILL_VERSION,
        &artifact,
        FIXTURE_SKILL_ID,
        FIXTURE_ENTRY_FLOW,
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(&versioned_skill_id()),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "expected rejection: {body}"
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("embedded skill manifest skill_id"),
        "expected embedded manifest rejection: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_disallowed_runtime_operations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = skill_artifact_bytes(AISOperationType::Ask);
    write_executable_skill_with_artifact(
        temp.path(),
        FIXTURE_PACKAGE_DIR,
        FIXTURE_SKILL_VERSION,
        &artifact,
        FIXTURE_SKILL_ID,
        FIXTURE_ENTRY_FLOW,
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_DISALLOWED_OPERATION),
        "expected policy rejection: {body}"
    );
}

#[tokio::test]
async fn skill_execute_rejects_invalid_session_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    for session_id in [
        INVALID_SESSION_ID_WITH_SEPARATOR,
        INVALID_SESSION_ID_WITH_BACKSLASH,
        INVALID_SESSION_ID_WITH_SPACE,
        INVALID_SESSION_ID_WITH_SEMICOLON,
        INVALID_SESSION_ID_CURRENT,
        INVALID_SESSION_ID_PARENT,
    ] {
        let (status, body) = post_json(
            app.clone(),
            &skill_execute_route(FIXTURE_SKILL_ID),
            serde_json::json!({ "session_id": session_id }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains(ERROR_INVALID_SESSION_ID),
            "expected invalid session id error: {body}"
        );
    }
}
