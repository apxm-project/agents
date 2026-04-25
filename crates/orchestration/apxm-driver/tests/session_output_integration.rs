use apxm_compiler::{AirEdge, AirModule, AirNode};
use apxm_core::constants;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::events::kind;
use apxm_core::events::payload::{OperationEndPayload, OperationStartPayload};
use apxm_core::events::{ApxmEvent, EventSource};
use apxm_core::paths::session_node_dir_name;
use apxm_core::types::{
    AISOperationType, DependencyType, NodeMetrics, OperationMetric, SessionStatus, Value,
};
use apxm_driver::session_output::SessionEventEmitter;
use apxm_runtime::ExecutionEventEmitter;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

const CLAUDE_PROFILE: &str = "claude";
const CLAUDE_CONTEXT_FILE: &str = "CLAUDE.md";
const MOCK_AGENT_PROFILE: &str = "mock-profile";
const EXECUTION_ID: &str = "exec-123";

#[derive(Debug, Deserialize)]
struct NodeStatusFile {
    status: SessionStatus,
    duration_ms: u128,
    retries: u32,
    error: Option<String>,
}

fn make_graph(nodes: Vec<AirNode>, edges: Vec<AirEdge>) -> AirModule {
    AirModule {
        name: "session-output".to_string(),
        nodes,
        edges,
        parameters: Vec::new(),
        metadata: HashMap::new(),
    }
}

fn make_node(
    id: u64,
    name: &str,
    op: AISOperationType,
    attributes: HashMap<String, Value>,
) -> AirNode {
    AirNode {
        id,
        name: name.to_string(),
        op,
        attributes,
    }
}

fn setup_project_root() -> TempDir {
    let dir = tempfile::tempdir().expect("project root");
    let skills_root = dir.path().join(".agents/skills");
    fs::create_dir_all(skills_root.join(CLAUDE_PROFILE)).expect("claude skill dir");
    fs::create_dir_all(skills_root.join(MOCK_AGENT_PROFILE)).expect("mock skill dir");
    fs::create_dir_all(skills_root.join("spawn_agent")).expect("spawn_agent skill dir");
    fs::write(
        skills_root.join(CLAUDE_PROFILE).join("SKILL.md"),
        "# Claude\n",
    )
    .expect("claude skill");
    fs::write(
        skills_root.join(MOCK_AGENT_PROFILE).join("SKILL.md"),
        "# Mock Profile\n",
    )
    .expect("mock skill");
    fs::write(skills_root.join("spawn_agent/SKILL.md"), "# Spawn Agent\n")
        .expect("spawn_agent skill");
    dir
}

fn make_emitter(session_dir: &Path, project_root: &Path, graph: &AirModule) -> SessionEventEmitter {
    SessionEventEmitter::new(
        session_dir,
        EXECUTION_ID.to_string(),
        Some(graph),
        Some(project_root),
    )
    .expect("session emitter")
}

fn read_trace(path: &Path) -> Vec<ApxmEvent> {
    fs::read_to_string(path)
        .expect("trace.ndjson")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("trace event"))
        .collect()
}

fn assert_runtime_source(event: &ApxmEvent) {
    match &event.meta.source {
        EventSource::Runtime => {}
        other => panic!("expected runtime event source, got {other:?}"),
    }
}

#[test]
fn node_workspace_creation_writes_context_and_outputs() {
    let session_root = tempfile::tempdir().expect("session root");
    let project_root = setup_project_root();
    let graph = make_graph(
        vec![
            make_node(1, "seed", AISOperationType::ConstStr, HashMap::new()),
            make_node(
                2,
                "spawn_architect",
                AISOperationType::SpawnAgent,
                HashMap::from([
                    (
                        graph_attrs::PROFILE.to_string(),
                        Value::String(CLAUDE_PROFILE.to_string()),
                    ),
                    (
                        graph_attrs::TASK_SPEC.to_string(),
                        Value::String("Draft the node workspace rollout".to_string()),
                    ),
                ]),
            ),
            make_node(3, "sink", AISOperationType::ConstStr, HashMap::new()),
        ],
        vec![
            AirEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 2,
                to: 3,
                dependency: DependencyType::Data,
            },
        ],
    );

    let emitter = make_emitter(session_root.path(), project_root.path(), &graph);
    emitter.set_total_nodes(3);

    emitter.emit_operation_start(1, AISOperationType::ConstStr);
    emitter.emit_node_output(1, &Value::String("upstream design".to_string()));
    emitter.emit_operation_end(
        1,
        AISOperationType::ConstStr,
        Duration::from_millis(5),
        true,
        None,
        None,
    );

    emitter.emit_operation_start(2, AISOperationType::SpawnAgent);

    let seed_dir = session_root
        .path()
        .join(constants::session::files::NODES_DIR)
        .join(session_node_dir_name(1, "seed"));
    let spawn_dir = session_root
        .path()
        .join(constants::session::files::NODES_DIR)
        .join(session_node_dir_name(2, "spawn_architect"));

    assert!(seed_dir.is_dir());
    assert!(spawn_dir.is_dir());

    let claude_md = fs::read_to_string(spawn_dir.join(CLAUDE_CONTEXT_FILE)).expect("CLAUDE.md");
    assert!(claude_md.contains("Draft the node workspace rollout"));
    assert!(claude_md.contains("upstream design"));
    assert!(claude_md.contains("skills/claude/SKILL.md"));
    assert!(claude_md.contains("skills/spawn_agent/SKILL.md"));

    let output_json = fs::read_to_string(seed_dir.join(constants::session::node::OUTPUT_JSON))
        .expect("seed output");
    assert!(output_json.contains("upstream design"));
}

#[test]
fn llm_prompt_and_response_are_persisted() {
    let session_root = tempfile::tempdir().expect("session root");
    let project_root = setup_project_root();
    let graph = make_graph(
        vec![make_node(
            1,
            "ask_plan",
            AISOperationType::Ask,
            HashMap::new(),
        )],
        Vec::new(),
    );

    let emitter = make_emitter(session_root.path(), project_root.path(), &graph);

    emitter.emit_operation_start(1, AISOperationType::Ask);
    emitter.emit_llm_prompt(1, "Write the implementation plan");
    emitter.emit_llm_token_for_node(1, "step one ");
    emitter.emit_llm_token_for_node(1, "step two");
    emitter.emit_node_output(1, &Value::String("step one step two".to_string()));
    emitter.emit_operation_end(
        1,
        AISOperationType::Ask,
        Duration::from_millis(8),
        true,
        None,
        None,
    );

    let ask_dir = session_root
        .path()
        .join(constants::session::files::NODES_DIR)
        .join(session_node_dir_name(1, "ask_plan"));

    let prompt =
        fs::read_to_string(ask_dir.join(constants::session::node::PROMPT_TXT)).expect("prompt.txt");
    let response = fs::read_to_string(ask_dir.join(constants::session::node::RESPONSE_TXT))
        .expect("response.txt");
    let output = fs::read_to_string(ask_dir.join(constants::session::node::OUTPUT_JSON))
        .expect("output.json");

    assert_eq!(prompt, "Write the implementation plan");
    assert_eq!(response, "step one step two");
    assert!(output.contains("step one step two"));
}

#[test]
fn node_lifecycle_events_are_persisted_to_root_and_node_traces() {
    let session_root = tempfile::tempdir().expect("session root");
    let project_root = setup_project_root();
    let graph = make_graph(
        vec![make_node(
            1,
            "seed",
            AISOperationType::ConstStr,
            HashMap::new(),
        )],
        Vec::new(),
    );

    let emitter = make_emitter(session_root.path(), project_root.path(), &graph);
    emitter.emit_operation_start(1, AISOperationType::ConstStr);
    emitter.emit_operation_end(
        1,
        AISOperationType::ConstStr,
        Duration::from_millis(7),
        true,
        None,
        None,
    );

    let root_events = read_trace(&session_root.path().join(constants::session::files::TRACE));
    assert_eq!(root_events.len(), 2);
    assert_eq!(root_events[0].kind(), kind::OPERATION_START);
    assert_eq!(root_events[1].kind(), kind::OPERATION_END);

    for event in &root_events {
        assert_eq!(event.meta.trace_id, EXECUTION_ID);
        assert_runtime_source(event);
    }

    let start_payload = root_events[0]
        .payload
        .downcast_ref::<OperationStartPayload>()
        .expect("operation start payload");
    assert_eq!(start_payload.node_id, 1);
    assert_eq!(start_payload.op_type, AISOperationType::ConstStr);

    let end_payload = root_events[1]
        .payload
        .downcast_ref::<OperationEndPayload>()
        .expect("operation end payload");
    assert_eq!(end_payload.node_id, 1);
    assert_eq!(end_payload.op_type, AISOperationType::ConstStr);
    assert_eq!(end_payload.duration_ms, 7);
    assert!(end_payload.success);

    let node_dir = session_root
        .path()
        .join(constants::session::files::NODES_DIR)
        .join(session_node_dir_name(1, "seed"));
    let node_events = read_trace(&node_dir.join(constants::session::node::TRACE_NDJSON));

    assert_eq!(node_events.len(), 2);
    assert_eq!(node_events[0].kind(), kind::OPERATION_START);
    assert_eq!(node_events[1].kind(), kind::OPERATION_END);
    assert_eq!(
        node_events[0].payload.to_json(),
        root_events[0].payload.to_json()
    );
    assert_eq!(
        node_events[1].payload.to_json(),
        root_events[1].payload.to_json()
    );
}

#[test]
fn failed_node_lifecycle_is_persisted_as_unsuccessful_operation_end() {
    let session_root = tempfile::tempdir().expect("session root");
    let project_root = setup_project_root();
    let graph = make_graph(
        vec![make_node(
            1,
            "seed",
            AISOperationType::ConstStr,
            HashMap::new(),
        )],
        Vec::new(),
    );

    let emitter = make_emitter(session_root.path(), project_root.path(), &graph);
    emitter.emit_operation_start(1, AISOperationType::ConstStr);
    emitter.emit_operation_end(
        1,
        AISOperationType::ConstStr,
        Duration::from_millis(7),
        false,
        None,
        None,
    );

    let root_events = read_trace(&session_root.path().join(constants::session::files::TRACE));
    assert_eq!(root_events[1].kind(), kind::OPERATION_END);
    let end_payload = root_events[1]
        .payload
        .downcast_ref::<OperationEndPayload>()
        .expect("operation end payload");
    assert!(!end_payload.success);

    let node_dir = session_root
        .path()
        .join(constants::session::files::NODES_DIR)
        .join(session_node_dir_name(1, "seed"));
    let status: NodeStatusFile = serde_json::from_str(
        &fs::read_to_string(node_dir.join(constants::session::node::STATUS_JSON))
            .expect("status.json"),
    )
    .expect("status json");
    assert_eq!(status.status, SessionStatus::Failed);
    assert_eq!(status.duration_ms, 7);
    assert_eq!(status.retries, 0);
    assert!(status.error.is_some());
}

#[test]
fn graph_lifecycle_callbacks_do_not_write_session_trace_events() {
    let session_root = tempfile::tempdir().expect("session root");
    let project_root = setup_project_root();
    let graph = make_graph(
        vec![make_node(
            1,
            "seed",
            AISOperationType::ConstStr,
            HashMap::new(),
        )],
        Vec::new(),
    );

    let emitter = make_emitter(session_root.path(), project_root.path(), &graph);
    emitter.emit_graph_start(EXECUTION_ID, 1);
    emitter.emit_graph_end(EXECUTION_ID, 1, true);

    let root_events = read_trace(&session_root.path().join(constants::session::files::TRACE));
    assert!(root_events.is_empty());
}

#[test]
fn node_metrics_are_persisted() {
    let session_root = tempfile::tempdir().expect("session root");
    let project_root = setup_project_root();
    let graph = make_graph(
        vec![make_node(
            1,
            "spawn_architect",
            AISOperationType::SpawnAgent,
            HashMap::new(),
        )],
        Vec::new(),
    );

    let emitter = make_emitter(session_root.path(), project_root.path(), &graph);
    emitter.emit_operation_start(1, AISOperationType::SpawnAgent);

    let mut metrics = NodeMetrics::new(1);
    metrics.record_operation(OperationMetric {
        node_id: 1,
        op_type: AISOperationType::SpawnAgent,
        duration_ms: 12,
        success: true,
    });
    emitter.emit_node_metrics(1, &metrics);
    emitter.emit_operation_end(
        1,
        AISOperationType::SpawnAgent,
        Duration::from_millis(12),
        true,
        None,
        None,
    );

    let node_dir = session_root
        .path()
        .join(constants::session::files::NODES_DIR)
        .join(session_node_dir_name(1, "spawn_architect"));
    let metrics_json = fs::read_to_string(node_dir.join(constants::session::node::METRICS_JSON))
        .expect("metrics.json");
    assert!(metrics_json.contains("\"operation\""));
    assert!(metrics_json.contains("\"processes\""));
    assert!(metrics_json.contains("\"attempts\": 1"));
}

#[test]
fn spawn_agent_skill_resolution_copies_profile_and_operation_skills() {
    let session_root = tempfile::tempdir().expect("session root");
    let project_root = setup_project_root();
    let graph = make_graph(
        vec![make_node(
            2,
            "spawn_architect",
            AISOperationType::SpawnAgent,
            HashMap::from([(
                graph_attrs::PROFILE.to_string(),
                Value::String(MOCK_AGENT_PROFILE.to_string()),
            )]),
        )],
        Vec::new(),
    );

    let emitter = make_emitter(session_root.path(), project_root.path(), &graph);
    emitter.emit_operation_start(2, AISOperationType::SpawnAgent);

    let skills_dir = session_root
        .path()
        .join(constants::session::files::NODES_DIR)
        .join(session_node_dir_name(2, "spawn_architect"))
        .join(constants::session::node::SKILLS_DIR);
    let profile_skill = skills_dir.join(MOCK_AGENT_PROFILE).join("SKILL.md");
    let op_skill = skills_dir.join("spawn_agent/SKILL.md");

    assert!(profile_skill.is_file());
    assert!(op_skill.is_file());
    assert!(
        !fs::symlink_metadata(&profile_skill)
            .expect("profile metadata")
            .file_type()
            .is_symlink()
    );
    assert!(
        !fs::symlink_metadata(&op_skill)
            .expect("spawn_agent metadata")
            .file_type()
            .is_symlink()
    );
}
