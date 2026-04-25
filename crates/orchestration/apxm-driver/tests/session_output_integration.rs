use apxm_compiler::{AirEdge, AirModule, AirNode};
use apxm_core::constants;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::paths::session_node_dir_name;
use apxm_core::types::{AISOperationType, DependencyType, NodeMetrics, OperationMetric, Value};
use apxm_driver::session_output::SessionEventEmitter;
use apxm_runtime::ExecutionEventEmitter;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

const CLAUDE_PROFILE: &str = "claude";
const CLAUDE_CONTEXT_FILE: &str = "CLAUDE.md";
const MOCK_AGENT_PROFILE: &str = "mock-profile";

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
        "exec-123".to_string(),
        Some(graph),
        Some(project_root),
    )
    .expect("session emitter")
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
