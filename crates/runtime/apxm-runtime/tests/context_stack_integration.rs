use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use apxm_core::constants::session;
use apxm_core::paths::session_node_dir_name;
use apxm_core::types::operations::AISOperationType;
use apxm_runtime::context_stack::{ContextScope, ContextStack, NodeMetadata};
use tempfile::tempdir;

const MOCK_AGENT_PROFILE: &str = "mock-profile";

#[test]
fn context_stack_reads_real_session_output() {
    let dir = tempdir().expect("tempdir");
    let session_dir = dir.path().join("session");
    let nodes_dir = session_dir.join(session::files::NODES_DIR);
    fs::create_dir_all(&nodes_dir).expect("nodes dir");

    let node1_dir = nodes_dir.join(session_node_dir_name(1, "seed"));
    fs::create_dir_all(&node1_dir).expect("node1 dir");
    fs::write(
        node1_dir.join(session::node::OUTPUT_JSON),
        r#"{"result": "upstream data"}"#,
    )
    .expect("output");

    let mut metadata = HashMap::new();
    metadata.insert(
        1,
        NodeMetadata {
            name: "seed".to_string(),
            op_type: AISOperationType::ConstStr,
        },
    );
    metadata.insert(
        2,
        NodeMetadata {
            name: "process".to_string(),
            op_type: AISOperationType::Communicate,
        },
    );

    let stack = ContextStack::new(session_dir, Arc::new(metadata), Arc::new(vec![(1, 2)]));
    let assembly = stack.assemble(2, MOCK_AGENT_PROFILE, 10_000);

    assert!(assembly.frames.iter().any(|frame| {
        matches!(frame.scope, ContextScope::Upstream(1)) && frame.content.contains("upstream data")
    }));
    assert!(!assembly.truncated);
}

#[test]
fn context_stack_full_demand_paging() {
    let dir = tempdir().expect("tempdir");
    let session_dir = dir.path().join("session");
    let nodes_dir = session_dir.join(session::files::NODES_DIR);
    fs::create_dir_all(&nodes_dir).expect("nodes dir");

    // Create graph summary
    fs::write(
        session_dir.join("graph_summary.json"),
        r#"{"name": "test-workflow", "node_count": 3, "edge_count": 2}"#,
    )
    .expect("graph summary");

    // Create upstream node 1 with output, prompt, and status
    let node1_dir = nodes_dir.join(session_node_dir_name(1, "planner"));
    fs::create_dir_all(&node1_dir).expect("node1 dir");
    fs::write(
        node1_dir.join(session::node::OUTPUT_JSON),
        r#"{"plan": "Step 1: analyze\nStep 2: implement"}"#,
    )
    .expect("output");
    fs::write(
        node1_dir.join(session::node::PROMPT_TXT),
        "Create a plan for the feature",
    )
    .expect("prompt");
    fs::write(
        node1_dir.join(session::node::STATUS_JSON),
        r#"{"status": "completed", "duration_ms": 1500}"#,
    )
    .expect("status");

    // Create upstream node 2 with only output
    let node2_dir = nodes_dir.join(session_node_dir_name(2, "coder"));
    fs::create_dir_all(&node2_dir).expect("node2 dir");
    fs::write(
        node2_dir.join(session::node::OUTPUT_JSON),
        r#"{"code": "fn main() { println!(\"Hello\"); }"}"#,
    )
    .expect("output");

    let mut metadata = HashMap::new();
    metadata.insert(
        1,
        NodeMetadata {
            name: "planner".to_string(),
            op_type: AISOperationType::SpawnAgent,
        },
    );
    metadata.insert(
        2,
        NodeMetadata {
            name: "coder".to_string(),
            op_type: AISOperationType::SpawnAgent,
        },
    );
    metadata.insert(
        3,
        NodeMetadata {
            name: "reviewer".to_string(),
            op_type: AISOperationType::SpawnAgent,
        },
    );

    let stack = ContextStack::new(
        session_dir.clone(),
        Arc::new(metadata),
        Arc::new(vec![(1, 2), (2, 3)]),
    );

    // Assemble context for node 3 (downstream of 1 and 2)
    // Use "reviewer" profile which includes upstream prompts
    let assembly = stack.assemble(3, "reviewer", 10_000);

    // Should have session frame with graph summary
    let session_frame = assembly
        .frames
        .iter()
        .find(|f| matches!(f.scope, ContextScope::Session));
    assert!(session_frame.is_some(), "No session frame found");
    let session_content = &session_frame.unwrap().content;
    eprintln!("Session content: {}", session_content);
    assert!(
        session_content.contains("test-workflow"),
        "Missing test-workflow in: {}",
        session_content
    );
    assert!(
        session_content.contains("Nodes: 3"),
        "Missing node count in: {}",
        session_content
    );
    assert!(
        session_content.contains("Edges: 2"),
        "Missing edge count in: {}",
        session_content
    );

    // Should have upstream frame for node 2
    assert!(assembly.frames.iter().any(|frame| {
        matches!(frame.scope, ContextScope::Upstream(2)) && frame.content.contains("fn main()")
    }));

    // Should have upstream frame for node 1 (including prompt)
    assert!(assembly.frames.iter().any(|frame| {
        matches!(frame.scope, ContextScope::Upstream(1))
            && frame.content.contains("Step 1")
            && frame.content.contains("Create a plan")
    }));

    // Should have local frame for node 3
    assert!(
        assembly
            .frames
            .iter()
            .any(|frame| matches!(frame.scope, ContextScope::Local))
    );

    assert!(!assembly.truncated);
}
