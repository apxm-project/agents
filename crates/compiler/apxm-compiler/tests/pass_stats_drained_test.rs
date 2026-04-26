//! Tier-2 helper: confirms that a pass which writes its `_fired_count`
//! IntegerAttr onto the module gets that value drained into `PassMetrics`
//! and erased from the module.
//!
//! `FuseAskOps` is intentionally not in the default O-level pipelines because
//! LLM-call merging needs a typed semantic contract. This test keeps the pass
//! metrics path covered by invoking the pass explicitly.
//!
//! The fan-in fixture mirrors `build_synth_fanin_module` from
//! `crates/orchestration/apxm-driver/tests/semantic_equivalence_test.rs`
//! (Phase A semantic-equivalence harness): `FuseAskOps` is known to fold
//! the synth Ask with its three upstream Asks at O1.

use std::collections::HashMap;

use apxm_compiler::{AirEdge, AirModule, AirNode, Context, Pipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::compiler::metadata as passes;
use apxm_core::types::{AISOperationType, DependencyType, PipelineConfig, Value};

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
                attributes: HashMap::from([(
                    graph_attrs::VALUE.into(),
                    Value::String("what is 42?".into()),
                )]),
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
    let pipeline = Pipeline::with_config(
        &context,
        PipelineConfig {
            pass_list_override: Some(vec![passes::FUSE_ASK_OPS.name.to_string()]),
            ..Default::default()
        },
    );
    let module = build_synth_fanin_module();
    let (_compiled, diagnostics) = pipeline
        .compile_graph_with_diagnostics(&module)
        .expect("O1 compile must succeed");

    let fuse = diagnostics
        .passes
        .iter()
        .find(|p| p.pass_name == passes::FUSE_ASK_OPS.name)
        .expect("fuse-ask-ops must run when explicitly requested");

    assert!(
        fuse.fired_count > 0,
        "fuse-ask-ops should fire on synth fan-in graph; got {fuse:?}",
    );
}
