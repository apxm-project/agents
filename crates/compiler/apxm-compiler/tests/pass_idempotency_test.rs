//! Phase A — Task 9
//! Compiling the same graph twice with identical config must be deterministic.
//!
//! Surfaces non-determinism (HashMap iteration order, time-based seeds, PIDs,
//! address-based hashing) by comparing the IR text of two back-to-back compiles
//! of the same input. Phase A is harness-only: failures are documented, not
//! fixed.

use std::collections::HashMap;

use apxm_compiler::{AirEdge, AirModule, AirNode, Context, Pipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, DependencyType, OptimizationLevel, Value};

/// Build the attribute map for an Ask node that consumes a single named input.
/// `tag` keeps each ask node textually distinct so CSE cannot merge them.
fn ask_attrs_one(tag: &str, input_name: &str) -> HashMap<String, Value> {
    HashMap::from([
        (
            graph_attrs::TEMPLATE_STR.into(),
            Value::String(format!("{{{input_name}}}: branch {tag}")),
        ),
        (
            graph_attrs::INPUT_NAMES.into(),
            Value::Array(vec![Value::String(input_name.into())]),
        ),
    ])
}

/// Shared-prefix fan-out (no synthesis):
///   preface → ask_a, ask_b, ask_c
/// Each ask consumes only `{preface}` so the multi-input path is not
/// exercised. Three text-distinct asks defeat CSE merging and stress
/// canonicalization independently — the passes most likely to leak HashMap
/// iteration order if non-deterministic.
fn build_shared_prefix_fanout() -> AirModule {
    AirModule {
        name: "shared_prefix_fanout".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "preface".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([("value".into(), Value::String("preface".into()))]),
            },
            AirNode {
                id: 2,
                name: "ask_a".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("a", "preface"),
            },
            AirNode {
                id: 3,
                name: "ask_b".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("b", "preface"),
            },
            AirNode {
                id: 4,
                name: "ask_c".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("c", "preface"),
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
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    }
}

#[test]
fn compile_is_deterministic_at_o2() {
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O2);

    let graph = build_shared_prefix_fanout();

    let m1 = pipeline.compile_graph(&graph).expect("compile #1");
    let m2 = pipeline.compile_graph(&graph).expect("compile #2");

    let ir1 = m1.to_string().expect("ir text #1");
    let ir2 = m2.to_string().expect("ir text #2");

    if ir1 != ir2 {
        let lines1: Vec<&str> = ir1.lines().collect();
        let lines2: Vec<&str> = ir2.lines().collect();
        let mut diffs = Vec::new();
        let max_len = lines1.len().max(lines2.len());
        for i in 0..max_len {
            let a = lines1.get(i).copied().unwrap_or("<missing>");
            let b = lines2.get(i).copied().unwrap_or("<missing>");
            if a != b {
                diffs.push(format!("line {}: ir1=`{}` ir2=`{}`", i + 1, a, b));
                if diffs.len() >= 5 {
                    break;
                }
            }
        }
        panic!(
            "compiler output is non-deterministic across two compiles of the same input - \
             find the source of variation (HashMap iteration? time? PID? address-based hash?). \
             First diffs:\n{}",
            diffs.join("\n")
        );
    }
}
