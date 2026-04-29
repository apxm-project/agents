use super::*;

#[tokio::test]
async fn skill_execute_static_artifact_returns_result_from_server_owned_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let versioned_id = versioned_skill_id();

    let (status, body) = post_json(
        app.clone(),
        &skill_execute_route(&versioned_id),
        serde_json::json!({ "session_id": FIXTURE_SESSION_ID }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "skill execute failed: {body}");
    let execution_id = body["execution_id"].as_str().expect("execution id");
    assert!(!execution_id.is_empty());
    assert_eq!(body["content"], FIXTURE_OUTPUT);
    assert_eq!(body["stats"]["executed_nodes"], 1);
    assert_eq!(body["stats"]["failed_nodes"], 0);
    let session_dir = body["session_dir"].as_str().expect("session dir");
    assert!(
        session_dir.contains("/skills/"),
        "session dir: {session_dir}"
    );
    assert!(
        session_dir.contains(FIXTURE_SKILL_ID),
        "session dir: {session_dir}"
    );
    assert!(
        session_dir.ends_with(FIXTURE_SESSION_ID),
        "session dir: {session_dir}"
    );
    assert!(std::path::Path::new(session_dir).is_dir());

    let (detail_status, detail_body) =
        get_json(app.clone(), &execution_detail_route(execution_id)).await;
    assert_eq!(
        detail_status,
        StatusCode::OK,
        "execution detail failed: {detail_body}"
    );
    assert_eq!(detail_body["execution_id"], execution_id);
    assert_eq!(detail_body["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(detail_body["skill_version"], FIXTURE_SKILL_VERSION);
    assert_eq!(detail_body["entry_flow"], FIXTURE_ENTRY_FLOW);
    assert!(detail_body["source_hash"].is_null());
    assert!(detail_body["air_hash"].is_null());
    assert!(
        detail_body["artifact_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("blake3:")),
        "artifact_hash should be persisted: {detail_body}"
    );
    assert_eq!(detail_body["session_id"], FIXTURE_SESSION_ID);
    assert_eq!(detail_body["session_dir"], session_dir);
    assert_eq!(detail_body["status"], "succeeded");
    assert_eq!(detail_body["result"]["content"], FIXTURE_OUTPUT);
    assert_eq!(detail_body["node_outputs"][0]["node_id"], 1);
    assert_eq!(
        detail_body["node_outputs"][0][NODE_OUTPUT_FIELD][SUMMARY_FIELD],
        FIXTURE_OUTPUT_SUMMARY
    );
    assert_eq!(
        detail_body["node_outputs"][0][NODE_OUTPUT_FIELD][REDACTED_FIELD],
        true
    );
    assert!(
        detail_body["node_outputs"][0][NODE_OUTPUT_FIELD][HASH_FIELD]
            .as_str()
            .expect("node output hash")
            .starts_with(REDACTION_HASH_PREFIX_BLAKE3)
    );
    assert_eq!(detail_body["node_metrics"][0]["node_id"], 1);
    assert_eq!(
        detail_body["node_metrics"][0]["metrics"]["operation"]["attempts"],
        1
    );
    assert!(detail_body["started_at_ms"].is_number());
    assert!(detail_body["completed_at_ms"].is_number());

    let (node_status, node_body) =
        get_json(app, &execution_node_detail_route(execution_id, 1)).await;
    assert_eq!(
        node_status,
        StatusCode::OK,
        "node detail failed: {node_body}"
    );
    assert_eq!(node_body["execution_id"], execution_id);
    assert_eq!(node_body["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(node_body["skill_version"], FIXTURE_SKILL_VERSION);
    assert_eq!(node_body["session_id"], FIXTURE_SESSION_ID);
    assert_eq!(node_body["session_dir"], session_dir);
    assert_eq!(node_body["node_id"], 1);
    assert_eq!(node_body["outputs"][0]["node_id"], 1);
    assert_eq!(
        node_body["outputs"][0][NODE_OUTPUT_FIELD][SUMMARY_FIELD],
        FIXTURE_OUTPUT_SUMMARY
    );
    assert!(node_body["outputs"][0]["observed_at_ms"].is_number());
    assert_eq!(node_body["metrics"][0]["node_id"], 1);
    assert_eq!(
        node_body["metrics"][0]["metrics"]["operation"]["attempts"],
        1
    );
    assert!(node_body["metrics"][0]["observed_at_ms"].is_number());

    let snapshot_path = execution_record_snapshot_path(session_dir, execution_id);
    let snapshot = std::fs::read_to_string(&snapshot_path).expect("execution record snapshot");
    let snapshot_body: serde_json::Value =
        serde_json::from_str(&snapshot).expect("execution record snapshot json");
    assert_eq!(snapshot_body["execution_id"], execution_id);
    assert_eq!(snapshot_body["status"], "succeeded");
    assert_eq!(snapshot_body["entry_flow"], FIXTURE_ENTRY_FLOW);
    assert!(
        snapshot_body["artifact_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("blake3:")),
        "artifact_hash should be persisted in snapshot: {snapshot_body}"
    );
    assert_eq!(
        snapshot_body["node_outputs"][0][NODE_OUTPUT_FIELD][SUMMARY_FIELD],
        FIXTURE_OUTPUT_SUMMARY
    );
    assert!(
        !snapshot_body["node_outputs"][0]
            .to_string()
            .contains(FIXTURE_OUTPUT)
    );
}

#[tokio::test]
async fn skill_execute_unknown_skill_returns_404() {
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(UNKNOWN_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

#[tokio::test]
async fn skill_execute_unknown_version_returns_404() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let unknown_version = format!("{FIXTURE_SKILL_ID}@{UNKNOWN_SKILL_VERSION}");

    let (status, body) = post_json(
        app,
        &skill_execute_route(&unknown_version),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

#[tokio::test]
async fn skill_execute_ambiguous_unversioned_skill_returns_400() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill_with_version(temp.path(), FIXTURE_PACKAGE_DIR, FIXTURE_SKILL_VERSION);
    write_complete_skill_with_version(
        temp.path(),
        FIXTURE_PACKAGE_V2_DIR,
        FIXTURE_SKILL_NEXT_VERSION,
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
}

#[tokio::test]
async fn skill_execute_versioned_skill_uses_selected_package() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill_with_output(
        temp.path(),
        FIXTURE_PACKAGE_DIR,
        FIXTURE_SKILL_VERSION,
        FIXTURE_OUTPUT,
    );
    write_executable_skill_with_output(
        temp.path(),
        FIXTURE_PACKAGE_V2_DIR,
        FIXTURE_SKILL_NEXT_VERSION,
        FIXTURE_OUTPUT_V2,
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let requested_id = format!("{FIXTURE_SKILL_ID}@{FIXTURE_SKILL_NEXT_VERSION}");

    let (status, body) = post_json(
        app,
        &skill_execute_route(&requested_id),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "versioned execute failed: {body}");
    assert_eq!(body["content"], FIXTURE_OUTPUT_V2);
}
