use super::*;

#[tokio::test]
async fn skill_execute_stream_emits_started_and_final_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let versioned_id = versioned_skill_id();

    let (status, text) = post_json_text(
        app.clone(),
        &skill_execute_stream_route(&versioned_id),
        serde_json::json!({ "session_id": FIXTURE_SESSION_ID }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "skill stream failed: {text}");
    let events = sse_data_events(&text);
    let started = events
        .iter()
        .find(|event| event["payload"]["kind"] == EVENT_SKILL_EXECUTE_STARTED)
        .expect("started event");
    let execution_id = started["payload"]["execution_id"]
        .as_str()
        .expect("execution id");
    assert_eq!(started["payload"]["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(started["payload"]["skill_version"], FIXTURE_SKILL_VERSION);
    assert_eq!(started["payload"]["session_id"], FIXTURE_SESSION_ID);

    let complete = events
        .iter()
        .find(|event| event["payload"]["kind"] == EVENT_SKILL_EXECUTE_COMPLETE)
        .expect("complete event");
    assert_eq!(complete["payload"]["execution_id"], execution_id);
    assert_eq!(complete["payload"]["result"]["content"], FIXTURE_OUTPUT);

    let (detail_status, detail_body) = get_json(app, &execution_detail_route(execution_id)).await;
    assert_eq!(
        detail_status,
        StatusCode::OK,
        "execution detail failed: {detail_body}"
    );
    assert_eq!(detail_body["status"], "succeeded");
    assert_eq!(detail_body["result"]["content"], FIXTURE_OUTPUT);
    assert_eq!(detail_body["node_outputs"][0]["node_id"], 1);
    assert_eq!(
        detail_body["node_outputs"][0][NODE_OUTPUT_FIELD][SUMMARY_FIELD],
        FIXTURE_OUTPUT_SUMMARY
    );
    assert_eq!(detail_body["node_metrics"][0]["node_id"], 1);
    assert_eq!(
        detail_body["node_metrics"][0]["metrics"]["operation"]["attempts"],
        1
    );
}

#[tokio::test]
async fn skill_execute_stream_emits_node_output_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let versioned_id = versioned_skill_id();

    let (status, text) = post_json_text(
        app.clone(),
        &skill_execute_stream_route(&versioned_id),
        serde_json::json!({ "session_id": FIXTURE_SESSION_ID }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "skill stream failed: {text}");
    let events = sse_data_events(&text);
    let started_index = events
        .iter()
        .position(|event| event["payload"]["kind"] == EVENT_SKILL_EXECUTE_STARTED)
        .expect("started event");
    let node_metrics_index = events
        .iter()
        .position(|event| event["payload"]["kind"] == EVENT_NODE_METRICS)
        .expect("node metrics event");
    let node_output_index = events
        .iter()
        .position(|event| event["payload"]["kind"] == EVENT_NODE_OUTPUT)
        .expect("node output event");
    let complete_index = events
        .iter()
        .position(|event| event["payload"]["kind"] == EVENT_SKILL_EXECUTE_COMPLETE)
        .expect("complete event");

    assert!(started_index < node_metrics_index);
    assert!(started_index < node_output_index);
    assert!(node_metrics_index < complete_index);
    assert!(node_output_index < complete_index);

    let node_metrics = &events[node_metrics_index];
    let node_output = &events[node_output_index];
    let execution_id = events[started_index]["payload"]["execution_id"]
        .as_str()
        .expect("execution id");
    assert_eq!(node_metrics["meta"]["source"], "runtime");
    assert_eq!(node_metrics["meta"]["trace_id"], execution_id);
    assert_eq!(node_metrics["payload"]["node_id"], 1);
    assert_eq!(
        node_metrics["payload"]["metrics"]["operation"]["attempts"],
        1
    );
    assert_eq!(node_output["meta"]["source"], "runtime");
    assert_eq!(node_output["meta"]["trace_id"], execution_id);
    assert_eq!(node_output["meta"]["skill"]["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(
        node_output["meta"]["skill"]["skill_version"],
        FIXTURE_SKILL_VERSION
    );
    assert_eq!(
        node_output["meta"]["skill"]["flow_name"],
        FIXTURE_ENTRY_FLOW
    );
    assert_eq!(node_output["payload"]["node_id"], 1);
    assert_eq!(
        node_output["payload"][NODE_OUTPUT_FIELD][SUMMARY_FIELD],
        FIXTURE_OUTPUT_SUMMARY
    );
    assert_eq!(
        node_output["payload"][NODE_OUTPUT_FIELD][REDACTED_FIELD],
        true
    );
    assert!(
        node_output["payload"][NODE_OUTPUT_FIELD][HASH_FIELD]
            .as_str()
            .expect("node output hash")
            .starts_with(REDACTION_HASH_PREFIX_BLAKE3)
    );
    assert!(!node_output.to_string().contains(FIXTURE_OUTPUT));

    let (node_status, node_body) =
        get_json(app, &execution_node_detail_route(execution_id, 1)).await;
    assert_eq!(
        node_status,
        StatusCode::OK,
        "node detail failed: {node_body}"
    );
    assert_eq!(node_body["execution_id"], execution_id);
    assert_eq!(node_body["node_id"], 1);
    assert_eq!(
        node_body["outputs"][0][NODE_OUTPUT_FIELD][SUMMARY_FIELD],
        FIXTURE_OUTPUT_SUMMARY
    );
    assert_eq!(
        node_body["metrics"][0]["metrics"]["operation"]["attempts"],
        1
    );
}
