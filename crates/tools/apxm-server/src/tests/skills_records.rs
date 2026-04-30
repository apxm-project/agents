use super::*;

#[tokio::test]
async fn execution_detail_unknown_returns_404() {
    let app = build_app(test_state().await);

    let (status, body) = get_json(app, &execution_detail_route(UNKNOWN_SKILL_ID)).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

#[tokio::test]
async fn execution_node_detail_unknown_node_returns_404() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app.clone(),
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "skill execute failed: {body}");
    let execution_id = body["execution_id"].as_str().expect("execution id");
    let (node_status, node_body) =
        get_json(app, &execution_node_detail_route(execution_id, 99)).await;

    assert_eq!(
        node_status,
        StatusCode::NOT_FOUND,
        "expected missing node 404: {node_body}"
    );
}

#[tokio::test]
async fn execution_store_reloads_persisted_records_for_api_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_root = temp.path().join("sessions");
    let session_dir = session_root
        .join(FIXTURE_SKILL_SESSION_DIR)
        .join(FIXTURE_SKILL_ID)
        .join(FIXTURE_SESSION_ID);
    let session_dir_str = session_dir.to_string_lossy().to_string();

    let writer = ExecutionStore::new();
    let record = writer.start_skill_execution(
        FIXTURE_SKILL_ID,
        FIXTURE_SKILL_VERSION,
        FIXTURE_SESSION_ID,
        &session_dir_str,
    );
    writer.record_node_output(
        &record.execution_id,
        FIXTURE_NODE_ID,
        Some(FIXTURE_OUTPUT_NAME.to_string()),
        RedactedContent::from_text(FIXTURE_OUTPUT),
    );
    writer.record_node_metrics(
        &record.execution_id,
        FIXTURE_NODE_ID,
        Some(FIXTURE_OUTPUT_NAME.to_string()),
        NodeMetrics::new(FIXTURE_NODE_ID),
    );
    writer.complete_success(
        &record.execution_id,
        fixture_execute_response(Some(session_dir_str.clone())),
    );

    let loaded_store = ExecutionStore::from_session_roots([session_root]);
    assert!(
        loaded_store.get(&record.execution_id).is_some(),
        "persisted execution should reload into the in-memory index"
    );
    let app =
        build_app(test_state_with_skill_roots_and_execution_store(Vec::new(), loaded_store).await);

    let (list_status, list_body) = get_json(app.clone(), routes::EXECUTIONS).await;
    assert_eq!(
        list_status,
        StatusCode::OK,
        "execution list failed: {list_body}"
    );
    let listed = list_body.as_array().expect("execution list body");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["execution_id"], record.execution_id);
    assert_eq!(listed[0]["skill_id"], FIXTURE_SKILL_ID);

    let (detail_status, detail_body) =
        get_json(app.clone(), &execution_detail_route(&record.execution_id)).await;
    assert_eq!(
        detail_status,
        StatusCode::OK,
        "execution detail failed after reload: {detail_body}"
    );
    assert_eq!(detail_body["execution_id"], record.execution_id);
    assert_eq!(detail_body["status"], STATUS_SUCCEEDED);
    assert_eq!(detail_body["result"]["content"], FIXTURE_OUTPUT);
    assert_eq!(
        detail_body["node_outputs"][0]["node_name"],
        FIXTURE_OUTPUT_NAME
    );

    let (node_status, node_body) = get_json(
        app,
        &execution_node_detail_route(&record.execution_id, FIXTURE_NODE_ID),
    )
    .await;
    assert_eq!(
        node_status,
        StatusCode::OK,
        "node detail failed after reload: {node_body}"
    );
    assert_eq!(node_body["outputs"][0]["node_name"], FIXTURE_OUTPUT_NAME);
    assert_eq!(node_body["metrics"][0]["node_name"], FIXTURE_OUTPUT_NAME);
}
