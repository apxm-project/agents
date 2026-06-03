use super::*;

const ERROR_WRITE_NOT_GRANTED: &str = "performs writes and was not granted";
const ERROR_RAW_PYTHON_TOOL_HANDLERS: &str = "python-backed tool handlers";
const ERROR_TOOLS_ENABLED_ALL: &str = "tools_enabled=true";
const ERROR_NOT_READ_ONLY: &str = "not read-only";
const FIXTURE_TOOL_GROUP: &str = "filesystem";

#[tokio::test]
async fn execute_invalid_air_returns_400() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        routes::EXECUTE,
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
    let (status, _body) = post_json(app, routes::EXECUTE, serde_json::json!({})).await;
    // Missing air field -> 400 or 422
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400/422 for empty request, got {status}"
    );
}

#[tokio::test]
async fn execute_allows_read_only_inv_tool() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": inv_tool_air(FIXTURE_TOOL)
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "execute failed: {body}");
    assert_eq!(body["content"], FIXTURE_OUTPUT);
}

#[tokio::test]
async fn execute_rejects_unknown_inv_tool_capability() {
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": inv_tool_air(FIXTURE_TOOL)
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_CAPABILITY_NOT_REGISTERED),
        "expected unknown capability rejection: {body}"
    );
}

#[tokio::test]
async fn execute_stream_rejects_unknown_inv_tool_capability() {
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app,
        routes::EXECUTE_STREAM,
        serde_json::json!({
            "air": inv_tool_air(FIXTURE_TOOL)
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_CAPABILITY_NOT_REGISTERED),
        "expected unknown capability rejection: {body}"
    );
}

#[tokio::test]
async fn execute_rejects_non_read_only_direct_inv_tool() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(FIXTURE_TOOL)))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": inv_tool_air(FIXTURE_TOOL)
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_WRITE_NOT_GRANTED),
        "expected ungranted write rejection: {body}"
    );
}

#[tokio::test]
async fn execute_allows_granted_direct_inv_tool() {
    // A non-read-only (Direct) capability runs when the caller explicitly grants
    // it via admit_capabilities — the consent channel the studio drives.
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(FIXTURE_TOOL)))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": inv_tool_air(FIXTURE_TOOL),
            "admit_capabilities": [FIXTURE_TOOL],
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "granted write execute failed: {body}");
    assert_eq!(body["content"], FIXTURE_OUTPUT);
}

#[tokio::test]
async fn execute_rejects_direct_inv_tool_when_admit_lists_other_capability() {
    // Granting an unrelated capability must not admit the write the graph uses.
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(FIXTURE_TOOL)))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": inv_tool_air(FIXTURE_TOOL),
            "admit_capabilities": ["some.other.capability"],
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_WRITE_NOT_GRANTED),
        "mismatched grant must not admit the write: {body}"
    );
}

#[tokio::test]
async fn execute_allows_sandboxed_inv_tool_after_preflight() {
    let mut runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    runtime.set_sandbox_registry(Arc::new(fixture_sandbox_registry()));
    runtime
        .capability_system()
        .register(Arc::new(FixtureSandboxedCapability::new(FIXTURE_TOOL)))
        .expect("register fixture sandboxed capability");
    let app = build_app(test_state_with_runtime_and_skill_roots(runtime, vec![]).await);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": inv_tool_air(FIXTURE_TOOL)
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "execute failed: {body}");
    assert_eq!(body["content"], FIXTURE_OUTPUT);
}

#[tokio::test]
async fn execute_rejects_python_handler_tool_attr() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": python_handler_inv_tool_air(FIXTURE_TOOL)
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_RAW_PYTHON_TOOL_HANDLERS),
        "expected python handler rejection: {body}"
    );
}

#[tokio::test]
async fn execute_rejects_ask_tools_enabled_when_any_capability_is_not_read_only() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(FIXTURE_TOOL)))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": ask_air(tools_enabled_all_attrs())
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_TOOLS_ENABLED_ALL),
        "expected tools_enabled rejection: {body}"
    );
}

#[tokio::test]
async fn execute_rejects_explicit_non_read_only_ask_tool() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(FIXTURE_TOOL)))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": ask_air(&explicit_tool_attrs(FIXTURE_TOOL))
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_NOT_READ_ONLY),
        "expected explicit non-read-only tool rejection: {body}"
    );
}

#[tokio::test]
async fn execute_rejects_grouped_non_read_only_ask_tool() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureGroupedSideEffectCapability::new(
            FIXTURE_TOOL,
            FIXTURE_TOOL_GROUP,
        )))
        .expect("register grouped side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::EXECUTE,
        serde_json::json!({
            "air": ask_air(&grouped_tools_attrs(FIXTURE_TOOL_GROUP))
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_NOT_READ_ONLY),
        "expected grouped non-read-only tool rejection: {body}"
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
        admit_capabilities: vec![],
        imports: vec![],
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
async fn compile_lowers_and_executes_caller_supplied_graph() {
    // A caller-supplied PlanGraph posted to /v1/compile lowers server-side and
    // runs through the same runtime as /v1/execute — here a read-only inv_tool.
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureReadCapability::new(FIXTURE_TOOL)))
        .expect("register fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::COMPILE,
        serde_json::json!({
            "graph": inv_tool_plan_graph(FIXTURE_TOOL),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "compile execute failed: {body}");
    assert_eq!(body["content"], FIXTURE_OUTPUT);
}

#[tokio::test]
async fn compile_applies_raw_execute_admission_gate() {
    // The /v1/compile route must share /v1/execute's admission gate: a
    // non-read-only (Direct) inv_tool without an admit grant is rejected even
    // though the graph never went through the LLM emission path.
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(FIXTURE_TOOL)))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::COMPILE,
        serde_json::json!({
            "graph": inv_tool_plan_graph(FIXTURE_TOOL),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(ERROR_WRITE_NOT_GRANTED),
        "compile must enforce the write-grant gate: {body}"
    );
}

#[tokio::test]
async fn compile_admits_granted_direct_capability() {
    let state = test_state().await;
    state
        .runtime
        .capability_system()
        .register(Arc::new(FixtureSideEffectCapability::new(FIXTURE_TOOL)))
        .expect("register side-effectful fixture capability");
    let app = build_app(state);

    let (status, body) = post_json(
        app,
        routes::COMPILE,
        serde_json::json!({
            "graph": inv_tool_plan_graph(FIXTURE_TOOL),
            "admit_capabilities": [FIXTURE_TOOL],
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "granted compile failed: {body}");
    assert_eq!(body["content"], FIXTURE_OUTPUT);
}

#[tokio::test]
async fn compile_rejects_invalid_graph() {
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app,
        routes::COMPILE,
        serde_json::json!({
            "graph": { "name": "empty", "entry": "plan", "nodes": [] },
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("plan graph lowering failed"),
        "expected lowering failure: {body}"
    );
}

fn inv_tool_plan_graph(capability: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "compile_test_plan",
        "entry": "plan",
        "nodes": [
            {
                "id": 1,
                "name": "call_tool",
                "op": "inv_tool",
                "capability": capability,
                "args": {}
            }
        ]
    })
}

struct FixtureGroupedSideEffectCapability {
    metadata: CapabilityMetadata,
}

impl FixtureGroupedSideEffectCapability {
    fn new(name: &str, group: &str) -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                name,
                "Fixture grouped side-effectful capability",
                serde_json::json!({ "type": "object", "properties": {} }),
            )
            .with_returns("string")
            .with_groups(vec![group.to_string()]),
        }
    }
}

#[async_trait]
impl CapabilityExecutor for FixtureGroupedSideEffectCapability {
    async fn execute(&self, _args: HashMap<String, Value>) -> Result<Value, RuntimeError> {
        Ok(Value::String(FIXTURE_OUTPUT.to_string()))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

fn inv_tool_air(capability: &str) -> String {
    format!(
        r#"module {{
  func.func @main() -> !ais.token attributes {{ais.entry}} {{
    %reg = ais.register_capability "{capability}" {{description = "fixture tool"}} : !ais.token
    %tool = ais.inv_tool "{capability}" ("{{}}") : !ais.token
    func.return %tool : !ais.token
  }}
}}
"#
    )
}

fn python_handler_inv_tool_air(capability: &str) -> String {
    format!(
        r#"module {{
  func.func @main() -> !ais.token attributes {{ais.entry}} {{
    %reg = ais.register_capability "{capability}" {{description = "fixture tool", python_handler_id = "sha256:0000000000000000000000000000000000000000000000000000000000000000"}} : !ais.token
    %tool = ais.inv_tool "{capability}" ("{{}}") : !ais.token
    func.return %tool : !ais.token
  }}
}}
"#
    )
}

fn ask_air(attrs: &str) -> String {
    format!(
        r#"module {{
  func.func @main() -> !ais.token attributes {{ais.entry}} {{
    %ask = ais.ask "hello" {attrs} : !ais.token
    func.return %ask : !ais.token
  }}
}}
"#
    )
}

fn tools_enabled_all_attrs() -> &'static str {
    "{tools_enabled = true}"
}

fn explicit_tool_attrs(tool: &str) -> String {
    format!(r#"{{tools = ["{tool}"]}}"#)
}

fn grouped_tools_attrs(group: &str) -> String {
    format!(r#"{{tools_enabled = true, tool_groups = ["{group}"]}}"#)
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
            admit_capabilities: vec![],
            imports: vec![],
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
        admit_capabilities: vec![],
        imports: vec![],
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
        routes::EXECUTE,
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
