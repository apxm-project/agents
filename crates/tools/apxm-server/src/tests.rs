// Integration Tests
//
// These tests exercise the HTTP API layer without binding to a TCP port.
// They use `tower::ServiceExt::oneshot` + `axum::body::Body` to send requests
// directly to the Axum router and collect responses via `http_body_util::BodyExt`.
//
// The focused modules below keep protocol basics, MCP, task/checkpoint routes,
// raw graph execution, and skill inventory/admission/execution/record tests
// separated while sharing fixtures from this file.
//
// No LLM API key is required — all LLM-touching tests are gated behind
// `#[cfg(feature = "integration")]` and use stub responses.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use apxm_artifact::{Artifact, ArtifactMetadata};
use apxm_backends::llm::backends::MockLLMBackend;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::events::payload::{REDACTION_HASH_PREFIX_BLAKE3, RedactedContent};
use apxm_core::types::AISOperationType;
use apxm_core::types::NodeMetrics;
use apxm_core::types::execution::{DagMetadata, ExecutionDag, Node, NodeMetadata};
use apxm_core::types::values::Value;
use apxm_runtime::capability::executor::CapabilityExecutor;
use apxm_runtime::capability::metadata::CapabilityMetadata;
use apxm_runtime::{
    DefaultBackend, ExecRequest, ExecResult, IsolationLevel, ModelRouterConfig, Runtime,
    RuntimeConfig, SandboxBackend, SandboxCapabilities, SandboxContext, SandboxError,
    SandboxRegistry, TransitionLabel, ValidationResult,
};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use dashmap::DashMap;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::build_app;
use crate::build_server_runtime;
use crate::checkpoints::{Checkpoint, CheckpointStatus, CheckpointStore};
use crate::execute::{ExecuteRequest, ExecuteResponse, prepare_request};
use crate::executions::{ExecutionStore, execution_record_snapshot_path};
use crate::helpers::{jsonrpc_err, jsonrpc_ok, mcp_tool_result, now_ms};
use crate::mcp::{
    MCP_METHOD_INITIALIZE, MCP_METHOD_RESOURCES_LIST, MCP_METHOD_RESOURCES_READ,
    MCP_METHOD_TOOLS_CALL, MCP_METHOD_TOOLS_LIST, MCP_RESOURCE_PARAM_URI as MCP_PARAM_URI,
    MCP_TOOL_APXM_AAM_RECALL, MCP_TOOL_APXM_CAPABILITY_LIST, MCP_TOOL_APXM_EVIDENCE_LOOKUP,
    MCP_TOOL_APXM_GOAL_CANCEL, MCP_TOOL_APXM_GOAL_EVENTS, MCP_TOOL_APXM_GOAL_START,
    MCP_TOOL_APXM_GOAL_STATUS, MCP_TOOL_APXM_PROMPT_AS_WORKFLOW, MCP_TOOL_APXM_SKILL_CALL,
    MCP_TOOL_APXM_SKILL_GET, MCP_TOOL_APXM_SKILL_VALIDATE, MCP_TOOL_APXM_SKILLS_LIST,
    MCP_TOOL_APXM_TRACE_FETCH, MCP_TOOL_APXM_WORKFLOW_CANCEL, MCP_TOOL_APXM_WORKFLOW_EVENTS,
    MCP_TOOL_APXM_WORKFLOW_START, MCP_TOOL_APXM_WORKFLOW_STATUS,
    MCP_TOOL_PARAM_ARGUMENTS as MCP_PARAM_ARGUMENTS, MCP_TOOL_PARAM_NAME as MCP_PARAM_NAME,
};
use crate::mcp_protocol::{
    admission_error, args as mcp_args, fields as mcp_fields, status as mcp_status, tool_result,
    workflow_skill,
};
use crate::routes;
use crate::skill_resources::{prepend_builtin_skill_root, skill_uri as skill_resource_uri};
use crate::skills::{SkillLibrary, parse_skill_roots};
use crate::state::AppState;
use crate::tasks::{QueuedTask, TaskQueueManager, TaskStatus};
use crate::types::responses::{ExecutionStats, LlmUsageSummary};

// NOTE: these submodules (`src/tests/agent.rs`, etc.) are not present in the
// tree — this integration harness file is currently wired in via
// `#[cfg(test)] mod tests;` but its sibling submodule files do not exist, so
// the declarations below are commented out to keep the harness + the tests
// that live directly in this file compiling. Re-enable a line only when its
// backing file is restored.
// mod agent;
// mod basic;
// mod call_skill_isolation;
// mod checkpoints;
// mod execute;
// mod goals;
// mod helpers;
// mod mcp;
// mod runs;
// mod skills_admission;
// mod skills_execution;
// mod skills_inventory;
// mod skills_records;
// mod skills_streaming;
// mod tasks;
// mod webhook;

// ── Test helpers ──────────────────────────────────────────────────────────

const MCP_JSONRPC_VERSION: &str = "2.0";
const MCP_REQUEST_ID: u64 = 23;
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
const STATUS_SUCCEEDED: &str = apxm_core::constants::orchestration::execution_status::SUCCEEDED;
const STATUS_RUNNING: &str = apxm_core::constants::orchestration::execution_status::RUNNING;
const STATUS_FAILED: &str = apxm_core::constants::orchestration::execution_status::FAILED;

const FIXTURE_PACKAGE_DIR: &str = "checkout";
const FIXTURE_SKILL_SESSION_DIR: &str = "skills";
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
const FIXTURE_AAM_KEY: &str = "fixture_mcp_belief";
const FIXTURE_AAM_VALUE: &str = "mcp memory ready";
const FIXTURE_AAM_QUERY: &str = "fixture_mcp";
const FIXTURE_EVIDENCE_QUERY: &str = "rust implementation";
const FIXTURE_WORKFLOW_BACKEND: &str = "mock-workflow";
const FIXTURE_WORKFLOW_NODE_NAME: &str = "final";
const FIXTURE_WORKFLOW_TASK: &str = "summarize release risk";
const FIXTURE_WORKFLOW_TRACE_ID: &str = "workflow-record-test";
const FIXTURE_WORKFLOW_SANDBOX_TRACE_ID: &str = "workflow-sandbox-test";
const FIXTURE_WORKFLOW_INVALID_TRACE_ID: &str = "../bad";
const FIXTURE_WORKFLOW_REPAIR_MARKER: &str = "Compiler feedback to repair AIR:";
const FIXTURE_WRITE_TOOL: &str = apxm_core::constants::capabilities::WRITE;
const FIXTURE_NODE_ID: u64 = 1;
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
const ERROR_SANDBOX_PREFLIGHT: &str = "sandbox preflight";
const ERROR_SANDBOX_DEGRADED: &str = "degraded guarantees";
const ERROR_MCP_AGENT_SAFE: &str = "not read-only";
const SIDE_EFFECT_POLICY_SANDBOXED: &str = "sandboxed";
const FIXTURE_SANDBOX_NAME: &str = "fixture-policy-sandbox";
const FIXTURE_DEGRADED_SANDBOX_NAME: &str = "fixture-degraded-sandbox";
const FIXTURE_DEGRADED_SANDBOX_WARNING: &str = "fixture sandbox degraded guarantees";

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

struct FixtureSideEffectCapability {
    metadata: CapabilityMetadata,
}

impl FixtureSideEffectCapability {
    fn new(name: &str) -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                name,
                "Fixture side-effectful direct capability",
                serde_json::json!({ "type": "object", "properties": {} }),
            )
            .with_returns("string"),
        }
    }
}

#[async_trait]
impl CapabilityExecutor for FixtureSideEffectCapability {
    async fn execute(&self, _args: HashMap<String, Value>) -> Result<Value, RuntimeError> {
        Ok(Value::String(FIXTURE_OUTPUT.to_string()))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

struct FixtureRequiredArgCapability {
    metadata: CapabilityMetadata,
}

impl FixtureRequiredArgCapability {
    fn new(name: &str) -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                name,
                "Fixture capability with a required argument",
                serde_json::json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["message"],
                    "properties": {
                        "message": { "type": "string" }
                    }
                }),
            )
            .with_returns("string")
            .with_read_only(),
        }
    }
}

#[async_trait]
impl CapabilityExecutor for FixtureRequiredArgCapability {
    async fn execute(&self, _args: HashMap<String, Value>) -> Result<Value, RuntimeError> {
        Ok(Value::String(FIXTURE_OUTPUT.to_string()))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

struct FixtureSandboxedCapability {
    metadata: CapabilityMetadata,
}

impl FixtureSandboxedCapability {
    fn new(name: &str) -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                name,
                "Fixture side-effectful sandboxed capability",
                serde_json::json!({ "type": "object", "properties": {} }),
            )
            .with_returns("string"),
        }
    }
}

#[async_trait]
impl CapabilityExecutor for FixtureSandboxedCapability {
    async fn execute(&self, _args: HashMap<String, Value>) -> Result<Value, RuntimeError> {
        Err(RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message: "fixture sandboxed capability must run through sandbox".to_string(),
        })
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }

    fn to_exec_request(&self, _args: &HashMap<String, Value>) -> Option<ExecRequest> {
        Some(ExecRequest {
            min_isolation: IsolationLevel::PolicyOnly,
            program: "fixture-sandboxed-tool".to_string(),
            origin_op: Some("inv_tool".to_string()),
            ..Default::default()
        })
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
/// The runtime has no LLM backends registered, so any workflow that calls
/// ASK/THINK/REASON will error. Tests that need execution should use AIR
/// composed entirely of CONST_STR and synchronisation ops.
async fn test_state() -> AppState {
    test_state_with_skill_roots(Vec::new()).await
}

async fn test_state_with_mock_workflow_response(plan_response: impl Into<String>) -> AppState {
    let runtime =
        runtime_with_mock_workflow_backend(MockLLMBackend::static_response(plan_response.into()))
            .await;
    test_state_with_runtime_and_skill_roots(runtime, Vec::new()).await
}

async fn runtime_with_mock_workflow_backend(backend: MockLLMBackend) -> Runtime {
    let mut runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    runtime
        .llm_registry()
        .register(FIXTURE_WORKFLOW_BACKEND, backend)
        .expect("register mock plan backend");
    runtime
        .llm_registry()
        .set_default(FIXTURE_WORKFLOW_BACKEND)
        .expect("set mock plan backend default");
    runtime
        .init_model_router(ModelRouterConfig::default())
        .expect("init test model router");
    runtime
}

async fn test_state_with_skill_roots(skill_roots: Vec<std::path::PathBuf>) -> AppState {
    test_state_with_skill_roots_and_execution_store(skill_roots, ExecutionStore::new()).await
}

async fn test_state_with_skill_roots_and_execution_store(
    skill_roots: Vec<std::path::PathBuf>,
    execution_store: ExecutionStore,
) -> AppState {
    // Use in-memory LTM to avoid SQLite file-locking across parallel tests.
    let runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    let mut runtime = Arc::new(runtime);
    let skill_library = SkillLibrary::new(skill_roots);
    install_test_runtime_bridges(&mut runtime, skill_library.clone());
    let server_config = apxm_driver::ServerConfig::default();
    let hardening = crate::state::HardeningDefaults::for_config(&server_config);
    AppState {
        runtime,
        agent_registry: Arc::new(DashMap::new()),
        task_manager: TaskQueueManager::new(),
        checkpoint_store: CheckpointStore::new(),
        start_time: SystemTime::now(),
        a2a_tasks: Arc::new(DashMap::new()),
        skill_library,
        execution_store,
        run_event_bus: crate::runs::RunEventBus::new(),
        webhook_dispatcher: None,
        rollout_paths: std::sync::Arc::new(apxm_rollout::RolloutPaths::new({
            // Forget the tempdir so its lifetime spans the AppState; the test
            // process exits cleanly and the OS reclaims /tmp on its own.
            let dir = tempfile::tempdir().expect("rollout home");
            let path = dir.path().to_path_buf();
            std::mem::forget(dir);
            path
        })),
        rollout_index: std::sync::Arc::new(tokio::sync::Mutex::new(
            apxm_rollout::IndexDb::open_in_memory().expect("rollout index"),
        )),
        rollout_registry: crate::rollout::RolloutRegistry::new(),
        inference_limiter: crate::state::InferenceLimiter::unlimited_for_tests(),
        server_config,
        bind_addr: hardening.bind_addr,
        effective_require_auth: hardening.effective_require_auth,
        safety_state: hardening.safety_state,
        shutdown: hardening.shutdown,
        cancel_registry: Arc::new(DashMap::new()),
        goal_runs: crate::goal_runs::GoalRunRegistry::new(),
        session_registry: crate::conversations::SessionRegistry::new(),
    }
}

async fn test_state_with_runtime_and_skill_roots(
    runtime: Runtime,
    skill_roots: Vec<std::path::PathBuf>,
) -> AppState {
    let mut runtime = Arc::new(runtime);
    let skill_library = SkillLibrary::new(skill_roots);
    install_test_runtime_bridges(&mut runtime, skill_library.clone());
    let server_config = apxm_driver::ServerConfig::default();
    let hardening = crate::state::HardeningDefaults::for_config(&server_config);
    AppState {
        runtime,
        agent_registry: Arc::new(DashMap::new()),
        task_manager: TaskQueueManager::new(),
        checkpoint_store: CheckpointStore::new(),
        start_time: SystemTime::now(),
        a2a_tasks: Arc::new(DashMap::new()),
        skill_library,
        execution_store: ExecutionStore::new(),
        run_event_bus: crate::runs::RunEventBus::new(),
        webhook_dispatcher: None,
        rollout_paths: std::sync::Arc::new(apxm_rollout::RolloutPaths::new({
            let dir = tempfile::tempdir().expect("rollout home");
            let path = dir.path().to_path_buf();
            std::mem::forget(dir);
            path
        })),
        rollout_index: std::sync::Arc::new(tokio::sync::Mutex::new(
            apxm_rollout::IndexDb::open_in_memory().expect("rollout index"),
        )),
        rollout_registry: crate::rollout::RolloutRegistry::new(),
        inference_limiter: crate::state::InferenceLimiter::unlimited_for_tests(),
        server_config,
        bind_addr: hardening.bind_addr,
        effective_require_auth: hardening.effective_require_auth,
        safety_state: hardening.safety_state,
        shutdown: hardening.shutdown,
        cancel_registry: Arc::new(DashMap::new()),
        goal_runs: crate::goal_runs::GoalRunRegistry::new(),
        session_registry: crate::conversations::SessionRegistry::new(),
    }
}

fn install_test_runtime_bridges(runtime: &mut Arc<Runtime>, skill_library: SkillLibrary) {
    let (skill_resolver, workflow_spawner) = {
        let runtime_mut = Arc::get_mut(runtime)
            .expect("test runtime bridges must be installed before runtime is shared");
        let skill_resolver = crate::call_skill::install_unattached(runtime_mut, skill_library);
        let workflow_spawner =
            apxm_driver::runtime::install_workflow_spawner_unattached(runtime_mut, None);
        (skill_resolver, workflow_spawner)
    };
    skill_resolver.attach_runtime(runtime);
    workflow_spawner.attach_runtime(runtime);
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

#[tokio::test]
async fn cancel_route_trips_in_flight_run_and_404s_unknown() {
    let state = test_state().await;
    // Share the registry Arc with the app and seed an in-flight handle, as
    // `execute_stream` would on a live run.
    let registry = Arc::clone(&state.cancel_registry);
    let notify = Arc::new(tokio::sync::Notify::new());
    registry.insert("exec-cancel-test".to_string(), Arc::clone(&notify));
    let app = crate::build_app(state);

    // A known in-flight run cancels and reports back.
    let (status, body) = post_json(
        app.clone(),
        &crate::routes::run_cancel_path("exec-cancel-test"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["cancelled"], serde_json::json!(true));
    assert_eq!(body["execution_id"], serde_json::json!("exec-cancel-test"));

    // The abort signal actually fired — a waiter wakes promptly.
    tokio::time::timeout(std::time::Duration::from_millis(500), notify.notified())
        .await
        .expect("cancel must trip the run's Notify");

    // An unknown / already-settled run is a 404, not a 500.
    let (status, _) = post_json(
        app.clone(),
        &crate::routes::run_cancel_path("nonexistent-run"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

fn mock_yield_air_response() -> String {
    format!(
        r#"module {{
  func.func @{entry}() -> !ais.token attributes {{ais.entry}} {{
    %{node} = ais.const_str "{output}" : !ais.token
    func.return %{node} : !ais.token
  }}
}}
"#,
        entry = FIXTURE_ENTRY_FLOW,
        node = FIXTURE_WORKFLOW_NODE_NAME,
        output = FIXTURE_OUTPUT
    )
}

fn mock_inv_tool_air_response(capability: &str) -> String {
    format!(
        r#"module {{
  func.func @{entry}() -> !ais.token attributes {{ais.entry}} {{
    %{node} = ais.inv_tool "{capability}" ("{{}}") : !ais.token
    func.return %{node} : !ais.token
  }}
}}
"#,
        entry = FIXTURE_ENTRY_FLOW,
        node = FIXTURE_WORKFLOW_NODE_NAME,
        capability = capability
    )
}

fn mock_invalid_air_response() -> String {
    "module { func.func @main() -> !ais.token attributes {ais.entry} { %bad = ais.unknown_op : !ais.token func.return %bad : !ais.token } }".to_string()
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

fn write_sandboxed_policy_skill_with_artifact(
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
side_effect_policy = "{SIDE_EFFECT_POLICY_SANDBOXED}"
"#
        ),
    );
}

fn write_broader_policy_skill_with_artifact(
    root: &std::path::Path,
    artifact_bytes: &[u8],
    required_capability: &str,
    allowed_tool: &str,
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
required_capabilities = ["{required_capability}"]
allowed_tools = ["{allowed_tool}"]
side_effect_policy = "{side_effect_policy}"
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

fn skill_artifact_bytes_with_embedded_manifest(manifest_toml: &str) -> Vec<u8> {
    let mut artifact = Artifact::from_bytes(&skill_artifact_bytes(AISOperationType::ConstStr))
        .expect("fixture artifact");
    artifact.replace_section(
        apxm_artifact::section_kinds::SKILL_MANIFEST_V1,
        manifest_toml.as_bytes().to_vec(),
    );
    artifact.to_bytes().expect("fixture artifact bytes")
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

fn fixture_execute_response(session_dir: Option<String>) -> ExecuteResponse {
    ExecuteResponse {
        results: HashMap::new(),
        content: Some(FIXTURE_OUTPUT.to_string()),
        session_dir,
        stats: ExecutionStats {
            executed_nodes: 1,
            failed_nodes: 0,
            duration_ms: 0,
        },
        llm_usage: LlmUsageSummary {
            input_tokens: 0,
            output_tokens: 0,
            total_requests: 0,
        },
        tool_call_counts: HashMap::new(),
    }
}

fn fixture_sandbox_registry() -> SandboxRegistry {
    let mut registry = SandboxRegistry::new();
    registry.register(Arc::new(DefaultBackend::new(
        SandboxCapabilities {
            isolation_level: IsolationLevel::PolicyOnly,
            supports_filesystem_restriction: true,
            supports_network_restriction: true,
            supports_syscall_filtering: false,
            supports_resource_limits: true,
            name: FIXTURE_SANDBOX_NAME.to_string(),
            version: "test".to_string(),
        },
        |_request| async {
            Ok(ExecResult {
                success: true,
                exit_code: Some(0),
                stdout: FIXTURE_OUTPUT.to_string(),
                stderr: String::new(),
                duration: std::time::Duration::from_millis(1),
                timed_out: false,
            })
        },
    )));
    registry
}

struct FixtureDegradedSandboxBackend;

#[async_trait]
impl SandboxBackend for FixtureDegradedSandboxBackend {
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            isolation_level: IsolationLevel::PolicyOnly,
            supports_filesystem_restriction: false,
            supports_network_restriction: false,
            supports_syscall_filtering: false,
            supports_resource_limits: false,
            name: FIXTURE_DEGRADED_SANDBOX_NAME.to_string(),
            version: "test".to_string(),
        }
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(&self, _request: &ExecRequest) -> ValidationResult {
        ValidationResult::Degraded {
            warnings: vec![FIXTURE_DEGRADED_SANDBOX_WARNING.to_string()],
        }
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        unreachable!("degraded fixture backend must be rejected before session creation")
    }

    async fn execute(
        &self,
        _ctx: &SandboxContext,
        _request: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        unreachable!("degraded fixture backend must be rejected before execution")
    }

    async fn destroy_session(&self, _ctx: SandboxContext) -> Result<(), SandboxError> {
        unreachable!("degraded fixture backend must be rejected before cleanup")
    }
}

fn fixture_degraded_sandbox_registry() -> SandboxRegistry {
    let mut registry = SandboxRegistry::new();
    registry.register(Arc::new(FixtureDegradedSandboxBackend));
    registry
}

fn skill_detail_route(id: &str) -> String {
    routes::skill_detail_path(id)
}

fn skill_validate_route(id: &str) -> String {
    routes::skill_validate_path(id)
}

fn skill_execute_route(id: &str) -> String {
    routes::skill_execute_path(id)
}

fn skill_execute_stream_route(id: &str) -> String {
    routes::skill_execute_stream_path(id)
}

fn execution_detail_route(id: &str) -> String {
    routes::execution_detail_path(id)
}

fn execution_node_detail_route(execution_id: &str, node_id: u64) -> String {
    routes::execution_node_detail_path(execution_id, node_id)
}

fn run_node_detail_route(execution_id: &str, node_id: u64) -> String {
    format!("/v1/runs/{execution_id}/nodes/{node_id}")
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

fn tool_json(body: &serde_json::Value) -> serde_json::Value {
    serde_json::from_str(tool_text(body)).expect("MCP tool JSON")
}

async fn successful_mcp_tool_json(
    app: Router,
    tool_name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let (status, body) = post_json(app, routes::MCP, mcp_call(tool_name, arguments)).await;
    assert_eq!(status, StatusCode::OK, "MCP tool call failed: {body}");
    assert_eq!(
        body["result"]["isError"], false,
        "MCP tool returned error: {body}"
    );
    tool_json(&body)
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

#[tokio::test]
#[allow(unsafe_code)]
async fn skill_execute_writes_workflow_run_node_artifacts_and_exposes_run_node_detail() {
    let skill_root = tempfile::tempdir().expect("skill root");
    let runs_root = tempfile::tempdir().expect("runs root");
    write_executable_skill(skill_root.path());

    let state = test_state_with_skill_roots(vec![skill_root.path().to_path_buf()]).await;
    let app = crate::build_app(state);

    // SAFETY: this test owns APXM_RUNS_ROOT for the synchronous request and
    // removes it before returning.
    unsafe { std::env::set_var("APXM_RUNS_ROOT", runs_root.path()) };
    let (status, body) = post_json(
        app.clone(),
        &skill_execute_route(FIXTURE_SKILL_ID),
        serde_json::json!({ "workflow_id": "wf-observe" }),
    )
    .await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK, "skill execute failed: {body}");

    let execution_id = body["execution_id"].as_str().expect("execution_id");
    let run_dir = runs_root.path().join("wf-observe").join(execution_id);
    let run_json_path = run_dir.join("run.json");
    assert!(
        run_json_path.is_file(),
        "missing {}",
        run_json_path.display()
    );
    let run_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&run_json_path).expect("run.json bytes"))
            .expect("run.json");
    assert_eq!(run_json["object"], "apxm.run");
    assert_eq!(run_json["artifact_schema_version"], 1);
    assert_eq!(run_json["run_id"], execution_id);
    assert_eq!(run_json["workflow_id"], "wf-observe");
    assert_eq!(run_json["node_output_count"], 1);
    assert_eq!(run_json["node_metric_count"], 1);

    let nodes_root = run_dir.join("nodes");
    let node_dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&nodes_root)
        .expect("nodes dir")
        .map(|entry| entry.expect("node dir entry").path())
        .filter(|path| path.is_dir())
        .collect();
    assert_eq!(
        node_dirs.len(),
        1,
        "expected one node dir under {nodes_root:?}"
    );
    let node_dir = &node_dirs[0];
    let node_json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(node_dir.join("node.json")).expect("node.json bytes"),
    )
    .expect("node.json");
    assert_eq!(node_json["node_id"], 1);
    assert_eq!(node_json["run_id"], execution_id);

    let output_json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(node_dir.join("output.json")).expect("output.json bytes"),
    )
    .expect("output.json");
    assert_eq!(output_json["node_id"], 1);
    assert_eq!(output_json["output"][SUMMARY_FIELD], FIXTURE_OUTPUT_SUMMARY);
    assert_eq!(output_json["output"][REDACTED_FIELD], true);

    let metrics_json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(node_dir.join("metrics.json")).expect("metrics.json bytes"),
    )
    .expect("metrics.json");
    assert_eq!(metrics_json["node_id"], 1);
    assert_eq!(metrics_json["metrics"]["operation"]["successes"], 1);

    let (status, node_body) = get_json(app.clone(), &run_node_detail_route(execution_id, 1)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "run node detail failed: {node_body}"
    );
    assert_eq!(node_body["node_id"], 1);
    assert_eq!(
        node_body["outputs"][0]["output"][SUMMARY_FIELD],
        FIXTURE_OUTPUT_SUMMARY
    );
    assert!(node_body["artifacts"]["node_dir"].as_str().is_some());
    assert!(node_body["artifacts"]["output_json"].as_str().is_some());

    let (status, execution_node_body) =
        get_json(app, &execution_node_detail_route(execution_id, 1)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "execution node detail failed: {execution_node_body}"
    );
    assert_eq!(
        execution_node_body["outputs"][0]["output"][SUMMARY_FIELD],
        FIXTURE_OUTPUT_SUMMARY
    );
}

// ────────────────────────────────────────────────────────────────────
// Session transcript history (`GET /v1/sessions/{id}/history`).
//
// Chat turns from the CLI and studio both POST `/v1/execute/stream` with a
// shared session_id; the stream handler mirrors the runtime event stream into
// the rollout JSONL, and the history route reassembles a role-tagged message
// list keyed by that session_id.
// ────────────────────────────────────────────────────────────────────
mod session_history_tests {
    use super::*;
    use crate::rollout::{RolloutEmitter, session_meta_from_chat};
    use apxm_core::events::payload::{LlmPromptPayload, RedactedContent, TokenPayload};
    use apxm_core::events::{ApxmEvent, EventEmitter, EventSource};

    fn single_ask_air() -> String {
        // One ASK node: drives the mock backend so the runtime emits a real
        // llm_prompt (redacted) + token (assistant reply) event for the turn.
        r#"module {
  func.func @chat() -> !ais.token attributes {ais.entry} {
    %reply = ais.ask "say hello" : !ais.token
    func.return %reply : !ais.token
  }
}
"#
        .to_string()
    }

    /// End-to-end: a real `/v1/execute/stream` turn with a session_id records
    /// the assistant reply durably, and `GET /v1/sessions/{id}/history` returns
    /// it role-tagged.
    #[tokio::test]
    async fn execute_stream_records_turn_and_history_returns_assistant_reply() {
        let runtime =
            runtime_with_mock_workflow_backend(MockLLMBackend::static_response("hello there"))
                .await;
        let state = test_state_with_runtime_and_skill_roots(runtime, Vec::new()).await;
        let app = crate::build_app(state);

        let session_id = "chat-sess-e2e";
        let (status, body) = post_json_text(
            app.clone(),
            crate::routes::EXECUTE_STREAM,
            serde_json::json!({
                "air": single_ask_air(),
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "execute_stream body: {body}");
        // The stream completed (channel closed → body drained), so the rollout
        // recorder was flushed + closed and its index row written.

        let (status, hist) = get_json(
            app.clone(),
            &crate::routes::session_history_path(session_id),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let messages = hist["messages"].as_array().expect("messages array");
        // The assistant reply is recoverable (token event carries raw text).
        let assistant = messages
            .iter()
            .find(|m| m["role"] == "assistant")
            .expect("an assistant message");
        assert!(
            assistant["content"]
                .as_str()
                .unwrap_or_default()
                .contains("hello there"),
            "assistant content should carry the model reply, got: {messages:?}"
        );
        // A user turn is present (from the redacted llm_prompt) even though its
        // raw text is not retained.
        assert!(
            messages.iter().any(|m| m["role"] == "user"),
            "a user turn should be present, got: {messages:?}"
        );
    }

    /// Drive the exact recording sink the stream handler installs
    /// (open_for_run + RolloutEmitter) to assert precise role ordering across
    /// two turns of one session.
    #[tokio::test]
    async fn history_orders_turns_and_tags_roles() {
        let state = test_state().await;
        let app = crate::build_app(state.clone());
        let session_id = "chat-sess-order";

        for (turn, reply) in [
            ("exec-turn-1", "first reply"),
            ("exec-turn-2", "second reply"),
        ] {
            let meta = session_meta_from_chat(turn, session_id, Vec::new());
            state
                .rollout_registry
                .open_for_run(
                    state.rollout_paths.clone(),
                    Some(state.rollout_index.clone()),
                    turn,
                    session_id,
                    meta,
                )
                .await
                .expect("recorder opens");
            let emitter = RolloutEmitter::new(state.rollout_registry.clone(), turn.to_string());
            // user side (redacted) then assistant token.
            emitter.emit(ApxmEvent::root(
                LlmPromptPayload {
                    node_id: 1,
                    node_name: None,
                    prompt: RedactedContent::from_text("hi turn"),
                },
                EventSource::Runtime,
                turn,
            ));
            emitter.emit(ApxmEvent::root(
                TokenPayload {
                    text: reply.to_string(),
                },
                EventSource::Runtime,
                turn,
            ));
            state.rollout_registry.close(turn).await;
        }

        let (status, hist) = get_json(app, &crate::routes::session_history_path(session_id)).await;
        assert_eq!(status, StatusCode::OK);
        let messages = hist["messages"].as_array().expect("messages array");
        // Expect: user, assistant(first), user, assistant(second) — in turn order.
        let roles: Vec<&str> = messages
            .iter()
            .map(|m| m["role"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            roles,
            vec!["user", "assistant", "user", "assistant"],
            "{messages:?}"
        );
        let assistant_contents: Vec<&str> = messages
            .iter()
            .filter(|m| m["role"] == "assistant")
            .map(|m| m["content"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(assistant_contents, vec!["first reply", "second reply"]);
    }

    /// With `user_text` supplied, the recorded user turn carries the REAL
    /// prompt (a typed UserMessage line), not the redacted "text(chars=N)"
    /// summary, and it appears exactly once (no duplicate from the runtime's
    /// redacted llm_prompt event).
    #[tokio::test]
    async fn execute_stream_with_user_text_records_faithful_user_turn() {
        let runtime =
            runtime_with_mock_workflow_backend(MockLLMBackend::static_response("hello there"))
                .await;
        let state = test_state_with_runtime_and_skill_roots(runtime, Vec::new()).await;
        let app = crate::build_app(state);

        let session_id = "chat-sess-usertext";
        let real_prompt = "please summarize the quarterly report";
        let (status, body) = post_json_text(
            app.clone(),
            crate::routes::EXECUTE_STREAM,
            serde_json::json!({
                "air": single_ask_air(),
                "session_id": session_id,
                "user_text": real_prompt,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "execute_stream body: {body}");

        let (status, hist) = get_json(
            app.clone(),
            &crate::routes::session_history_path(session_id),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let messages = hist["messages"].as_array().expect("messages array");

        let user_turns: Vec<&str> = messages
            .iter()
            .filter(|m| m["role"] == "user")
            .map(|m| m["content"].as_str().unwrap_or_default())
            .collect();
        // Exactly one user turn, carrying the real text — not a redaction summary.
        assert_eq!(
            user_turns.len(),
            1,
            "expected exactly one user turn (no duplicate), got: {messages:?}"
        );
        assert_eq!(
            user_turns[0], real_prompt,
            "user turn must be the real text"
        );
        assert!(
            !user_turns[0].contains("text(chars="),
            "user turn must not be the redaction summary, got: {messages:?}"
        );
        // Assistant reply is still faithful.
        assert!(
            messages.iter().any(|m| m["role"] == "assistant"
                && m["content"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("hello there")),
            "assistant reply should still be present, got: {messages:?}"
        );
    }

    #[tokio::test]
    async fn history_unknown_session_is_empty_200() {
        let state = test_state().await;
        let app = crate::build_app(state);
        let (status, hist) = get_json(
            app,
            &crate::routes::session_history_path("never-seen-session"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            hist["messages"].as_array().map(|a| a.len()),
            Some(0),
            "unknown session must be an empty 200, not a 404"
        );
    }

    #[tokio::test]
    async fn history_invalid_session_id_is_400() {
        let state = test_state().await;
        let app = crate::build_app(state);
        // A single path segment containing an out-of-charset character (an
        // encoded space) decodes to one id `bad id` that the validator rejects
        // with 400 — distinct from the 200-empty unknown-session case.
        let (status, _hist) = get_json(app, "/v1/sessions/bad%20id/history").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
