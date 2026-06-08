use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn mcp_initialize_returns_protocol_version() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 1,
            "method": MCP_METHOD_INITIALIZE,
            "params": { (mcp_fields::PROTOCOL_VERSION): apxm_core::constants::protocols::MCP_VERSION }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "mcp initialize failed: {body}");
    assert_eq!(body["jsonrpc"], MCP_JSONRPC_VERSION);
    assert_eq!(body["id"], 1);
    assert!(body["result"].is_object(), "expected result object: {body}");
    let version = body["result"][mcp_fields::PROTOCOL_VERSION]
        .as_str()
        .unwrap_or("");
    assert!(!version.is_empty(), "protocolVersion missing: {body}");
    assert_eq!(
        body["result"][mcp_fields::CAPABILITIES][mcp_fields::RESOURCES][mcp_fields::LIST_CHANGED],
        false
    );
    assert_eq!(
        body["result"][mcp_fields::CAPABILITIES][mcp_fields::RESOURCES][mcp_fields::SUBSCRIBE],
        false
    );
}

#[tokio::test]
async fn mcp_tools_list_returns_array() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 2,
            "method": MCP_METHOD_TOOLS_LIST,
            "params": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tools = &body["result"]["tools"];
    assert!(tools.is_array(), "expected tools array: {body}");
}

#[tokio::test]
async fn mcp_resources_list_returns_skill_resources() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 12,
            "method": MCP_METHOD_RESOURCES_LIST,
            "params": {}
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "MCP resources/list failed: {body}");
    let resources = body["result"][mcp_fields::RESOURCES]
        .as_array()
        .expect("resources array");
    let uris: Vec<&str> = resources
        .iter()
        .filter_map(|resource| resource[mcp_fields::URI].as_str())
        .collect();
    assert!(
        uris.contains(&"skill://checkout-context-triage/SKILL.md"),
        "resources: {body}"
    );
    assert!(
        uris.contains(&"skill://checkout-context-triage/_manifest"),
        "resources: {body}"
    );
}

#[tokio::test]
async fn mcp_resources_read_returns_skill_content() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let mut params = serde_json::Map::new();
    params.insert(
        MCP_PARAM_URI.to_string(),
        serde_json::Value::String("skill://checkout-context-triage/SKILL.md".to_string()),
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 13,
            "method": MCP_METHOD_RESOURCES_READ,
            "params": serde_json::Value::Object(params)
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "MCP resources/read failed: {body}");
    assert_eq!(
        body["result"][mcp_fields::CONTENTS][0][mcp_fields::MIME_TYPE],
        "text/markdown"
    );
    assert_eq!(
        body["result"][mcp_fields::CONTENTS][0][mcp_fields::TEXT],
        FIXTURE_SOURCE
    );
}

#[tokio::test]
async fn mcp_resources_require_versioned_uri_for_duplicate_skill_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill_with_version(temp.path(), FIXTURE_PACKAGE_DIR, FIXTURE_SKILL_VERSION);
    write_complete_skill_with_version(
        temp.path(),
        FIXTURE_PACKAGE_V2_DIR,
        FIXTURE_SKILL_NEXT_VERSION,
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let versioned_v1_id = format!("{FIXTURE_SKILL_ID}@{FIXTURE_SKILL_VERSION}");
    let versioned_v2_id = format!("{FIXTURE_SKILL_ID}@{FIXTURE_SKILL_NEXT_VERSION}");
    let versioned_v1_uri = skill_resource_uri(&versioned_v1_id, FILE_SKILL_SOURCE);
    let versioned_v2_uri = skill_resource_uri(&versioned_v2_id, FILE_SKILL_SOURCE);

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 14,
            "method": MCP_METHOD_RESOURCES_LIST,
            "params": {}
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "MCP resources/list failed: {body}");
    let resources = body["result"][mcp_fields::RESOURCES]
        .as_array()
        .expect("resources array");
    let uris: Vec<&str> = resources
        .iter()
        .filter_map(|resource| resource[mcp_fields::URI].as_str())
        .collect();
    assert!(
        uris.contains(&versioned_v1_uri.as_str()),
        "resources: {body}"
    );
    assert!(
        uris.contains(&versioned_v2_uri.as_str()),
        "resources: {body}"
    );

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 15,
            "method": MCP_METHOD_RESOURCES_READ,
            "params": { (MCP_PARAM_URI): skill_resource_uri(FIXTURE_SKILL_ID, FILE_SKILL_SOURCE) }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "MCP resources/read failed: {body}");
    assert_eq!(body["error"]["code"], -32602);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("multiple versions"),
        "expected ambiguity error: {body}"
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 16,
            "method": MCP_METHOD_RESOURCES_READ,
            "params": { (MCP_PARAM_URI): versioned_v2_uri }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "MCP resources/read failed: {body}");
    assert_eq!(
        body["result"][mcp_fields::CONTENTS][0][mcp_fields::TEXT],
        FIXTURE_SOURCE
    );
}

#[tokio::test]
async fn mcp_tools_list_includes_skill_inventory_tools() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 22,
            "method": MCP_METHOD_TOOLS_LIST,
            "params": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tools = body["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&MCP_TOOL_APXM_SKILLS_LIST), "tools: {body}");
    assert!(names.contains(&MCP_TOOL_APXM_SKILL_GET), "tools: {body}");
    assert!(
        names.contains(&MCP_TOOL_APXM_SKILL_VALIDATE),
        "tools: {body}"
    );
    assert!(names.contains(&MCP_TOOL_APXM_SKILL_CALL), "tools: {body}");
    assert!(
        names.contains(&MCP_TOOL_APXM_PLAN_AS_GRAPH),
        "tools: {body}"
    );
    assert!(names.contains(&MCP_TOOL_APXM_TRACE_FETCH), "tools: {body}");
    assert!(names.contains(&MCP_TOOL_APXM_AAM_RECALL), "tools: {body}");
    assert!(
        names.contains(&MCP_TOOL_APXM_EVIDENCE_LOOKUP),
        "tools: {body}"
    );
    assert!(
        names.contains(&MCP_TOOL_APXM_CAPABILITY_LIST),
        "tools: {body}"
    );
    assert!(
        names.contains(&MCP_TOOL_APXM_WORKFLOW_START),
        "tools: {body}"
    );
    assert!(
        names.contains(&MCP_TOOL_APXM_WORKFLOW_STATUS),
        "tools: {body}"
    );
    assert!(
        names.contains(&MCP_TOOL_APXM_WORKFLOW_EVENTS),
        "tools: {body}"
    );
    assert!(
        names.contains(&MCP_TOOL_APXM_WORKFLOW_CANCEL),
        "tools: {body}"
    );
}

#[tokio::test]
async fn mcp_workflow_start_status_and_events_use_server_execution_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_path = write_workflow_fixture(temp.path(), "child.air", &const_only_air());
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_START,
            serde_json::json!({
                "workflow_path": workflow_path,
                "session_id": "mcp-workflow-start-status"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "workflow start failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let started: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("workflow start response JSON");
    let execution_id = started[tool_result::EXECUTION_ID]
        .as_str()
        .expect("execution_id");
    assert_eq!(started[tool_result::STATUS], STATUS_RUNNING);
    assert_eq!(started[MCP_ARG_SESSION_ID], "mcp-workflow-start-status");
    assert!(
        started["session_dir"]
            .as_str()
            .is_some_and(|path| !path.is_empty()),
        "session_dir missing: {started}"
    );

    let status_body = wait_for_workflow_status(app.clone(), execution_id, STATUS_SUCCEEDED).await;
    let workflow_status: serde_json::Value =
        serde_json::from_str(tool_text(&status_body)).expect("workflow status response JSON");
    assert_eq!(workflow_status[tool_result::STATUS], STATUS_SUCCEEDED);
    assert_eq!(
        workflow_status[tool_result::EXECUTION_ID],
        execution_id,
        "status should use the server-owned execution_id"
    );
    assert!(
        workflow_status["result"]["results"]
            .as_object()
            .expect("result map")
            .values()
            .any(|value| value["result"] == FIXTURE_OUTPUT),
        "workflow spawn result should carry child output: {workflow_status}"
    );

    let events = workflow_events(app, execution_id, 0, 25).await;
    let event_items = events["events"].as_array().expect("events array");
    assert_eq!(
        workflow_status["totals"]["events"].as_u64(),
        Some(event_items.len() as u64),
        "status event total should match event stream after completion: {events}"
    );
    assert_eq!(
        events["done"], true,
        "single-page event fetch should be done"
    );
    let mut previous_seq = None;
    for event in event_items {
        assert_eq!(event["meta"]["trace_id"], execution_id);
        let seq = event["meta"]["seq"].as_u64().expect("event seq");
        if let Some(previous) = previous_seq {
            assert!(seq > previous, "event seq must be increasing: {events}");
        }
        previous_seq = Some(seq);
    }
    if let Some(last_seq) = previous_seq {
        assert_eq!(events["next_seq"], last_seq + 1);
    }
    assert!(
        event_items
            .iter()
            .any(|event| event["payload"]["kind"] == "execution_started"),
        "expected execution_started event: {events}"
    );
    assert!(
        event_items
            .iter()
            .any(|event| event["payload"]["kind"] == "execute_complete"),
        "expected execute_complete event: {events}"
    );
    assert!(
        event_items.iter().any(|event| {
            event["payload"]["kind"] == "operation_start"
                && event["payload"]["op_type"] == "WORKFLOW_SPAWN"
        }),
        "expected parent WORKFLOW_SPAWN operation_start event: {events}"
    );
    assert!(
        event_items.iter().any(|event| {
            event["payload"]["kind"] == "operation_end"
                && event["payload"]["op_type"] == "WORKFLOW_SPAWN"
                && event["payload"]["success"] == true
        }),
        "expected parent WORKFLOW_SPAWN operation_end event: {events}"
    );
}

#[tokio::test]
async fn mcp_workflow_fans_out_independent_steps_and_fans_in_output() {
    const LEFT_TOOL: &str = "fixture_parallel_left";
    const RIGHT_TOOL: &str = "fixture_parallel_right";

    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_path = write_parallel_workflow_fixture(
        temp.path(),
        &[
            ("left", "left.air", &tool_air(LEFT_TOOL)),
            ("right", "right.air", &tool_air(RIGHT_TOOL)),
        ],
        "{{left.output}}+{{right.output}}",
    );
    let state = test_state().await;
    let probe = Arc::new(WorkflowBarrier::new(2));
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureBarrierCapability::new(
            LEFT_TOOL,
            "left",
            Arc::clone(&probe),
        )))
        .expect("register left capability");
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureBarrierCapability::new(
            RIGHT_TOOL,
            "right",
            Arc::clone(&probe),
        )))
        .expect("register right capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_START,
            serde_json::json!({
                "workflow_path": workflow_path,
                "session_id": "mcp-workflow-parallel"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "workflow start failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let started: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("workflow start response JSON");
    assert_eq!(started[tool_result::STATUS], STATUS_RUNNING);
    let execution_id = started[tool_result::EXECUTION_ID]
        .as_str()
        .expect("execution_id");

    let status_body = wait_for_workflow_status(app.clone(), execution_id, STATUS_SUCCEEDED).await;
    let workflow_status: serde_json::Value =
        serde_json::from_str(tool_text(&status_body)).expect("workflow status response JSON");
    let spawn_payload = workflow_spawn_payload(&workflow_status);
    assert_eq!(spawn_payload["result"], "left+right");
    assert!(
        probe.max_seen.load(Ordering::SeqCst) >= 2,
        "independent .apxmw steps should overlap before workflow fan-in"
    );

    let workflow_session_dir = spawn_payload["session_dir"]
        .as_str()
        .expect("workflow session_dir");
    let results_path = std::path::Path::new(workflow_session_dir).join("results.json");
    let workflow_results: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&results_path).expect("read workflow results.json"))
            .expect("workflow results JSON");
    assert_eq!(workflow_results["status"], "Success");
    assert_eq!(workflow_results["output"], "left+right");
    assert_eq!(
        workflow_results["step_results"]["left"]["status"],
        "Success"
    );
    assert_eq!(
        workflow_results["step_results"]["right"]["status"],
        "Success"
    );
    assert_ne!(
        workflow_results["step_results"]["left"]["session_dir"],
        workflow_results["step_results"]["right"]["session_dir"],
        "parallel children should keep distinct child session dirs"
    );

    let events = workflow_events(app, execution_id, 0, 50).await;
    let event_items = events["events"].as_array().expect("events array");
    assert!(
        event_items
            .iter()
            .any(|event| event["payload"]["kind"] == "execute_complete"),
        "fan-in should complete parent workflow: {events}"
    );
    assert!(
        event_items
            .iter()
            .filter(|event| {
                event["payload"]["kind"] == "operation_start"
                    && event["payload"]["op_type"] == "INV_TOOL"
            })
            .count()
            >= 2,
        "child graph tool events should be visible through apxm_workflow_events: {events}"
    );
}

#[tokio::test]
async fn mcp_workflow_resume_parks_and_wakes_through_checkpoint_endpoint() {
    const CHECKPOINT_ID: &str = "mcp-workflow-resume-cp";

    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_path =
        write_workflow_fixture(temp.path(), "resume.air", &resume_air(CHECKPOINT_ID));
    let app = build_app(test_state().await);
    create_pending_checkpoint(app.clone(), CHECKPOINT_ID).await;

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_START,
            serde_json::json!({
                "workflow_path": workflow_path,
                "session_id": "mcp-workflow-resume"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "workflow start failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let started: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("workflow start response JSON");
    let execution_id = started[tool_result::EXECUTION_ID]
        .as_str()
        .expect("execution_id");

    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let running = workflow_status_json(app.clone(), execution_id).await;
    assert_eq!(running[tool_result::STATUS], STATUS_RUNNING);

    let (status, body) = post_json(
        app.clone(),
        &routes::checkpoint_resume_path(CHECKPOINT_ID),
        serde_json::json!({ "human_input": "approved" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "checkpoint resume failed: {body}");
    assert_eq!(body["status"], "resumed");

    let status_body = wait_for_workflow_status(app.clone(), execution_id, STATUS_SUCCEEDED).await;
    let workflow_status: serde_json::Value =
        serde_json::from_str(tool_text(&status_body)).expect("workflow status response JSON");
    assert_eq!(
        workflow_spawn_payload(&workflow_status)["result"],
        "approved"
    );

    let events = workflow_events(app, execution_id, 0, 50).await;
    let event_items = events["events"].as_array().expect("events array");
    assert!(
        event_items
            .iter()
            .any(|event| event["payload"]["kind"] == "execute_complete"),
        "expected execute_complete after checkpoint wake: {events}"
    );
    assert!(
        !event_items
            .iter()
            .any(|event| event["payload"]["kind"] == "turn_aborted"),
        "resume path should not abort: {events}"
    );
}

#[tokio::test]
async fn mcp_workflow_cancel_interrupts_parked_resume_without_late_success() {
    const CHECKPOINT_ID: &str = "mcp-workflow-cancel-parked-cp";

    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_path =
        write_workflow_fixture(temp.path(), "resume.air", &resume_air(CHECKPOINT_ID));
    let app = build_app(test_state().await);
    create_pending_checkpoint(app.clone(), CHECKPOINT_ID).await;

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_START,
            serde_json::json!({ "workflow_path": workflow_path }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "workflow start failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let started: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("workflow start response JSON");
    let execution_id = started[tool_result::EXECUTION_ID]
        .as_str()
        .expect("execution_id");

    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert_eq!(
        workflow_status_json(app.clone(), execution_id).await[tool_result::STATUS],
        STATUS_RUNNING
    );

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_CANCEL,
            serde_json::json!({ "execution_id": execution_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "workflow cancel failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let status_body = wait_for_workflow_status(app.clone(), execution_id, STATUS_FAILED).await;
    let workflow_status: serde_json::Value =
        serde_json::from_str(tool_text(&status_body)).expect("workflow status response JSON");
    assert!(
        workflow_status["error"]
            .as_str()
            .unwrap_or_default()
            .contains("apxm_workflow_cancel"),
        "expected cancellation error: {workflow_status}"
    );

    let (status, body) = post_json(
        app.clone(),
        &routes::checkpoint_resume_path(CHECKPOINT_ID),
        serde_json::json!({ "human_input": "too late" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "late checkpoint resume failed: {body}"
    );
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert_eq!(
        workflow_status_json(app.clone(), execution_id).await[tool_result::STATUS],
        STATUS_FAILED,
        "late checkpoint resume must not flip cancelled workflow to success"
    );

    let events = workflow_events(app.clone(), execution_id, 0, 50).await;
    let event_items = events["events"].as_array().expect("events array");
    assert!(
        event_items
            .iter()
            .any(|event| event["payload"]["kind"] == "turn_aborted"),
        "expected turn_aborted event: {events}"
    );
    assert!(
        !event_items
            .iter()
            .any(|event| event["payload"]["kind"] == "execute_complete"),
        "cancelled parked workflow must not emit execute_complete: {events}"
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_CANCEL,
            serde_json::json!({ "execution_id": execution_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "second cancel failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], true);
}

#[tokio::test]
async fn mcp_workflow_cancel_interrupts_in_flight_run() {
    const SLEEP_TOOL: &str = "fixture_sleep";

    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_path =
        write_workflow_fixture(temp.path(), "sleep.air", &sleep_tool_air(SLEEP_TOOL));
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSleepCapability::new(SLEEP_TOOL)))
        .expect("register sleep capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_START,
            serde_json::json!({ "workflow_path": workflow_path }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "workflow start failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let started: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("workflow start response JSON");
    let execution_id = started[tool_result::EXECUTION_ID]
        .as_str()
        .expect("execution_id");

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_CANCEL,
            serde_json::json!({ "execution_id": execution_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "workflow cancel failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let cancelled: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("workflow cancel response JSON");
    assert_eq!(cancelled["cancelled"], true);

    let status_body = wait_for_workflow_status(app.clone(), execution_id, STATUS_FAILED).await;
    let workflow_status: serde_json::Value =
        serde_json::from_str(tool_text(&status_body)).expect("workflow status response JSON");
    assert_eq!(workflow_status[tool_result::STATUS], STATUS_FAILED);
    assert!(
        workflow_status["error"]
            .as_str()
            .unwrap_or_default()
            .contains("apxm_workflow_cancel"),
        "expected cancellation error: {workflow_status}"
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_EVENTS,
            serde_json::json!({ "execution_id": execution_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "workflow events failed: {body}");
    let events: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("workflow events response JSON");
    assert!(
        events["events"]
            .as_array()
            .expect("events array")
            .iter()
            .any(|event| event["payload"]["kind"] == "turn_aborted"),
        "expected turn_aborted event: {events}"
    );
}

#[tokio::test]
async fn mcp_plan_as_graph_reports_missing_router_as_tool_error() {
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 26,
            "method": MCP_METHOD_TOOLS_CALL,
            "params": {
                (MCP_PARAM_NAME): MCP_TOOL_APXM_PLAN_AS_GRAPH,
                (MCP_PARAM_ARGUMENTS): {
                    (mcp_args::TASK): "audit this repository",
                    (mcp_args::EXECUTE): false
                }
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "MCP tool call failed: {body}");
    assert_eq!(body["result"]["isError"], true);
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("model router unavailable"),
        "expected router guidance: {body}"
    );
}

#[tokio::test]
async fn mcp_plan_as_graph_compiles_mock_model_plan() {
    let app = build_app(test_state_with_mock_plan_response(mock_yield_plan_response()).await);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_PLAN_AS_GRAPH,
            serde_json::json!({
                (mcp_args::TASK): FIXTURE_PLAN_TASK,
                (mcp_args::EXECUTE): false
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp plan-as-graph failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("plan response JSON");
    assert_eq!(response[tool_result::STATUS], mcp_status::COMPILED);
    assert!(
        response[tool_result::TRACE_ID]
            .as_str()
            .is_some_and(|trace| !trace.is_empty()),
        "trace_id missing: {response}"
    );
    assert!(
        response[tool_result::AIR_HASH]
            .as_str()
            .is_some_and(|hash| hash.starts_with(REDACTION_HASH_PREFIX_BLAKE3)),
        "air hash missing: {response}"
    );
    assert!(
        response[tool_result::ARTIFACT_HASH]
            .as_str()
            .is_some_and(|hash| hash.starts_with(REDACTION_HASH_PREFIX_BLAKE3)),
        "artifact hash missing: {response}"
    );
    assert_eq!(
        response[tool_result::PLAN][plan_field::NAME],
        FIXTURE_PLAN_NAME
    );
    assert_eq!(
        response[tool_result::SUMMARY][tool_result::EXECUTED_NODES],
        0
    );
}

#[tokio::test]
async fn mcp_plan_as_graph_normalizes_named_dependency_refs() {
    let app =
        build_app(test_state_with_mock_plan_response(mock_named_dependency_plan_response()).await);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_PLAN_AS_GRAPH,
            serde_json::json!({
                (mcp_args::TASK): FIXTURE_PLAN_TASK,
                (mcp_args::EXECUTE): false
            }),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "mcp named dependency normalization failed: {body}"
    );
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("plan response JSON");
    assert_eq!(
        response[tool_result::PLAN][plan_field::NODES][1][plan_field::DEPENDS_ON][0]
            [plan_field::NODE],
        1
    );
}

#[tokio::test]
async fn mcp_plan_as_graph_repairs_invalid_candidate_before_compile() {
    let backend = MockLLMBackend::static_response(mock_invalid_plan_response().to_string())
        .when_prompt_contains(
            FIXTURE_PLAN_REPAIR_MARKER,
            mock_yield_plan_response().to_string(),
        );
    let app = build_app(
        test_state_with_runtime_and_skill_roots(
            runtime_with_mock_plan_backend(backend).await,
            Vec::new(),
        )
        .await,
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_PLAN_AS_GRAPH,
            serde_json::json!({
                (mcp_args::TASK): FIXTURE_PLAN_TASK,
                (mcp_args::EXECUTE): false
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp plan repair failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("plan response JSON");
    assert_eq!(response[tool_result::STATUS], mcp_status::COMPILED);
    assert_eq!(
        response[tool_result::PLAN][plan_field::NODES][0][plan_field::OP],
        FIXTURE_PLAN_OP_YIELD
    );
}

#[tokio::test]
async fn mcp_plan_as_graph_normalizes_top_level_attribute_alias() {
    // A plan wrapped in a top-level `attr` alias with a stray `description`
    // field must compile on the first try via the
    // normalize_plan_top_level_attribute_aliases pass — without it, the
    // wrapper would burn a repair turn before serde could parse the graph.
    let app = build_app(
        test_state_with_mock_plan_response(mock_top_level_attr_alias_plan_response()).await,
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_PLAN_AS_GRAPH,
            serde_json::json!({
                (mcp_args::TASK): FIXTURE_PLAN_TASK,
                (mcp_args::EXECUTE): false
            }),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "mcp top-level attr-alias normalization failed: {body}"
    );
    assert_eq!(
        body[tool_result::RESULT][mcp_fields::IS_ERROR],
        false,
        "top-level attr-alias normalization returned an error: {body}"
    );
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("plan response JSON");
    assert_eq!(response[tool_result::STATUS], mcp_status::COMPILED);
    assert_eq!(
        response[tool_result::PLAN][plan_field::NAME],
        FIXTURE_PLAN_NAME
    );
    assert_eq!(
        response[tool_result::PLAN][plan_field::NODES][0][plan_field::OP],
        FIXTURE_PLAN_OP_YIELD
    );
    // The stray top-level field must not survive normalization.
    assert!(
        response[tool_result::PLAN].get("description").is_none(),
        "stray top-level field leaked into normalized plan: {response}"
    );
}

#[tokio::test]
async fn mcp_plan_as_graph_records_execution_under_trace_id() {
    let app = build_app(test_state_with_mock_plan_response(mock_yield_plan_response()).await);

    let (status, body) = post_json(
        app.clone(),
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_PLAN_AS_GRAPH,
            serde_json::json!({
                (mcp_args::TASK): FIXTURE_PLAN_TASK,
                (mcp_args::EXECUTE): true,
                (mcp_args::TRACE_ID): FIXTURE_PLAN_TRACE_ID
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp plan execution failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("plan response JSON");
    assert_eq!(response[tool_result::STATUS], mcp_status::EXECUTED);
    assert_eq!(
        response[tool_result::EXECUTION_ID],
        FIXTURE_PLAN_TRACE_ID,
        "execution_id should be the requested trace_id: {response}"
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_TRACE_FETCH,
            serde_json::json!({ (mcp_args::TRACE_ID): FIXTURE_PLAN_TRACE_ID }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp trace fetch failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let trace: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("trace response JSON");
    assert_eq!(trace[tool_result::STATUS], mcp_status::FOUND);
    assert_eq!(
        trace[tool_result::EXECUTION][tool_result::EXECUTION_ID],
        FIXTURE_PLAN_TRACE_ID
    );
    assert_eq!(
        trace[tool_result::EXECUTION][tool_result::SKILL_ID],
        plan_skill::ID
    );
}

#[tokio::test]
async fn mcp_plan_as_graph_rejects_unsafe_generated_direct_tool() {
    let state =
        test_state_with_mock_plan_response(mock_inv_tool_plan_response(FIXTURE_WRITE_TOOL)).await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(
            FIXTURE_WRITE_TOOL,
        )))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_PLAN_AS_GRAPH,
            serde_json::json!({
                (mcp_args::TASK): FIXTURE_PLAN_TASK,
                (mcp_args::EXECUTE): true
            }),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "MCP tool errors should stay JSON-RPC 200: {body}"
    );
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], true);
    assert!(
        tool_text(&body).contains(ERROR_MCP_AGENT_SAFE),
        "expected generated-plan side-effect rejection: {body}"
    );
}

#[tokio::test]
async fn mcp_plan_as_graph_allows_sandboxed_generated_tool() {
    let mut runtime = runtime_with_mock_plan_backend(MockLLMBackend::static_response(
        mock_inv_tool_plan_response(FIXTURE_WRITE_TOOL).to_string(),
    ))
    .await;
    runtime.set_sandbox_registry(Arc::new(fixture_sandbox_registry()));
    runtime
        .capability_system()
        .register(Arc::new(FixtureSandboxedCapability::new(
            FIXTURE_WRITE_TOOL,
        )))
        .expect("register fixture sandboxed capability");
    let app = build_app(test_state_with_runtime_and_skill_roots(runtime, Vec::new()).await);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_PLAN_AS_GRAPH,
            serde_json::json!({
                (mcp_args::TASK): FIXTURE_PLAN_TASK,
                (mcp_args::EXECUTE): true,
                (mcp_args::TRACE_ID): FIXTURE_PLAN_SANDBOX_TRACE_ID
            }),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "sandboxed generated plan should return MCP 200: {body}"
    );
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("plan response JSON");
    assert_eq!(response[tool_result::STATUS], mcp_status::EXECUTED);
    assert_eq!(
        response[tool_result::EXECUTION_ID],
        FIXTURE_PLAN_SANDBOX_TRACE_ID
    );
}

#[tokio::test]
async fn mcp_plan_as_graph_rejects_unsafe_trace_id_before_emission() {
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_PLAN_AS_GRAPH,
            serde_json::json!({
                (mcp_args::TASK): FIXTURE_PLAN_TASK,
                (mcp_args::EXECUTE): false,
                (mcp_args::TRACE_ID): FIXTURE_PLAN_INVALID_TRACE_ID
            }),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "mcp plan trace validation failed: {body}"
    );
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], true);
    assert!(
        tool_text(&body).contains(admission_error::TRACE_ID_UNSAFE),
        "expected trace_id validation error: {body}"
    );
}

#[tokio::test]
async fn mcp_trace_fetch_returns_execution_store_summary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = ExecutionStore::new();
    let session_dir = temp.path().to_string_lossy().to_string();
    let record = store.start_skill_execution(
        FIXTURE_SKILL_ID,
        FIXTURE_SKILL_VERSION,
        FIXTURE_SESSION_ID,
        &session_dir,
    );
    let trace_id = record.execution_id.clone();
    let app = build_app(test_state_with_skill_roots_and_execution_store(Vec::new(), store).await);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_TRACE_FETCH,
            serde_json::json!({ (mcp_args::TRACE_ID): trace_id }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp trace fetch failed: {body}");
    assert_eq!(body["result"]["isError"], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("trace response JSON");
    assert_eq!(response[tool_result::STATUS], mcp_status::FOUND);
    assert_eq!(
        response[tool_result::EXECUTION][tool_result::EXECUTION_ID],
        record.execution_id
    );
    assert_eq!(
        response[tool_result::EXECUTION][tool_result::SKILL_ID],
        FIXTURE_SKILL_ID
    );
}

#[tokio::test]
async fn mcp_aam_recall_returns_matching_beliefs() {
    let state = test_state().await;
    state.runtime.aam().set_belief(
        FIXTURE_AAM_KEY.to_string(),
        Value::String(FIXTURE_AAM_VALUE.to_string()),
        TransitionLabel::custom("fixture"),
    );
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_AAM_RECALL,
            serde_json::json!({ (mcp_args::QUERY): FIXTURE_AAM_QUERY }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp aam recall failed: {body}");
    assert_eq!(body["result"]["isError"], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("AAM response JSON");
    let beliefs = response[tool_result::AAM][tool_result::BELIEFS]
        .as_array()
        .expect("beliefs array");
    assert!(
        beliefs
            .iter()
            .any(|belief| belief[tool_result::NAME] == FIXTURE_AAM_KEY),
        "beliefs: {response}"
    );
}

#[tokio::test]
async fn mcp_evidence_lookup_reads_repo_local_apxm_docs() {
    let app = build_app(test_state().await);
    let evidence_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("workspace root")
        .join(FIXTURE_EVIDENCE_PATH);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_EVIDENCE_LOOKUP,
            serde_json::json!({
                (mcp_args::PATH): evidence_path,
                (mcp_args::QUERY): FIXTURE_EVIDENCE_QUERY,
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp evidence lookup failed: {body}");
    assert_eq!(body["result"]["isError"], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("evidence response JSON");
    let matches = response[tool_result::MATCHES]
        .as_array()
        .expect("evidence matches array");
    assert_eq!(matches.len(), 1, "matches: {response}");
    assert!(
        matches[0][mcp_args::PATH]
            .as_str()
            .unwrap_or_default()
            .ends_with("MCP-SERVER-PLAN.md"),
        "matches: {response}"
    );
}

#[tokio::test]
async fn mcp_capability_list_returns_registered_runtime_capabilities() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_CAPABILITY_LIST,
            serde_json::json!({ (mcp_args::QUERY): FIXTURE_TOOL }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp capability list failed: {body}");
    assert_eq!(body["result"]["isError"], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("capability response JSON");
    let capabilities = response[tool_result::CAPABILITIES]
        .as_array()
        .expect("capabilities array");
    assert!(
        capabilities
            .iter()
            .any(|capability| capability[tool_result::NAME] == FIXTURE_TOOL),
        "capabilities: {response}"
    );
    assert!(response[tool_result::BACKENDS].is_array(), "{response}");
}

#[tokio::test]
async fn mcp_tools_list_exposes_only_read_only_generic_capabilities() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register read-only fixture capability");
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(
            FIXTURE_WRITE_TOOL,
        )))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 24,
            "method": MCP_METHOD_TOOLS_LIST,
            "params": {}
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "MCP tools/list failed: {body}");
    let tools = body["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&FIXTURE_TOOL), "tools: {body}");
    assert!(!names.contains(&FIXTURE_WRITE_TOOL), "tools: {body}");
}

#[tokio::test]
async fn mcp_skill_get_returns_installed_skill_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_valid_skill(temp.path(), FIXTURE_PACKAGE_DIR);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let mut arguments = serde_json::Map::new();
    arguments.insert(
        MCP_ARG_ID.to_string(),
        serde_json::Value::String(FIXTURE_SKILL_ID.to_string()),
    );
    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_SKILL_GET,
            serde_json::Value::Object(arguments),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp skill get failed: {body}");
    assert_eq!(body["result"]["isError"], false);
    let record: serde_json::Value = serde_json::from_str(tool_text(&body)).expect("record json");
    assert_eq!(record["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(record["validation"]["status"], "valid");
}

#[tokio::test]
async fn mcp_skill_call_executes_static_server_owned_skill() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        MCP_ARG_ID.to_string(),
        serde_json::Value::String(versioned_skill_id()),
    );
    arguments.insert(
        MCP_ARG_SESSION_ID.to_string(),
        serde_json::Value::String(FIXTURE_SESSION_ID.to_string()),
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_SKILL_CALL,
            serde_json::Value::Object(arguments),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "mcp skill call failed: {body}");
    assert_eq!(body["result"]["isError"], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("MCP execute response JSON");
    assert!(response["execution_id"].is_string());
    assert_eq!(response["content"], FIXTURE_OUTPUT);
    let session_dir = response["session_dir"].as_str().expect("session dir");
    assert!(
        session_dir.contains("/skills/"),
        "session dir: {session_dir}"
    );
    assert!(
        session_dir.ends_with(FIXTURE_SESSION_ID),
        "session dir: {session_dir}"
    );
}

#[tokio::test]
async fn mcp_skill_call_rejects_undeclared_inv_tool_with_tool_error() {
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
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        MCP_ARG_ID.to_string(),
        serde_json::Value::String(versioned_skill_id()),
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_SKILL_CALL,
            serde_json::Value::Object(arguments),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "MCP tool errors should stay JSON-RPC 200: {body}"
    );
    assert_eq!(body["result"]["isError"], true);
    assert!(
        tool_text(&body).contains(ERROR_UNDECLARED_CAPABILITY),
        "expected undeclared INV_TOOL rejection: {body}"
    );
}

#[tokio::test]
async fn mcp_skill_call_rejects_python_backed_inv_tool_with_tool_error() {
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
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        MCP_ARG_ID.to_string(),
        serde_json::Value::String(versioned_skill_id()),
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_SKILL_CALL,
            serde_json::Value::Object(arguments),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "MCP tool errors should stay JSON-RPC 200: {body}"
    );
    assert_eq!(body["result"]["isError"], true);
    assert!(
        tool_text(&body).contains(ERROR_PYTHON_INV_TOOL_UNSUPPORTED),
        "expected python-backed INV_TOOL rejection: {body}"
    );
}

#[tokio::test]
async fn mcp_skill_call_rejects_non_read_only_side_effect_policy_with_tool_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = skill_artifact_bytes(AISOperationType::ConstStr);
    write_side_effect_policy_skill_with_artifact(temp.path(), &artifact, "write_files");
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        MCP_ARG_ID.to_string(),
        serde_json::Value::String(versioned_skill_id()),
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_SKILL_CALL,
            serde_json::Value::Object(arguments),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "MCP tool errors should stay JSON-RPC 200: {body}"
    );
    assert_eq!(body["result"]["isError"], true);
    assert!(
        tool_text(&body).contains(ERROR_SIDE_EFFECT_POLICY_UNSUPPORTED),
        "expected side-effect policy rejection: {body}"
    );
}

#[tokio::test]
async fn mcp_skill_call_allows_sandboxed_side_effectful_capability_after_preflight() {
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
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        MCP_ARG_ID.to_string(),
        serde_json::Value::String(versioned_skill_id()),
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_SKILL_CALL,
            serde_json::Value::Object(arguments),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "MCP tool errors should stay JSON-RPC 200: {body}"
    );
    assert_eq!(body["result"]["isError"], false);
    let response: serde_json::Value =
        serde_json::from_str(tool_text(&body)).expect("MCP execute response JSON");
    assert_eq!(response["content"], FIXTURE_OUTPUT);
}

#[tokio::test]
async fn mcp_skill_call_rejects_sandboxed_policy_without_backend_preflight() {
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
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        MCP_ARG_ID.to_string(),
        serde_json::Value::String(versioned_skill_id()),
    );

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_SKILL_CALL,
            serde_json::Value::Object(arguments),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "MCP tool errors should stay JSON-RPC 200: {body}"
    );
    assert_eq!(body["result"]["isError"], true);
    assert!(
        tool_text(&body).contains(ERROR_SANDBOX_PREFLIGHT),
        "expected sandbox preflight rejection: {body}"
    );
}

#[tokio::test]
async fn mcp_unknown_method_returns_error_code() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
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

#[tokio::test]
async fn mcp_tools_call_validates_registered_capability_arguments() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureRequiredArgCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(FIXTURE_TOOL, serde_json::json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "expected MCP response: {body}");
    assert_eq!(body["result"]["isError"], true);
    assert!(
        tool_text(&body).contains("Input validation failed"),
        "expected capability schema validation error: {body}"
    );
}

#[tokio::test]
async fn mcp_tools_call_allows_read_only_capability() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(FIXTURE_TOOL, serde_json::json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "expected MCP response: {body}");
    assert_eq!(body["result"]["isError"], false);
    assert!(
        tool_text(&body).contains(FIXTURE_OUTPUT),
        "expected read-only capability output: {body}"
    );
}

#[tokio::test]
async fn mcp_tools_call_rejects_non_read_only_direct_capability() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(FIXTURE_TOOL)))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(FIXTURE_TOOL, serde_json::json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "expected MCP response: {body}");
    assert_eq!(body["result"]["isError"], true);
    assert!(
        tool_text(&body).contains(ERROR_MCP_AGENT_SAFE),
        "expected MCP agent safety rejection: {body}"
    );
}

#[tokio::test]
async fn mcp_tools_call_routes_sandboxed_capability_through_registry() {
    let mut runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    runtime.set_sandbox_registry(Arc::new(fixture_sandbox_registry()));
    runtime
        .capability_system()
        .register(Arc::new(FixtureSandboxedCapability::new(FIXTURE_TOOL)))
        .expect("register fixture sandboxed capability");
    let app = build_app(test_state_with_runtime_and_skill_roots(runtime, Vec::new()).await);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(FIXTURE_TOOL, serde_json::json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "expected MCP response: {body}");
    assert_eq!(body["result"]["isError"], false);
    assert!(
        tool_text(&body).contains(FIXTURE_OUTPUT),
        "expected sandboxed capability output: {body}"
    );
}

#[tokio::test]
async fn mcp_tools_call_rejects_degraded_sandboxed_capability() {
    let mut runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    runtime.set_sandbox_registry(Arc::new(fixture_degraded_sandbox_registry()));
    runtime
        .capability_system()
        .register(Arc::new(FixtureSandboxedCapability::new(FIXTURE_TOOL)))
        .expect("register fixture sandboxed capability");
    let app = build_app(test_state_with_runtime_and_skill_roots(runtime, Vec::new()).await);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(FIXTURE_TOOL, serde_json::json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "expected MCP response: {body}");
    assert_eq!(body["result"]["isError"], true);
    assert!(
        tool_text(&body).contains(ERROR_SANDBOX_DEGRADED),
        "expected degraded sandbox rejection: {body}"
    );
}

#[tokio::test]
async fn mcp_tools_call_rejects_non_object_arguments() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(FIXTURE_TOOL, serde_json::json!(["not", "an", "object"])),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "expected MCP response: {body}");
    assert_eq!(body["result"]["isError"], true);
    assert!(
        tool_text(&body).contains("arguments must be an object"),
        "expected argument shape rejection: {body}"
    );
}

// ── Skill-root discovery (CLI args + APXM_SKILL_ROOTS env var) ──────────────
//
// These tests exercise `parse_skill_roots` and `prepend_builtin_skill_root`,
// which are the same functions used by `apxm-server` (HTTP) startup and
// `apxm-mcp-server` (stdio) `discovered_skill_roots`. The env-var path mutates
// process global state, so tests that touch APXM_SKILL_ROOTS serialize through
// `SKILL_ROOTS_ENV_LOCK` and restore the previous value when they finish.

const APXM_SKILL_ROOTS_ENV: &str = "APXM_SKILL_ROOTS";
const USER_SKILL_PACKAGE_DIR: &str = "user-skill-fixture";
const USER_SKILL_ID: &str = "user-skill-fixture";
const USER_SKILL_VERSION: &str = "0.1.0";
const USER_SKILL_SOURCE: &str = "# User Skill Fixture\n";
const ALT_USER_SKILL_PACKAGE_DIR: &str = "alt-user-skill";
const ALT_USER_SKILL_ID: &str = "alt-user-skill";
const ALT_USER_SKILL_VERSION: &str = "0.2.0";
const ALT_USER_SKILL_SOURCE: &str = "# Alt User Skill\n";
const BUILTIN_SKILL_ID: &str = "apxm-plan-as-graph";
const COLLIDING_USER_VERSION: &str = "9.9.9-user-override";
const COLLIDING_USER_SOURCE: &str = "# User override of apxm-plan-as-graph\n";

static SKILL_ROOTS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct SkillRootsEnvGuard {
    prior: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl SkillRootsEnvGuard {
    #[allow(unsafe_code)]
    fn set(value: Option<&std::ffi::OsStr>) -> Self {
        let lock = SKILL_ROOTS_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let prior = std::env::var_os(APXM_SKILL_ROOTS_ENV);
        // SAFETY: tests that touch APXM_SKILL_ROOTS hold SKILL_ROOTS_ENV_LOCK,
        // so no other test thread observes the mutation. The previous value
        // is restored on drop. `std::env::set_var`/`remove_var` require
        // `unsafe` in Rust 2024 because they are process-global.
        unsafe {
            match value {
                Some(value) => std::env::set_var(APXM_SKILL_ROOTS_ENV, value),
                None => std::env::remove_var(APXM_SKILL_ROOTS_ENV),
            }
        }
        Self { prior, _lock: lock }
    }
}

impl Drop for SkillRootsEnvGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: lock is still held; restore prior value (or remove if unset).
        unsafe {
            match self.prior.take() {
                Some(value) => std::env::set_var(APXM_SKILL_ROOTS_ENV, value),
                None => std::env::remove_var(APXM_SKILL_ROOTS_ENV),
            }
        }
    }
}

fn write_minimal_user_skill(
    root: &std::path::Path,
    package_dir: &str,
    skill_id: &str,
    version: &str,
    source: &str,
) {
    let skill_dir = root.join(package_dir);
    std::fs::create_dir_all(&skill_dir).expect("user skill dir");
    std::fs::write(
        skill_dir.join("skill.toml"),
        format!(
            r#"skill_id = "{skill_id}"
version = "{version}"
display_name = "User Skill {skill_id}"
description = "User-provided skill fixture"
entry_flow = "noop"
required_capabilities = []
timeout_ms = 60000
token_limit = 4096
side_effect_policy = "read_only"
"#
        ),
    )
    .expect("user manifest");
    std::fs::write(skill_dir.join(FILE_SKILL_SOURCE), source).expect("user SKILL.md");
}

async fn list_mcp_resource_uris(app: Router) -> Vec<String> {
    let (status, body) = post_json(
        app,
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 42,
            "method": MCP_METHOD_RESOURCES_LIST,
            "params": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "MCP resources/list failed: {body}");
    body["result"][mcp_fields::RESOURCES]
        .as_array()
        .expect("resources array")
        .iter()
        .filter_map(|resource| resource[mcp_fields::URI].as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn mcp_resources_list_includes_user_skill_from_cli_root() {
    let _guard = SkillRootsEnvGuard::set(None);
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_user_skill(
        temp.path(),
        USER_SKILL_PACKAGE_DIR,
        USER_SKILL_ID,
        USER_SKILL_VERSION,
        USER_SKILL_SOURCE,
    );
    let args = vec![
        "apxm-mcp-server".to_string(),
        "--skill-root".to_string(),
        temp.path().to_string_lossy().to_string(),
    ];
    let roots = prepend_builtin_skill_root(parse_skill_roots(&args));

    let app = build_app(test_state_with_skill_roots(roots).await);
    let uris = list_mcp_resource_uris(app).await;
    let user_uri = skill_resource_uri(USER_SKILL_ID, FILE_SKILL_SOURCE);
    let builtin_uri = skill_resource_uri(BUILTIN_SKILL_ID, FILE_SKILL_SOURCE);
    assert!(
        uris.contains(&user_uri),
        "user skill missing from CLI root resources: {uris:?}"
    );
    assert!(
        uris.contains(&builtin_uri),
        "builtin skill missing from resources: {uris:?}"
    );
}

#[tokio::test]
async fn mcp_resources_list_includes_user_skill_from_env_var() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_user_skill(
        temp.path(),
        USER_SKILL_PACKAGE_DIR,
        USER_SKILL_ID,
        USER_SKILL_VERSION,
        USER_SKILL_SOURCE,
    );
    let _guard = SkillRootsEnvGuard::set(Some(temp.path().as_os_str()));
    let args = vec!["apxm-mcp-server".to_string()];
    let roots = prepend_builtin_skill_root(parse_skill_roots(&args));

    let app = build_app(test_state_with_skill_roots(roots).await);
    let uris = list_mcp_resource_uris(app).await;
    let user_uri = skill_resource_uri(USER_SKILL_ID, FILE_SKILL_SOURCE);
    let builtin_uri = skill_resource_uri(BUILTIN_SKILL_ID, FILE_SKILL_SOURCE);
    assert!(
        uris.contains(&user_uri),
        "user skill missing from APXM_SKILL_ROOTS env resources: {uris:?}"
    );
    assert!(
        uris.contains(&builtin_uri),
        "builtin skill missing from resources: {uris:?}"
    );
}

#[tokio::test]
async fn mcp_resources_list_combines_cli_and_env_roots() {
    let cli_root = tempfile::tempdir().expect("cli tempdir");
    let env_root = tempfile::tempdir().expect("env tempdir");
    write_minimal_user_skill(
        cli_root.path(),
        USER_SKILL_PACKAGE_DIR,
        USER_SKILL_ID,
        USER_SKILL_VERSION,
        USER_SKILL_SOURCE,
    );
    write_minimal_user_skill(
        env_root.path(),
        ALT_USER_SKILL_PACKAGE_DIR,
        ALT_USER_SKILL_ID,
        ALT_USER_SKILL_VERSION,
        ALT_USER_SKILL_SOURCE,
    );
    let _guard = SkillRootsEnvGuard::set(Some(env_root.path().as_os_str()));
    let args = vec![
        "apxm-mcp-server".to_string(),
        "--skill-root".to_string(),
        cli_root.path().to_string_lossy().to_string(),
    ];
    let roots = prepend_builtin_skill_root(parse_skill_roots(&args));

    let app = build_app(test_state_with_skill_roots(roots).await);
    let uris = list_mcp_resource_uris(app).await;
    let cli_uri = skill_resource_uri(USER_SKILL_ID, FILE_SKILL_SOURCE);
    let env_uri = skill_resource_uri(ALT_USER_SKILL_ID, FILE_SKILL_SOURCE);
    let builtin_uri = skill_resource_uri(BUILTIN_SKILL_ID, FILE_SKILL_SOURCE);
    assert!(
        uris.contains(&cli_uri),
        "CLI-root user skill missing: {uris:?}"
    );
    assert!(
        uris.contains(&env_uri),
        "env-root user skill missing: {uris:?}"
    );
    assert!(
        uris.contains(&builtin_uri),
        "builtin skill missing: {uris:?}"
    );
}

#[tokio::test]
async fn mcp_resources_list_builtin_wins_on_id_collision() {
    // User attempts to shadow the bundled `apxm-plan-as-graph` skill by
    // contributing a package with the same skill_id (different version) via
    // `--skill-root`. The builtin must still appear, and listings must
    // disambiguate by version so the bundled artifact is not silently
    // overridden.
    let _guard = SkillRootsEnvGuard::set(None);
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_user_skill(
        temp.path(),
        "user-plan-override",
        BUILTIN_SKILL_ID,
        COLLIDING_USER_VERSION,
        COLLIDING_USER_SOURCE,
    );
    let args = vec![
        "apxm-mcp-server".to_string(),
        "--skill-root".to_string(),
        temp.path().to_string_lossy().to_string(),
    ];
    let roots = prepend_builtin_skill_root(parse_skill_roots(&args));

    let state = test_state_with_skill_roots(roots).await;
    let library = state.skill_library.clone();
    let app = build_app(state);

    let uris = list_mcp_resource_uris(app.clone()).await;
    let user_versioned_id = format!("{BUILTIN_SKILL_ID}@{COLLIDING_USER_VERSION}");
    let user_versioned_uri = skill_resource_uri(&user_versioned_id, FILE_SKILL_SOURCE);
    let unversioned_uri = skill_resource_uri(BUILTIN_SKILL_ID, FILE_SKILL_SOURCE);
    assert!(
        uris.contains(&user_versioned_uri),
        "expected versioned user URI on id collision: {uris:?}"
    );
    assert!(
        uris.iter().any(
            |uri| uri.starts_with(&format!("skill://{BUILTIN_SKILL_ID}@"))
                && uri.ends_with(&format!("/{FILE_SKILL_SOURCE}"))
                && uri != &user_versioned_uri
        ),
        "expected builtin to appear as a separate versioned URI on collision: {uris:?}"
    );
    assert!(
        !uris.contains(&unversioned_uri),
        "unversioned URI must not appear when skill_id is duplicated: {uris:?}"
    );

    // Resolving the unversioned URI must be ambiguous — the builtin is not
    // silently overridden by the user package.
    let resolve_error = library
        .resolve_skill_uri(&unversioned_uri)
        .expect_err("ambiguous unversioned resolve");
    assert!(
        resolve_error.to_string().contains("multiple versions"),
        "expected ambiguity error, got: {resolve_error}"
    );

    // The user-contributed version is still readable via its versioned URI;
    // its contents are the user-supplied source (not the builtin) — the
    // builtin remains addressable under its own version, which is what
    // "builtin wins" means here: the user cannot silently replace it.
    let user_read = library
        .resolve_skill_uri(&user_versioned_uri)
        .expect("user versioned read");
    assert_eq!(user_read.text, COLLIDING_USER_SOURCE);
}

fn write_workflow_fixture(
    root: &std::path::Path,
    graph_name: &str,
    graph_air: &str,
) -> std::path::PathBuf {
    let graph_path = root.join(graph_name);
    std::fs::write(&graph_path, graph_air).expect("write workflow graph");
    let workflow_path = root.join("workflow.apxmw");
    std::fs::write(
        &workflow_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": "mcp_workflow_fixture",
            "graphs": [
                { "id": "step", "path": graph_name }
            ],
            "output": "{{step.output}}"
        }))
        .expect("serialize workflow fixture"),
    )
    .expect("write workflow fixture");
    workflow_path
}

fn write_parallel_workflow_fixture(
    root: &std::path::Path,
    steps: &[(&str, &str, &str)],
    output: &str,
) -> std::path::PathBuf {
    let graphs: Vec<serde_json::Value> = steps
        .iter()
        .map(|(id, graph_name, graph_air)| {
            std::fs::write(root.join(graph_name), graph_air).expect("write workflow graph");
            serde_json::json!({ "id": id, "path": graph_name })
        })
        .collect();
    let workflow_path = root.join("workflow.apxmw");
    std::fs::write(
        &workflow_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": "mcp_parallel_workflow_fixture",
            "graphs": graphs,
            "output": output
        }))
        .expect("serialize workflow fixture"),
    )
    .expect("write workflow fixture");
    workflow_path
}

fn tool_air(capability: &str) -> String {
    format!(
        r#"module {{
  func.func @main() -> !ais.token attributes {{ais.entry}} {{
    %reg = ais.register_capability "{capability}" {{description = "fixture tool"}} : !ais.token
    %tool = ais.inv_tool "{capability}" ("{{}}") [%reg : !ais.token] : !ais.token
    func.return %tool : !ais.token
  }}
}}
"#
    )
}

fn sleep_tool_air(capability: &str) -> String {
    format!(
        r#"module {{
  func.func @main() -> !ais.token attributes {{ais.entry}} {{
    %reg = ais.register_capability "{capability}" {{description = "fixture sleep tool"}} : !ais.token
    %tool = ais.inv_tool "{capability}" ("{{}}") [%reg : !ais.token] : !ais.token
    func.return %tool : !ais.token
  }}
}}
"#
    )
}

fn resume_air(checkpoint_id: &str) -> String {
    format!(
        r#"module {{
  func.func @main() -> !ais.token attributes {{ais.entry}} {{
    %resumed = ais.resume "{checkpoint_id}" : !ais.token
    func.return %resumed : !ais.token
  }}
}}
"#
    )
}

async fn create_pending_checkpoint(app: Router, checkpoint_id: &str) {
    let (status, body) = post_json(
        app,
        routes::CHECKPOINTS,
        serde_json::json!({
            "checkpoint_id": checkpoint_id,
            "message": "fixture workflow checkpoint",
            "display_data": null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "checkpoint create failed: {body}");
    assert_eq!(body["checkpoint_id"], checkpoint_id);
    assert_eq!(body["status"], "pending");
}

async fn workflow_status_json(app: Router, execution_id: &str) -> serde_json::Value {
    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_STATUS,
            serde_json::json!({ "execution_id": execution_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "workflow status failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    serde_json::from_str(tool_text(&body)).expect("workflow status JSON")
}

async fn workflow_events(
    app: Router,
    execution_id: &str,
    since: u64,
    limit: usize,
) -> serde_json::Value {
    let (status, body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_WORKFLOW_EVENTS,
            serde_json::json!({
                "execution_id": execution_id,
                "since": since,
                "limit": limit
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "workflow events failed: {body}");
    assert_eq!(body[tool_result::RESULT][mcp_fields::IS_ERROR], false);
    serde_json::from_str(tool_text(&body)).expect("workflow events response JSON")
}

fn workflow_spawn_payload(workflow_status: &serde_json::Value) -> &serde_json::Value {
    workflow_status["result"]["results"]
        .as_object()
        .expect("result map")
        .values()
        .find(|value| value.get("result").is_some())
        .expect("workflow spawn payload")
}

async fn wait_for_workflow_status(
    app: Router,
    execution_id: &str,
    expected_status: &str,
) -> serde_json::Value {
    let mut last_body = serde_json::Value::Null;
    for _ in 0..100 {
        let (status, body) = post_json(
            app.clone(),
            routes::MCP,
            mcp_call(
                MCP_TOOL_APXM_WORKFLOW_STATUS,
                serde_json::json!({ "execution_id": execution_id }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "workflow status failed: {body}");
        if body[tool_result::RESULT][mcp_fields::IS_ERROR] == false {
            let status_json: serde_json::Value =
                serde_json::from_str(tool_text(&body)).expect("workflow status JSON");
            if status_json[tool_result::STATUS] == expected_status {
                return body;
            }
        }
        last_body = body;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("workflow did not reach status {expected_status}: {last_body}");
}

struct WorkflowBarrier {
    expected: usize,
    current: AtomicUsize,
    max_seen: AtomicUsize,
    notify: tokio::sync::Notify,
}

impl WorkflowBarrier {
    fn new(expected: usize) -> Self {
        Self {
            expected,
            current: AtomicUsize::new(0),
            max_seen: AtomicUsize::new(0),
            notify: tokio::sync::Notify::new(),
        }
    }

    async fn enter(&self) {
        let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_seen.fetch_max(current, Ordering::SeqCst);
        if current >= self.expected {
            self.notify.notify_waiters();
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            self.notify.notified(),
        )
        .await;
    }

    fn exit(&self) {
        self.current.fetch_sub(1, Ordering::SeqCst);
    }
}

struct FixtureBarrierCapability {
    metadata: CapabilityMetadata,
    label: String,
    barrier: Arc<WorkflowBarrier>,
}

impl FixtureBarrierCapability {
    fn new(name: &str, label: &str, barrier: Arc<WorkflowBarrier>) -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                name,
                "Fixture workflow barrier capability",
                serde_json::json!({ "type": "object", "properties": {} }),
            )
            .with_returns("string")
            .with_read_only(),
            label: label.to_string(),
            barrier,
        }
    }
}

#[async_trait]
impl CapabilityExecutor for FixtureBarrierCapability {
    async fn execute(&self, _args: HashMap<String, Value>) -> Result<Value, RuntimeError> {
        self.barrier.enter().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        self.barrier.exit();
        Ok(Value::String(self.label.clone()))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

struct FixtureSleepCapability {
    metadata: CapabilityMetadata,
}

impl FixtureSleepCapability {
    fn new(name: &str) -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                name,
                "Fixture slow read-only capability",
                serde_json::json!({ "type": "object", "properties": {} }),
            )
            .with_returns("string")
            .with_read_only(),
        }
    }
}

#[async_trait]
impl CapabilityExecutor for FixtureSleepCapability {
    async fn execute(&self, _args: HashMap<String, Value>) -> Result<Value, RuntimeError> {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        Ok(Value::String(FIXTURE_OUTPUT.to_string()))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}
