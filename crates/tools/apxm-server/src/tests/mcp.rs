use super::*;

#[tokio::test]
async fn mcp_initialize_returns_protocol_version() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        ROUTE_MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 1,
            "method": MCP_METHOD_INITIALIZE,
            "params": { "protocolVersion": apxm_core::constants::protocols::MCP_VERSION }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "mcp initialize failed: {body}");
    assert_eq!(body["jsonrpc"], MCP_JSONRPC_VERSION);
    assert_eq!(body["id"], 1);
    assert!(body["result"].is_object(), "expected result object: {body}");
    let version = body["result"]["protocolVersion"].as_str().unwrap_or("");
    assert!(!version.is_empty(), "protocolVersion missing: {body}");
}

#[tokio::test]
async fn mcp_tools_list_returns_array() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        ROUTE_MCP,
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
async fn mcp_tools_list_includes_skill_inventory_tools() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        ROUTE_MCP,
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
        .register(Arc::new(FixtureSideEffectCapability::new("write_file")))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        ROUTE_MCP,
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
    assert!(!names.contains(&"write_file"), "tools: {body}");
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
        ROUTE_MCP,
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
