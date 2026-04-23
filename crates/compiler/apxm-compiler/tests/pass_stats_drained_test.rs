//! Tier-2 helper: confirms that a pass which writes its `_fired_count`
//! IntegerAttr onto the module gets that value drained into `PassMetrics`
//! and erased from the module.
//!
//! `FuseAskOps` will write `fuse-ask-ops_fired_count` once Task 7 wires
//! `_fired_count` + `_ir_size_delta` for every transform pass. Until then
//! this test is `#[ignore]`d — the FFI plumbing it exercises is already
//! covered by Task 2's CAPI and Task 3's `drain_pass_stats` helper, but
//! the end-to-end signal (FuseAskOps actually firing) only appears after
//! Task 7.
//!
//! The fan-in fixture mirrors `build_synth_fanin_module` from
//! `crates/orchestration/apxm-driver/tests/semantic_equivalence_test.rs`
//! (Phase A semantic-equivalence harness): `FuseAskOps` is known to fold
//! the synth Ask with its three upstream Asks at O1.

use std::collections::HashMap;

use apxm_compiler::{AirEdge, AirModule, AirNode, Context, Pipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, DependencyType, OptimizationLevel, Value};

fn ask_attrs_one(tag: &str, input_name: &str) -> HashMap<String, Value> {
    HashMap::from([
        (
            graph_attrs::TEMPLATE_STR.into(),
            Value::String(format!("[{tag}] answer: {{{input_name}}}")),
        ),
        (
            graph_attrs::INPUT_NAMES.into(),
            Value::Array(vec![Value::String(input_name.into())]),
        ),
    ])
}

fn ask_attrs_many(template: &str, names: &[&str]) -> HashMap<String, Value> {
    HashMap::from([
        (
            graph_attrs::TEMPLATE_STR.into(),
            Value::String(template.into()),
        ),
        (
            graph_attrs::INPUT_NAMES.into(),
            Value::Array(names.iter().map(|s| Value::String((*s).into())).collect()),
        ),
    ])
}

/// question -> ask_a, ask_b, ask_c -> synth (consumes {ask_a}/{ask_b}/{ask_c}).
fn build_synth_fanin_module() -> AirModule {
    AirModule {
        name: "synth_fanin".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "question".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([("value".into(), Value::String("what is 42?".into()))]),
            },
            AirNode {
                id: 2,
                name: "ask_a".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("a", "question"),
            },
            AirNode {
                id: 3,
                name: "ask_b".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("b", "question"),
            },
            AirNode {
                id: 4,
                name: "ask_c".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("c", "question"),
            },
            AirNode {
                id: 5,
                name: "synth".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_many(
                    "combine: {ask_a} + {ask_b} + {ask_c}",
                    &["ask_a", "ask_b", "ask_c"],
                ),
            },
        ],
        edges: vec![
            AirEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 1,
                to: 3,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 1,
                to: 4,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 2,
                to: 5,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 3,
                to: 5,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 4,
                to: 5,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    }
}

#[test]
fn fuse_ask_ops_reports_nonzero_fired_count() {
    let context = Context::new().expect("MLIR context must initialize");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);
    let module = build_synth_fanin_module();
    let (_compiled, diagnostics) = pipeline
        .compile_graph_with_diagnostics(&module)
        .expect("O1 compile must succeed");

    let fuse = diagnostics
        .passes
        .iter()
        .find(|p| p.pass_name == "fuse-ask-ops")
        .expect("fuse-ask-ops must run at O1");

    assert!(
        fuse.fired_count > 0,
        "fuse-ask-ops should fire on synth fan-in graph; got {fuse:?}",
    );
}
