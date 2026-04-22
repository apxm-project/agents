//! Integration test: per-node tokens flow through SessionEventEmitter into live.json
//!
//! Verifies that emit_operation_end accepts an Option<TokenUsageSummary> and that
//! the resulting CompletedNodeInfo entry in live.json carries input_tokens and
//! output_tokens populated from the passed usage.

use std::collections::HashMap;
use std::fs;
use std::time::Duration;

use apxm_compiler::{AirEdge, AirModule, AirNode};
use apxm_core::constants;
use apxm_core::types::session::{LiveSessionState, SessionStatus};
use apxm_core::types::AISOperationType;
use apxm_driver::session_output::SessionEventEmitter;
use apxm_runtime::{ExecutionEventEmitter, TokenUsageSummary};

fn single_ask_graph() -> AirModule {
    AirModule {
        name: "per-node-tokens-test".to_string(),
        nodes: vec![AirNode {
            id: 42,
            name: "ask_one".to_string(),
            op: AISOperationType::Ask,
            attributes: HashMap::new(),
        }],
        edges: Vec::<AirEdge>::new(),
        parameters: Vec::new(),
        metadata: HashMap::new(),
    }
}

#[test]
fn per_node_tokens_appear_in_live_json() {
    let session_root = tempfile::tempdir().expect("session root");
    let graph = single_ask_graph();

    let emitter = SessionEventEmitter::new(
        session_root.path(),
        "exec-per-node-tokens".to_string(),
        Some(&graph),
        None,
    )
    .expect("session emitter");

    emitter.set_total_nodes(1);
    emitter.emit_operation_start(42, "ASK");
    emitter.emit_operation_end(
        42,
        "ASK",
        Duration::from_millis(5),
        true,
        Some(TokenUsageSummary {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            call_count: 1,
        }),
        None,
    );

    let live_path = session_root
        .path()
        .join(constants::session::files::LIVE);
    let live_data = fs::read_to_string(&live_path).expect("read live.json");
    let live: LiveSessionState =
        serde_json::from_str(&live_data).expect("parse LiveSessionState");

    assert_eq!(
        live.completed_nodes.len(),
        1,
        "expected exactly one completed node entry"
    );
    let entry = &live.completed_nodes[0];
    assert_eq!(entry.id, 42);
    assert_eq!(entry.status, SessionStatus::Completed);
    assert_eq!(entry.input_tokens, Some(10));
    assert_eq!(entry.output_tokens, Some(20));
}
