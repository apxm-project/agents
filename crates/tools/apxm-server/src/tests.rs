// Integration Tests
//
// These tests exercise the HTTP API layer without binding to a TCP port.
// They use `tower::ServiceExt::oneshot` + `axum::body::Body` to send requests
// directly to the Axum router and collect responses via `http_body_util::BodyExt`.
//
// No LLM API key is required — all LLM-touching tests are gated behind
// `#[cfg(feature = "integration")]` and use stub responses.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use apxm_artifact::{Artifact, ArtifactMetadata};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::events::payload::REDACTION_HASH_PREFIX_BLAKE3;
use apxm_core::types::AISOperationType;
use apxm_core::types::execution::{DagMetadata, ExecutionDag, Node, NodeMetadata};
use apxm_core::types::values::Value;
use apxm_runtime::capability::executor::CapabilityExecutor;
use apxm_runtime::capability::metadata::CapabilityMetadata;
use apxm_runtime::{Runtime, RuntimeConfig};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use dashmap::DashMap;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::build_app;
use crate::checkpoints::{Checkpoint, CheckpointStatus, CheckpointStore};
use crate::execute::{ExecuteRequest, prepare_request};
use crate::executions::{EXECUTION_RECORD_FILE, ExecutionStore};
use crate::helpers::{jsonrpc_err, jsonrpc_ok, mcp_tool_result, now_ms};
use crate::mcp::{
    MCP_TOOL_APXM_SKILL_CALL, MCP_TOOL_APXM_SKILL_GET, MCP_TOOL_APXM_SKILL_VALIDATE,
    MCP_TOOL_APXM_SKILLS_LIST,
};
use crate::skills::SkillLibrary;
use crate::state::AppState;
use crate::tasks::{QueuedTask, TaskQueueManager, TaskStatus};

// ── Test helpers ──────────────────────────────────────────────────────────

const ROUTE_SKILLS: &str = "/v1/skills";
const ROUTE_EXECUTIONS: &str = "/v1/executions";
const ROUTE_MCP: &str = "/v1/mcp";
const MCP_METHOD_INITIALIZE: &str = "initialize";
const MCP_METHOD_TOOLS_LIST: &str = "tools/list";
const MCP_METHOD_TOOLS_CALL: &str = "tools/call";
const MCP_JSONRPC_VERSION: &str = "2.0";
const MCP_REQUEST_ID: u64 = 23;
const MCP_PARAM_NAME: &str = "name";
const MCP_PARAM_ARGUMENTS: &str = "arguments";
const MCP_ARG_ID: &str = "id";
const MCP_ARG_SESSION_ID: &str = "session_id";
const EVENT_SKILL_EXECUTE_STARTED: &str = "skill_execute_started";
const EVENT_SKILL_EXECUTE_COMPLETE: &str = "skill_execute_complete";
const EVENT_NODE_OUTPUT: &str = "node_output";
const EVENT_NODE_METRICS: &str = "node_metrics";
const NODE_OUTPUT_FIELD: &str = "output";
const REDACTED_FIELD: &str = "redacted";
const SUMMARY_FIELD: &str = "summary";
const HASH_FIELD: &str = "hash";

const FIXTURE_PACKAGE_DIR: &str = "checkout";
const FIXTURE_PACKAGE_ALT_DIR: &str = "checkout-alt";
const FIXTURE_PACKAGE_V2_DIR: &str = "checkout-v2";
const FIXTURE_SKILL_ID: &str = "checkout-context-triage";
const FIXTURE_SKILL_VERSION: &str = "0.1.0";
const FIXTURE_SKILL_NEXT_VERSION: &str = "0.2.0";
const UNKNOWN_SKILL_VERSION: &str = "9.9.9";
const FIXTURE_DISPLAY_NAME: &str = "Checkout Context Triage";
const FIXTURE_DESCRIPTION: &str = "Triage checkout duplicate authorization incidents";
const FIXTURE_ENTRY_FLOW: &str = "main";
const FIXTURE_ALT_ENTRY_FLOW: &str = "alternate";
const FIXTURE_CAPABILITY: &str = "llm:showcase";
const FIXTURE_TOOL: &str = "read_file";
const FIXTURE_INPUT_NAME: &str = "checkout_context";
const FIXTURE_OUTPUT_NAME: &str = "triage_report";
const FIXTURE_TIMEOUT_MS: u64 = 30_000;
const FIXTURE_TOKEN_LIMIT: u64 = 4096;
const FIXTURE_SOURCE: &str = "# Checkout Context Triage\n\nFollow the checkout triage steps.\n";
const FIXTURE_AIR: &str = "module { func.func @main() attributes {ais.entry} }";
const FIXTURE_OUTPUT: &str = "ok";
const FIXTURE_OUTPUT_SUMMARY: &str = "string(chars=2)";
const FIXTURE_OUTPUT_V2: &str = "ok-v2";
const FIXTURE_COMPILER_VERSION: &str = "test-compiler";
const FIXTURE_CONVERSION_REPORT: &str = r#"{"status":"hand-authored","unmapped":[]}"#;
const FILE_SKILL_MANIFEST: &str = "skill.toml";
const FILE_SKILL_SOURCE: &str = "SKILL.md";
const FILE_SKILL_AIR: &str = "skill.air";
const FILE_SKILL_ARTIFACT: &str = "skill.apxmobj";
const FILE_CONVERSION_REPORT: &str = "conversion-report.json";
const DIR_RESOURCES: &str = "resources";
const DIR_TESTS: &str = "tests";
const MSG_SKILL_DIR: &str = "skill dir";
const MSG_RESOURCES_DIR: &str = "resources dir";
const MSG_TESTS_DIR: &str = "tests dir";
const CLI_ARG_BIN: &str = "apxm-server";
const CLI_ARG_SKILL_ROOT: &str = "--skill-root";
const UNKNOWN_SKILL_ID: &str = "missing-skill";
const QUERY_ROOT_INJECTION: &str = "root=/tmp/should-not-be-read";
const REQUEST_ROOT_FIELD: &str = "root";
const REQUEST_ROOT_INJECTION: &str = "/tmp/should-not-be-read";
const REQUEST_SESSION_ROOT_FIELD: &str = "session_root";
const REQUEST_AIR_FIELD: &str = "air";
const FIXTURE_SESSION_ID: &str = "skill-session";
const INVALID_SESSION_ID_WITH_SEPARATOR: &str = "skill/session";
const INVALID_SESSION_ID_WITH_BACKSLASH: &str = "skill\\session";
const INVALID_SESSION_ID_WITH_SPACE: &str = "skill session";
const INVALID_SESSION_ID_WITH_SEMICOLON: &str = "skill;session";
const INVALID_SESSION_ID_CURRENT: &str = ".";
const INVALID_SESSION_ID_PARENT: &str = "..";
const BAD_ARTIFACT_HASH: &str =
    "blake3:0000000000000000000000000000000000000000000000000000000000000000";
const ERROR_SKILL_NOT_COMPILED: &str = "skill is not compiled";
const ERROR_ARTIFACT_HASH_REQUIRED: &str = "manifest artifact_hash is required";
const ERROR_ARTIFACT_HASH_MISMATCH: &str = "artifact_hash mismatch";
const ERROR_ENTRY_FLOW_MISMATCH: &str = "does not match manifest entry_flow";
const ERROR_INVALID_SESSION_ID: &str = "session_id must contain";
const ERROR_INVALID_ARTIFACT: &str = "invalid skill artifact";
const ERROR_DISALLOWED_OPERATION: &str = "disallowed operation";
const ERROR_CAPABILITY_NOT_REGISTERED: &str = "is not registered";
const ERROR_UNDECLARED_CAPABILITY: &str = "invokes undeclared capability";
const ERROR_PYTHON_INV_TOOL_UNSUPPORTED: &str = "does not support python-backed INV_TOOL";
const ERROR_SIDE_EFFECT_POLICY_UNSUPPORTED: &str = "does not support side_effect_policy";

struct FixtureReadCapability {
    metadata: CapabilityMetadata,
}

impl FixtureReadCapability {
    fn new(name: &str) -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                name,
                "Fixture read-only capability",
                serde_json::json!({ "type": "object", "properties": {} }),
            )
            .with_returns("string")
            .with_read_only(),
        }
    }
}

#[async_trait]
impl CapabilityExecutor for FixtureReadCapability {
    async fn execute(&self, _args: HashMap<String, Value>) -> Result<Value, RuntimeError> {
        Ok(Value::String(FIXTURE_OUTPUT.to_string()))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

struct SkillFixture {
    source_hash: String,
    air_hash: String,
    artifact_hash: String,
}

fn const_only_air() -> String {
    r#"module {
  func.func @const_only() -> !ais.token attributes {ais.entry} {
    %value = ais.const_str "ok" : !ais.token
    func.return %value : !ais.token
  }
}
"#
    .to_string()
}

/// Build a test AppState backed by a real (but unconfigured) Runtime.
///
/// The runtime has no LLM backends registered, so any graph that calls
/// ASK/THINK/REASON will error.  Tests that need execution should use
/// graphs composed entirely of CONST_STR and synchronisation ops.
async fn test_state() -> AppState {
    test_state_with_skill_roots(Vec::new()).await
}

async fn test_state_with_skill_roots(skill_roots: Vec<std::path::PathBuf>) -> AppState {
    // Use in-memory LTM to avoid SQLite file-locking across parallel tests.
    let runtime = Arc::new(
        Runtime::new(RuntimeConfig::in_memory())
            .await
            .expect("test runtime"),
    );
    AppState {
        runtime,
        agent_registry: Arc::new(DashMap::new()),
        task_manager: TaskQueueManager::new(),
        checkpoint_store: CheckpointStore::new(),
        start_time: SystemTime::now(),
        a2a_tasks: Arc::new(DashMap::new()),
        skill_library: SkillLibrary::new(skill_roots),
        execution_store: ExecutionStore::new(),
    }
}

/// POST a JSON body to `path` and return `(StatusCode, serde_json::Value)`.
async fn post_json(
    app: Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn post_json_text(app: Router, path: &str, body: serde_json::Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).expect("utf8 response");
    (status, text)
}

/// GET `path` and return `(StatusCode, serde_json::Value)`.
async fn get_json(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn write_skill_manifest(dir: &std::path::Path, contents: &str) {
    std::fs::create_dir_all(dir).expect(MSG_SKILL_DIR);
    std::fs::write(dir.join(FILE_SKILL_MANIFEST), contents).expect(FILE_SKILL_MANIFEST);
}

fn write_valid_skill(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let skill_dir = root.join(name);
    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{FIXTURE_SKILL_ID}"
version = "{FIXTURE_SKILL_VERSION}"
display_name = "{FIXTURE_DISPLAY_NAME}"
entry_flow = "{FIXTURE_ENTRY_FLOW}"
required_capabilities = ["{FIXTURE_CAPABILITY}"]
"#
        ),
    );
    std::fs::write(skill_dir.join(FILE_SKILL_AIR), FIXTURE_AIR).expect(FILE_SKILL_AIR);
    skill_dir
}

fn write_complete_skill(root: &std::path::Path) -> SkillFixture {
    write_complete_skill_with_version(root, FIXTURE_PACKAGE_DIR, FIXTURE_SKILL_VERSION)
}

fn write_complete_skill_with_version(
    root: &std::path::Path,
    package_name: &str,
    version: &str,
) -> SkillFixture {
    write_skill_with_artifact(
        root,
        package_name,
        version,
        &skill_artifact_bytes(AISOperationType::ConstStr),
        Some(FIXTURE_SKILL_ID),
        Some(FIXTURE_ENTRY_FLOW),
    )
}

fn write_executable_skill(root: &std::path::Path) {
    write_executable_skill_with_output(
        root,
        FIXTURE_PACKAGE_DIR,
        FIXTURE_SKILL_VERSION,
        FIXTURE_OUTPUT,
    );
}

fn write_executable_skill_with_output(
    root: &std::path::Path,
    package_name: &str,
    version: &str,
    output: &str,
) {
    let artifact = skill_artifact_bytes_with_output(AISOperationType::ConstStr, output);
    write_executable_skill_with_artifact(
        root,
        package_name,
        version,
        &artifact,
        FIXTURE_SKILL_ID,
        FIXTURE_ENTRY_FLOW,
    );
}

fn write_executable_skill_with_artifact(
    root: &std::path::Path,
    package_name: &str,
    version: &str,
    artifact_bytes: &[u8],
    skill_id: &str,
    entry_flow: &str,
) {
    let skill_dir = root.join(package_name);
    std::fs::create_dir_all(&skill_dir).expect(MSG_SKILL_DIR);
    std::fs::write(skill_dir.join(FILE_SKILL_ARTIFACT), artifact_bytes).expect(FILE_SKILL_ARTIFACT);
    let artifact_hash = tagged_blake3(artifact_bytes);
    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{skill_id}"
version = "{version}"
entry_flow = "{entry_flow}"
artifact_hash = "{artifact_hash}"
timeout_ms = {FIXTURE_TIMEOUT_MS}
"#
        ),
    );
}

fn write_policy_skill_with_artifact(
    root: &std::path::Path,
    artifact_bytes: &[u8],
    required_capability: &str,
    allowed_tool: &str,
) {
    let skill_dir = root.join(FIXTURE_PACKAGE_DIR);
    std::fs::create_dir_all(&skill_dir).expect(MSG_SKILL_DIR);
    std::fs::write(skill_dir.join(FILE_SKILL_ARTIFACT), artifact_bytes).expect(FILE_SKILL_ARTIFACT);
    let artifact_hash = tagged_blake3(artifact_bytes);
    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{FIXTURE_SKILL_ID}"
version = "{FIXTURE_SKILL_VERSION}"
entry_flow = "{FIXTURE_ENTRY_FLOW}"
artifact_hash = "{artifact_hash}"
required_capabilities = ["{required_capability}"]
allowed_tools = ["{allowed_tool}"]
side_effect_policy = "read_only"
"#
        ),
    );
}

fn write_side_effect_policy_skill_with_artifact(
    root: &std::path::Path,
    artifact_bytes: &[u8],
    side_effect_policy: &str,
) {
    let skill_dir = root.join(FIXTURE_PACKAGE_DIR);
    std::fs::create_dir_all(&skill_dir).expect(MSG_SKILL_DIR);
    std::fs::write(skill_dir.join(FILE_SKILL_ARTIFACT), artifact_bytes).expect(FILE_SKILL_ARTIFACT);
    let artifact_hash = tagged_blake3(artifact_bytes);
    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{FIXTURE_SKILL_ID}"
version = "{FIXTURE_SKILL_VERSION}"
entry_flow = "{FIXTURE_ENTRY_FLOW}"
artifact_hash = "{artifact_hash}"
side_effect_policy = "{side_effect_policy}"
"#
        ),
    );
}

fn write_skill_with_artifact(
    root: &std::path::Path,
    package_name: &str,
    version: &str,
    artifact_bytes: &[u8],
    skill_id: Option<&str>,
    entry_flow: Option<&str>,
) -> SkillFixture {
    let skill_dir = root.join(package_name);
    std::fs::create_dir_all(skill_dir.join(DIR_RESOURCES)).expect(MSG_RESOURCES_DIR);
    std::fs::create_dir_all(skill_dir.join(DIR_TESTS)).expect(MSG_TESTS_DIR);
    std::fs::write(skill_dir.join(FILE_SKILL_SOURCE), FIXTURE_SOURCE).expect(FILE_SKILL_SOURCE);
    std::fs::write(skill_dir.join(FILE_SKILL_AIR), FIXTURE_AIR).expect(FILE_SKILL_AIR);
    std::fs::write(skill_dir.join(FILE_SKILL_ARTIFACT), artifact_bytes).expect(FILE_SKILL_ARTIFACT);
    std::fs::write(
        skill_dir.join(FILE_CONVERSION_REPORT),
        FIXTURE_CONVERSION_REPORT,
    )
    .expect(FILE_CONVERSION_REPORT);

    let source_hash = tagged_blake3(FIXTURE_SOURCE.as_bytes());
    let air_hash = tagged_blake3(FIXTURE_AIR.as_bytes());
    let artifact_hash = tagged_blake3(artifact_bytes);
    let skill_id = skill_id.unwrap_or(FIXTURE_SKILL_ID);
    let entry_flow = entry_flow.unwrap_or(FIXTURE_ENTRY_FLOW);

    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{skill_id}"
version = "{version}"
display_name = "{FIXTURE_DISPLAY_NAME}"
description = "{FIXTURE_DESCRIPTION}"
entry_flow = "{entry_flow}"
source_hash = "{source_hash}"
air_hash = "{air_hash}"
artifact_hash = "{artifact_hash}"
required_capabilities = ["{FIXTURE_CAPABILITY}"]
allowed_tools = ["{FIXTURE_TOOL}"]
timeout_ms = {FIXTURE_TIMEOUT_MS}
token_limit = {FIXTURE_TOKEN_LIMIT}
isolation_policy = "process"
side_effect_policy = "read_only"

[[inputs]]
name = "{FIXTURE_INPUT_NAME}"
type = "string"
required = true

[[outputs]]
name = "{FIXTURE_OUTPUT_NAME}"
type = "string"
"#
        ),
    );

    SkillFixture {
        source_hash,
        air_hash,
        artifact_hash,
    }
}

fn skill_artifact_bytes(op_type: AISOperationType) -> Vec<u8> {
    skill_artifact_bytes_with_entry_and_output(op_type, FIXTURE_ENTRY_FLOW, FIXTURE_OUTPUT)
}

fn corrupted_skill_artifact_bytes() -> Vec<u8> {
    let mut bytes = skill_artifact_bytes(AISOperationType::ConstStr);
    bytes[0] ^= 0xff;
    bytes
}

fn skill_artifact_bytes_with_output(op_type: AISOperationType, output: &str) -> Vec<u8> {
    skill_artifact_bytes_with_entry_and_output(op_type, FIXTURE_ENTRY_FLOW, output)
}

fn skill_artifact_bytes_with_entry(op_type: AISOperationType, entry_flow: &str) -> Vec<u8> {
    skill_artifact_bytes_with_entry_and_output(op_type, entry_flow, FIXTURE_OUTPUT)
}

fn skill_artifact_bytes_with_entry_and_output(
    op_type: AISOperationType,
    entry_flow: &str,
    output: &str,
) -> Vec<u8> {
    let mut node = Node {
        id: 1,
        op_type,
        attributes: std::collections::HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![100],
        metadata: NodeMetadata::default(),
    };
    node.attributes.insert(
        graph_attrs::VALUE.to_string(),
        Value::String(output.to_string()),
    );
    let dag = ExecutionDag {
        nodes: vec![node],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![1],
        metadata: DagMetadata {
            name: Some(entry_flow.to_string()),
            is_entry: true,
            parameters: vec![],
        },
    };
    let artifact = Artifact::new(
        ArtifactMetadata::new(Some(FIXTURE_SKILL_ID.to_string()), FIXTURE_COMPILER_VERSION),
        vec![dag],
    );
    artifact.to_bytes().expect("artifact bytes")
}

fn inv_tool_artifact_bytes(capability: &str, python_handler_id: Option<&str>) -> Vec<u8> {
    let mut node = Node {
        id: 1,
        op_type: AISOperationType::InvTool,
        attributes: HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![100],
        metadata: NodeMetadata::default(),
    };
    node.attributes.insert(
        graph_attrs::CAPABILITY.to_string(),
        Value::String(capability.to_string()),
    );
    node.attributes.insert(
        graph_attrs::PARAMS_JSON.to_string(),
        Value::String("{}".to_string()),
    );
    if let Some(handler_id) = python_handler_id {
        node.attributes.insert(
            graph_attrs::PYTHON_HANDLER_ID.to_string(),
            Value::String(handler_id.to_string()),
        );
    }
    let dag = ExecutionDag {
        nodes: vec![node],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![1],
        metadata: DagMetadata {
            name: Some(FIXTURE_ENTRY_FLOW.to_string()),
            is_entry: true,
            parameters: vec![],
        },
    };
    let artifact = Artifact::new(
        ArtifactMetadata::new(Some(FIXTURE_SKILL_ID.to_string()), FIXTURE_COMPILER_VERSION),
        vec![dag],
    );
    artifact.to_bytes().expect("artifact bytes")
}

fn tagged_blake3(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

fn skill_detail_route(id: &str) -> String {
    format!("{ROUTE_SKILLS}/{id}")
}

fn skill_validate_route(id: &str) -> String {
    format!("{ROUTE_SKILLS}/{id}/validate")
}

fn skill_execute_route(id: &str) -> String {
    format!("{ROUTE_SKILLS}/{id}/execute")
}

fn skill_execute_stream_route(id: &str) -> String {
    format!("{ROUTE_SKILLS}/{id}/execute/stream")
}

fn execution_detail_route(id: &str) -> String {
    format!("{ROUTE_EXECUTIONS}/{id}")
}

fn execution_node_detail_route(execution_id: &str, node_id: u64) -> String {
    format!("{ROUTE_EXECUTIONS}/{execution_id}/nodes/{node_id}")
}

fn sse_data_events(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("SSE JSON event"))
        .collect()
}

fn versioned_skill_id() -> String {
    format!("{FIXTURE_SKILL_ID}@{FIXTURE_SKILL_VERSION}")
}

fn tool_text(body: &serde_json::Value) -> &str {
    body["result"]["content"][0]["text"]
        .as_str()
        .expect("MCP text content")
}

fn mcp_call(tool_name: &str, arguments: serde_json::Value) -> serde_json::Value {
    let mut params = serde_json::Map::new();
    params.insert(
        MCP_PARAM_NAME.to_string(),
        serde_json::Value::String(tool_name.to_string()),
    );
    params.insert(MCP_PARAM_ARGUMENTS.to_string(), arguments);
    serde_json::json!({
        "jsonrpc": MCP_JSONRPC_VERSION,
        "id": MCP_REQUEST_ID,
        "method": MCP_METHOD_TOOLS_CALL,
        "params": params
    })
}

fn assert_complete_skill_record(record: &serde_json::Value, fixture: &SkillFixture) {
    assert_eq!(record["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(record["version"], FIXTURE_SKILL_VERSION);
    assert_eq!(record["package"]["name"], FIXTURE_PACKAGE_DIR);
    assert_eq!(record["manifest"]["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(record["manifest"]["version"], FIXTURE_SKILL_VERSION);
    assert_eq!(record["manifest"]["display_name"], FIXTURE_DISPLAY_NAME);
    assert_eq!(record["manifest"]["description"], FIXTURE_DESCRIPTION);
    assert_eq!(record["manifest"]["entry_flow"], FIXTURE_ENTRY_FLOW);
    assert_eq!(
        record["manifest"]["required_capabilities"][0],
        FIXTURE_CAPABILITY
    );
    assert_eq!(record["manifest"]["allowed_tools"][0], FIXTURE_TOOL);
    assert_eq!(record["manifest"]["timeout_ms"], FIXTURE_TIMEOUT_MS);
    assert_eq!(record["manifest"]["token_limit"], FIXTURE_TOKEN_LIMIT);
    assert_eq!(record["manifest"]["inputs"][0]["name"], FIXTURE_INPUT_NAME);
    assert_eq!(
        record["manifest"]["outputs"][0]["name"],
        FIXTURE_OUTPUT_NAME
    );

    assert_eq!(record["files"]["has_skill_md"], true);
    assert_eq!(record["files"]["has_air"], true);
    assert_eq!(record["files"]["has_artifact"], true);
    assert_eq!(record["files"]["has_conversion_report"], true);
    assert_eq!(record["files"]["has_resources"], true);
    assert_eq!(record["files"]["has_tests"], true);

    assert_eq!(record["hashes"]["source_hash"], fixture.source_hash);
    assert_eq!(record["hashes"]["air_hash"], fixture.air_hash);
    assert_eq!(record["hashes"]["artifact_hash"], fixture.artifact_hash);
    assert_eq!(record["compile_status"], "compiled");
    assert_eq!(record["validation"]["status"], "valid");
    assert_eq!(record["validation"]["errors"].as_array().unwrap().len(), 0);
}

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

// ── /v1/skills (server-owned skill library) ──────────────────────────────

#[tokio::test]
async fn skills_list_returns_installed_manifests() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_valid_skill(temp.path(), FIXTURE_PACKAGE_DIR);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = get_json(app, ROUTE_SKILLS).await;

    assert_eq!(status, StatusCode::OK, "list skills failed: {body}");
    assert_eq!(body["object"], "list");
    assert_eq!(body["data"][0]["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(body["data"][0]["version"], FIXTURE_SKILL_VERSION);
    assert_eq!(
        body["data"][0]["manifest"]["entry_flow"],
        FIXTURE_ENTRY_FLOW
    );
    assert_eq!(body["data"][0]["compile_status"], "not_compiled");
    assert_eq!(body["data"][0]["validation"]["status"], "valid");
}

#[tokio::test]
async fn skill_detail_returns_manifest_and_status() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_valid_skill(temp.path(), FIXTURE_PACKAGE_DIR);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = get_json(app, &skill_detail_route(FIXTURE_SKILL_ID)).await;

    assert_eq!(status, StatusCode::OK, "skill detail failed: {body}");
    assert_eq!(body["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(body["manifest"]["display_name"], FIXTURE_DISPLAY_NAME);
    assert_eq!(body["validation"]["status"], "valid");
}

#[tokio::test]
async fn skill_validate_reports_hash_mismatch() {
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
        &skill_validate_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "validate failed: {body}");
    assert_eq!(body["validation"]["status"], "invalid");
    assert!(
        body["validation"]["errors"][0]
            .as_str()
            .unwrap_or_default()
            .contains("artifact_hash mismatch"),
        "expected hash mismatch: {body}"
    );
}

#[tokio::test]
async fn skill_detail_unknown_returns_404() {
    let app = build_app(test_state().await);
    let (status, body) = get_json(app, &skill_detail_route(UNKNOWN_SKILL_ID)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

#[tokio::test]
async fn skills_registry_reports_duplicate_id_version_as_ambiguous() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill_with_version(temp.path(), FIXTURE_PACKAGE_DIR, FIXTURE_SKILL_VERSION);
    write_complete_skill_with_version(temp.path(), FIXTURE_PACKAGE_ALT_DIR, FIXTURE_SKILL_VERSION);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (list_status, list_body) = get_json(app.clone(), ROUTE_SKILLS).await;
    assert_eq!(list_status, StatusCode::OK, "list failed: {list_body}");
    let records = list_body["data"].as_array().expect("skill records");
    assert_eq!(records.len(), 2);
    for record in records {
        assert_eq!(record["validation"]["status"], "invalid");
        assert!(
            record["validation"]["errors"]
                .as_array()
                .expect("errors")
                .iter()
                .any(|error| error.as_str().unwrap_or_default().contains("duplicate")),
            "duplicate error missing: {record}"
        );
    }

    let (detail_status, detail_body) = get_json(app, &skill_detail_route(FIXTURE_SKILL_ID)).await;
    assert_eq!(
        detail_status,
        StatusCode::BAD_REQUEST,
        "ambiguous detail should fail: {detail_body}"
    );
}

#[tokio::test]
async fn skills_registry_selects_versioned_skill_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill_with_version(temp.path(), FIXTURE_PACKAGE_DIR, FIXTURE_SKILL_VERSION);
    write_complete_skill_with_version(
        temp.path(),
        FIXTURE_PACKAGE_V2_DIR,
        FIXTURE_SKILL_NEXT_VERSION,
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let requested_id = format!("{FIXTURE_SKILL_ID}@{FIXTURE_SKILL_NEXT_VERSION}");

    let (status, body) = get_json(app, &skill_detail_route(&requested_id)).await;

    assert_eq!(status, StatusCode::OK, "versioned lookup failed: {body}");
    assert_eq!(body["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(body["version"], FIXTURE_SKILL_NEXT_VERSION);
    assert_eq!(body["package"]["name"], FIXTURE_PACKAGE_V2_DIR);
}

#[tokio::test]
async fn skills_registry_reports_missing_required_manifest_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let skill_dir = temp.path().join(FIXTURE_PACKAGE_DIR);
    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{FIXTURE_SKILL_ID}"
version = "{FIXTURE_SKILL_VERSION}"
"#
        ),
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = get_json(app, ROUTE_SKILLS).await;

    assert_eq!(status, StatusCode::OK, "list failed: {body}");
    assert_eq!(body["data"][0]["validation"]["status"], "invalid");
    assert!(
        body["data"][0]["validation"]["errors"]
            .as_array()
            .expect("errors")
            .iter()
            .any(|error| error.as_str().unwrap_or_default().contains("entry_flow")),
        "missing entry_flow error absent: {body}"
    );
}

#[test]
fn parse_cli_skill_roots_uses_repeated_roots_and_dedupes() {
    let first_root = std::path::PathBuf::from("/tmp/apxm-skill-root-a");
    let second_root = std::path::PathBuf::from("/tmp/apxm-skill-root-b");
    let args = vec![
        CLI_ARG_BIN.to_string(),
        CLI_ARG_SKILL_ROOT.to_string(),
        first_root.to_string_lossy().to_string(),
        CLI_ARG_SKILL_ROOT.to_string(),
        second_root.to_string_lossy().to_string(),
        CLI_ARG_SKILL_ROOT.to_string(),
        first_root.to_string_lossy().to_string(),
    ];

    let roots = crate::skills::parse_cli_skill_roots(&args);

    assert_eq!(roots, vec![first_root, second_root]);
}

#[tokio::test]
async fn skills_registry_rest_and_mcp_share_read_only_validated_inventory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = write_complete_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let versioned_id = versioned_skill_id();

    // REST list must ignore request-provided roots and use only AppState roots.
    let injected_list_route = format!("{ROUTE_SKILLS}?{QUERY_ROOT_INJECTION}");
    let (list_status, list_body) = get_json(app.clone(), &injected_list_route).await;
    assert_eq!(list_status, StatusCode::OK, "list failed: {list_body}");
    assert!(
        list_body.get("roots").is_none(),
        "normal API responses must not expose filesystem roots: {list_body}"
    );
    assert_eq!(list_body["data"].as_array().unwrap().len(), 1);
    assert_complete_skill_record(&list_body["data"][0], &fixture);

    let (detail_status, detail_body) =
        get_json(app.clone(), &skill_detail_route(&versioned_id)).await;
    assert_eq!(
        detail_status,
        StatusCode::OK,
        "detail failed: {detail_body}"
    );
    assert_complete_skill_record(&detail_body, &fixture);

    // Validate re-reads the installed package and ignores client body paths.
    let mut validate_request = serde_json::Map::new();
    validate_request.insert(
        REQUEST_ROOT_FIELD.to_string(),
        serde_json::Value::String(REQUEST_ROOT_INJECTION.to_string()),
    );
    let (validate_status, validate_body) = post_json(
        app.clone(),
        &skill_validate_route(&versioned_id),
        serde_json::Value::Object(validate_request),
    )
    .await;
    assert_eq!(
        validate_status,
        StatusCode::OK,
        "validate failed: {validate_body}"
    );
    assert_complete_skill_record(&validate_body, &fixture);

    let (mcp_list_status, mcp_list_body) = post_json(
        app.clone(),
        ROUTE_MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 1,
            "method": MCP_METHOD_TOOLS_LIST,
            "params": {}
        }),
    )
    .await;
    assert_eq!(
        mcp_list_status,
        StatusCode::OK,
        "MCP tools/list failed: {mcp_list_body}"
    );
    let tool_names: Vec<&str> = mcp_list_body["result"]["tools"]
        .as_array()
        .expect("MCP tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tool_names.contains(&MCP_TOOL_APXM_SKILLS_LIST));
    assert!(tool_names.contains(&MCP_TOOL_APXM_SKILL_GET));
    assert!(tool_names.contains(&MCP_TOOL_APXM_SKILL_VALIDATE));
    assert!(tool_names.contains(&MCP_TOOL_APXM_SKILL_CALL));

    let mut mcp_args = serde_json::Map::new();
    mcp_args.insert(
        MCP_ARG_ID.to_string(),
        serde_json::Value::String(versioned_id),
    );
    let (mcp_validate_status, mcp_validate_body) = post_json(
        app,
        ROUTE_MCP,
        mcp_call(
            MCP_TOOL_APXM_SKILL_VALIDATE,
            serde_json::Value::Object(mcp_args),
        ),
    )
    .await;
    assert_eq!(
        mcp_validate_status,
        StatusCode::OK,
        "MCP validate failed: {mcp_validate_body}"
    );
    assert_eq!(mcp_validate_body["result"]["isError"], false);
    let mcp_record: serde_json::Value =
        serde_json::from_str(tool_text(&mcp_validate_body)).expect("MCP skill JSON");
    assert_complete_skill_record(&mcp_record, &fixture);
}

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

    let snapshot_path = std::path::Path::new(session_dir).join(EXECUTION_RECORD_FILE);
    let snapshot = std::fs::read_to_string(&snapshot_path).expect("execution record snapshot");
    let snapshot_body: serde_json::Value =
        serde_json::from_str(&snapshot).expect("execution record snapshot json");
    assert_eq!(snapshot_body["execution_id"], execution_id);
    assert_eq!(snapshot_body["status"], "succeeded");
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

// ── Agent Registry ────────────────────────────────────────────────────────

#[tokio::test]
async fn agent_registry_register_returns_ok() {
    let app = build_app(test_state().await);

    let (status, body) = post_json(
        app,
        "/v1/agents/register",
        serde_json::json!({
            "name": "test-agent",
            "url": "http://localhost:19999",
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
    let (status, body) = get_json(app, "/v1/agents").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array(), "expected array: {body}");
}

#[tokio::test]
async fn agent_registry_missing_name_returns_400() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/agents/register",
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
    let (status, body) = get_json(app, "/.well-known/agent.json").await;
    assert_eq!(status, StatusCode::OK, "agent card failed: {body}");
    assert!(body["name"].is_string(), "missing 'name': {body}");
    assert!(body["url"].is_string(), "missing 'url': {body}");
    assert!(body["version"].is_string(), "missing 'version': {body}");
    assert!(
        body["capabilities"].is_object(),
        "missing 'capabilities': {body}"
    );
}

// ── /v1/mcp (MCP JSON-RPC) ────────────────────────────────────────────────

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

// ── /v1/execute (AIR execution) ───────────────────────────────────────────

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

#[test]
fn prepare_request_uses_explicit_session_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_root = temp.path().join("sessions");
    let request = ExecuteRequest {
        air: const_only_air(),
        args: vec![],
        session_id: Some("explicit-session".to_string()),
        session_root: Some(session_root.to_string_lossy().to_string()),
    };

    let (_air, _args, session_id, session_dir) = prepare_request(request).expect("prepare");
    let session_dir = session_dir.expect("session dir");

    assert_eq!(session_id.as_deref(), Some("explicit-session"));
    assert_eq!(
        std::path::Path::new(&session_dir),
        session_root.join("explicit-session")
    );
    assert!(std::path::Path::new(&session_dir).is_dir());
}

#[test]
fn prepare_request_generates_session_id_for_root_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_root = temp.path().join("sessions");
    let request = ExecuteRequest {
        air: const_only_air(),
        args: vec![],
        session_id: None,
        session_root: Some(session_root.to_string_lossy().to_string()),
    };

    let (_air, _args, session_id, session_dir) = prepare_request(request).expect("prepare");
    let session_id = session_id.expect("generated session id");
    let session_dir = session_dir.expect("session dir");

    assert!(!session_id.is_empty());
    assert_eq!(
        std::path::Path::new(&session_dir),
        session_root.join(&session_id)
    );
    assert!(std::path::Path::new(&session_dir).is_dir());
}

#[tokio::test]
async fn execute_returns_session_dir_when_requested() {
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

    assert_eq!(status, StatusCode::OK, "expected execution success: {body}");
    assert_eq!(
        body["session_dir"].as_str(),
        Some(
            session_root
                .join("server-session")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(session_root.join("server-session").is_dir());
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

// ── /v1/tasks (task queue) ────────────────────────────────────────────────

#[tokio::test]
async fn task_queue_create_returns_id() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/tasks",
        serde_json::json!({
            "queue": "test-queue",
            "data": { "work": "process this" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create task failed: {body}");
    assert_eq!(body["ok"], true);
    assert!(body["id"].is_string(), "expected task id: {body}");
    assert_eq!(body["queue"], "test-queue");
}

#[tokio::test]
async fn task_queue_list_returns_tasks() {
    let state = test_state().await;
    let app = build_app(state);

    // Create a task first
    post_json(
        app.clone(),
        "/v1/tasks",
        serde_json::json!({
            "queue": "list-test",
            "data": { "item": 1 }
        }),
    )
    .await;

    let (status, body) = get_json(app, "/v1/tasks/list-test").await;
    assert_eq!(status, StatusCode::OK, "list tasks failed: {body}");
    assert_eq!(body["queue"], "list-test");
    assert!(
        body["count"].as_u64().unwrap_or(0) >= 1,
        "expected count >= 1: {body}"
    );
    assert!(body["tasks"].is_array(), "expected tasks array: {body}");
}

#[tokio::test]
async fn task_queue_claim_and_complete() {
    let state = test_state().await;
    let app = build_app(state);

    // 1. Create task
    let (_, create_body) = post_json(
        app.clone(),
        "/v1/tasks",
        serde_json::json!({ "queue": "work", "data": { "job": "test" } }),
    )
    .await;
    let task_id = create_body["id"].as_str().unwrap().to_string();

    // 2. Claim task
    let (claim_status, claim_body) = post_json(
        app.clone(),
        "/v1/tasks/work/claim",
        serde_json::json!({ "agent_id": "test-agent", "lease_ms": 30000 }),
    )
    .await;
    assert_eq!(claim_status, StatusCode::OK, "claim failed: {claim_body}");
    assert_eq!(claim_body["task_id"], task_id);
    let claim_token = claim_body["claim_token"].as_str().unwrap().to_string();

    // 3. Complete task
    let complete_path = format!("/v1/tasks/{}/complete", task_id);
    let (complete_status, complete_body) = post_json(
        app,
        &complete_path,
        serde_json::json!({
            "claim_token": claim_token,
            "result": { "output": "done" },
            "success": true
        }),
    )
    .await;
    assert_eq!(
        complete_status,
        StatusCode::OK,
        "complete failed: {complete_body}"
    );
    assert_eq!(complete_body["ok"], true);
}

#[tokio::test]
async fn task_queue_claim_empty_queue_returns_404() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/tasks/empty-queue/claim",
        serde_json::json!({ "agent_id": "agent-1" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

// ── /v1/checkpoints (PAUSE/RESUME HITL) ───────────────────────────────────

#[tokio::test]
async fn checkpoint_create_returns_pending() {
    let app = build_app(test_state().await);
    let (status, body) = post_json(
        app,
        "/v1/checkpoints",
        serde_json::json!({
            "checkpoint_id": "cp-test-001",
            "message": "Please review the generated plan",
            "display_data": { "plan": "step 1, step 2, step 3" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create checkpoint failed: {body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["checkpoint_id"], "cp-test-001");
    assert_eq!(body["status"], "pending");
    assert!(
        body["resume_url"].is_string(),
        "expected resume_url: {body}"
    );
}

#[tokio::test]
async fn checkpoint_get_returns_checkpoint() {
    let state = test_state().await;
    let app = build_app(state);

    // Create first
    post_json(
        app.clone(),
        "/v1/checkpoints",
        serde_json::json!({
            "checkpoint_id": "cp-get-001",
            "message": "Review needed"
        }),
    )
    .await;

    // Get it
    let (status, body) = get_json(app, "/v1/checkpoints/cp-get-001").await;
    assert_eq!(status, StatusCode::OK, "get checkpoint failed: {body}");
    assert_eq!(body["id"], "cp-get-001");
    assert_eq!(body["status"], "pending");
    assert!(body["message"].is_string());
}

#[tokio::test]
async fn checkpoint_resume_workflow() {
    let state = test_state().await;
    let app = build_app(state);

    // 1. Create checkpoint
    post_json(
        app.clone(),
        "/v1/checkpoints",
        serde_json::json!({
            "checkpoint_id": "cp-resume-001",
            "message": "Human decision required"
        }),
    )
    .await;

    // 2. Resume with human input
    let (status, body) = post_json(
        app,
        "/v1/checkpoints/cp-resume-001/resume",
        serde_json::json!({ "human_input": { "decision": "approved", "notes": "LGTM" } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resume failed: {body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["checkpoint_id"], "cp-resume-001");
    assert_eq!(body["status"], "resumed");
    assert_eq!(body["human_input"]["decision"], "approved");
    assert!(body["resumed_at_ms"].is_number());
}

#[tokio::test]
async fn checkpoint_get_missing_returns_404() {
    let app = build_app(test_state().await);
    let (status, body) = get_json(app, "/v1/checkpoints/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

#[tokio::test]
async fn checkpoint_resume_already_resumed_returns_400() {
    let state = test_state().await;
    let app = build_app(state);

    // Create + resume once
    post_json(
        app.clone(),
        "/v1/checkpoints",
        serde_json::json!({
            "checkpoint_id": "cp-double-001",
            "message": "Once only"
        }),
    )
    .await;
    post_json(
        app.clone(),
        "/v1/checkpoints/cp-double-001/resume",
        serde_json::json!({ "human_input": { "ok": true } }),
    )
    .await;

    // Attempt to resume again
    let (status, body) = post_json(
        app,
        "/v1/checkpoints/cp-double-001/resume",
        serde_json::json!({ "human_input": { "ok": false } }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "expected 400 on double-resume: {body}"
    );
}

// ── TaskQueueManager unit tests ─────────────────────────────────────────

fn make_task(id: &str, queue: &str) -> QueuedTask {
    QueuedTask {
        id: id.to_string(),
        queue: queue.to_string(),
        data: serde_json::json!({"work": id}),
        status: TaskStatus::Pending,
        claimed_by: None,
        claim_token: None,
        lease_expires_ms: None,
        result: None,
        created_at_ms: now_ms(),
        completed_at_ms: None,
    }
}

#[tokio::test]
async fn task_manager_enqueue_and_get() {
    let mgr = TaskQueueManager::new();
    let task = make_task("t1", "q1");
    mgr.enqueue(task).await;

    // Verify task exists in all_tasks index
    assert!(mgr.all_tasks.get("t1").is_some());
    let stored = mgr.all_tasks.get("t1").unwrap();
    assert_eq!(stored.queue, "q1");
    assert_eq!(stored.status, TaskStatus::Pending);
}

#[tokio::test]
async fn task_manager_list_queue() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("a", "q")).await;
    mgr.enqueue(make_task("b", "q")).await;
    mgr.enqueue(make_task("c", "other")).await;

    let q_tasks = mgr.list_queue("q");
    assert_eq!(q_tasks.len(), 2);
    assert!(q_tasks.iter().any(|t| t.id == "a"));
    assert!(q_tasks.iter().any(|t| t.id == "b"));

    let other_tasks = mgr.list_queue("other");
    assert_eq!(other_tasks.len(), 1);
    assert_eq!(other_tasks[0].id, "c");

    // Non-existent queue returns empty
    assert!(mgr.list_queue("nope").is_empty());
}

#[tokio::test]
async fn task_manager_claim_returns_first_pending() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("t1", "q")).await;
    mgr.enqueue(make_task("t2", "q")).await;

    let claimed = mgr.claim("q", "agent-a", 60_000).await;
    assert!(claimed.is_some());
    let claimed = claimed.unwrap();
    assert_eq!(claimed.id, "t1");
    assert_eq!(claimed.status, TaskStatus::Claimed);
    assert_eq!(claimed.claimed_by.as_deref(), Some("agent-a"));
    assert!(claimed.claim_token.is_some());
    assert!(claimed.lease_expires_ms.is_some());
}

#[tokio::test]
async fn task_manager_claim_empty_queue_returns_none() {
    let mgr = TaskQueueManager::new();
    assert!(mgr.claim("nonexistent", "a", 1000).await.is_none());
}

#[tokio::test]
async fn task_manager_claim_skips_already_claimed() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("t1", "q")).await;
    mgr.enqueue(make_task("t2", "q")).await;

    // Claim first task
    let first = mgr.claim("q", "agent-a", 60_000).await.unwrap();
    assert_eq!(first.id, "t1");

    // Next claim should get the second task
    let second = mgr.claim("q", "agent-b", 60_000).await.unwrap();
    assert_eq!(second.id, "t2");

    // No more pending tasks
    assert!(mgr.claim("q", "agent-c", 60_000).await.is_none());
}

#[tokio::test]
async fn task_manager_complete_success() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("t1", "q")).await;

    let claimed = mgr.claim("q", "agent", 60_000).await.unwrap();
    let token = claimed.claim_token.unwrap();

    let result = mgr
        .complete("t1", &token, serde_json::json!({"output": "done"}))
        .await;
    assert!(result.is_ok());

    // Verify completed state in all_tasks
    let stored = mgr.all_tasks.get("t1").unwrap();
    assert_eq!(stored.status, TaskStatus::Completed);
    assert!(stored.result.is_some());
    assert!(stored.completed_at_ms.is_some());
}

#[tokio::test]
async fn task_manager_complete_wrong_token_fails() {
    let mgr = TaskQueueManager::new();
    mgr.enqueue(make_task("t1", "q")).await;
    mgr.claim("q", "agent", 60_000).await;

    let result = mgr
        .complete("t1", "wrong-token", serde_json::json!({}))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid claim token"));
}

#[tokio::test]
async fn task_manager_complete_missing_task_fails() {
    let mgr = TaskQueueManager::new();
    let result = mgr
        .complete("nonexistent", "tok", serde_json::json!({}))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

// ── CheckpointStore unit tests ──────────────────────────────────────────

#[test]
fn checkpoint_store_create_and_get() {
    let store = CheckpointStore::new();
    let cp = Checkpoint {
        id: "cp1".to_string(),
        message: "Review this".to_string(),
        display_data: serde_json::json!({"plan": "step 1"}),
        status: CheckpointStatus::Pending,
        human_input: None,
        notification_url: None,
        created_at_ms: now_ms(),
        resumed_at_ms: None,
    };
    store.create(cp);

    let loaded = store.get("cp1");
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.id, "cp1");
    assert_eq!(loaded.message, "Review this");
    assert_eq!(loaded.status, CheckpointStatus::Pending);
}

#[test]
fn checkpoint_store_get_missing_returns_none() {
    let store = CheckpointStore::new();
    assert!(store.get("nope").is_none());
}

#[test]
fn checkpoint_store_resume_success() {
    let store = CheckpointStore::new();
    store.create(Checkpoint {
        id: "cp2".to_string(),
        message: "Approve?".to_string(),
        display_data: serde_json::json!(null),
        status: CheckpointStatus::Pending,
        human_input: None,
        notification_url: None,
        created_at_ms: now_ms(),
        resumed_at_ms: None,
    });

    let result = store.resume("cp2", serde_json::json!({"decision": "yes"}));
    assert!(result.is_ok());
    let resumed = result.unwrap();
    assert_eq!(resumed.status, CheckpointStatus::Resumed);
    assert_eq!(
        resumed.human_input,
        Some(serde_json::json!({"decision": "yes"}))
    );
    assert!(resumed.resumed_at_ms.is_some());
}

#[test]
fn checkpoint_store_resume_missing_returns_error() {
    let store = CheckpointStore::new();
    let result = store.resume("nope", serde_json::json!({}));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

#[test]
fn checkpoint_store_resume_already_resumed_returns_error() {
    let store = CheckpointStore::new();
    store.create(Checkpoint {
        id: "cp3".to_string(),
        message: "once".to_string(),
        display_data: serde_json::json!(null),
        status: CheckpointStatus::Pending,
        human_input: None,
        notification_url: None,
        created_at_ms: now_ms(),
        resumed_at_ms: None,
    });

    // First resume succeeds
    assert!(store.resume("cp3", serde_json::json!({"ok": true})).is_ok());

    // Second resume fails
    let result = store.resume("cp3", serde_json::json!({"ok": false}));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not in pending state"));
}

// ── JSON-RPC helper unit tests ──────────────────────────────────────────

#[test]
fn jsonrpc_ok_format() {
    let resp = jsonrpc_ok(
        serde_json::json!(42),
        serde_json::json!({"answer": "hello"}),
    );
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 42);
    assert_eq!(body["result"]["answer"], "hello");
    assert!(body.get("error").is_none());
}

#[test]
fn jsonrpc_ok_with_null_id() {
    let resp = jsonrpc_ok(serde_json::json!(null), serde_json::json!("ok"));
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert!(body["id"].is_null());
    assert_eq!(body["result"], "ok");
}

#[test]
fn jsonrpc_err_format() {
    let resp = jsonrpc_err(serde_json::json!(7), -32601, "Method not found");
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 7);
    assert!(body.get("result").is_none());
    assert_eq!(body["error"]["code"], -32601);
    assert_eq!(body["error"]["message"], "Method not found");
}

#[test]
fn jsonrpc_err_with_string_id() {
    let resp = jsonrpc_err(serde_json::json!("req-abc"), -32600, "Invalid request");
    let body = resp.0;
    assert_eq!(body["id"], "req-abc");
    assert_eq!(body["error"]["code"], -32600);
}

// ── mcp_tool_result helper tests ────────────────────────────────────────

#[test]
fn mcp_tool_result_success() {
    let resp = mcp_tool_result(serde_json::json!(1), "tool output text".to_string(), false);
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);

    let content = &body["result"]["content"];
    assert!(content.is_array());
    let items = content.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "text");
    assert_eq!(items[0]["text"], "tool output text");
    assert_eq!(body["result"]["isError"], false);
}

#[test]
fn mcp_tool_result_error() {
    let resp = mcp_tool_result(
        serde_json::json!(2),
        "something went wrong".to_string(),
        true,
    );
    let body = resp.0;
    assert_eq!(body["result"]["isError"], true);
    assert_eq!(body["result"]["content"][0]["text"], "something went wrong");
}
