use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use apxm_core::constants::session;
use apxm_core::paths::session_node_dir_name;
use apxm_runtime::context_stack::{ContextScope, ContextStack, NodeMetadata};
use tempfile::tempdir;

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
            op_type: "ConstStr".to_string(),
        },
    );
    metadata.insert(
        2,
        NodeMetadata {
            name: "process".to_string(),
            op_type: "Communicate".to_string(),
        },
    );

    let stack = ContextStack::new(session_dir, Arc::new(metadata), Arc::new(vec![(1, 2)]));
    let assembly = stack.assemble(2, "claude", 10_000);

    assert!(assembly.frames.iter().any(|frame| {
        matches!(frame.scope, ContextScope::Upstream(1)) && frame.content.contains("upstream data")
    }));
    assert!(!assembly.truncated);
}
