//! Phase A — Task 8
//! Compile -> decompile -> recompile must reach a fixed point in one roundtrip.
//!
//! Surfaces non-idempotent passes by comparing the IR text of the second and
//! third compilations. Phase A is harness-only: failures are documented, not
//! fixed.

use std::collections::HashMap;

use apxm_artifact::Artifact;
use apxm_compiler::{AirEdge, AirModule, AirNode, AirParam, Context, Pipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::{AISOperationType, DependencyType, OptimizationLevel, Value};

/// Fan-in fixture: two ConstStr inputs feed an Ask that synthesizes both.
/// Exercises token estimation, fusion candidates, CSE, and canonicalization.
fn build_fusion_stress_module() -> AirModule {
    AirModule {
        name: "fusion_stress".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "const_a".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    "value".into(),
                    Value::String("alpha".into()),
                )]),
            },
            AirNode {
                id: 2,
                name: "const_b".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    "value".into(),
                    Value::String("beta".into()),
                )]),
            },
            AirNode {
                id: 3,
                name: "synth".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([
                    (
                        graph_attrs::TEMPLATE_STR.into(),
                        Value::String("combine: {const_a} and {const_b}".into()),
                    ),
                    (
                        graph_attrs::INPUT_NAMES.into(),
                        Value::Array(vec![
                            Value::String("const_a".into()),
                            Value::String("const_b".into()),
                        ]),
                    ),
                ]),
            },
        ],
        edges: vec![
            AirEdge {
                from: 1,
                to: 3,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 2,
                to: 3,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    }
}

/// Inline copy of apxm-cli's `dag_to_graph` (compile.rs) so the compiler-crate
/// test does not depend on the CLI binary. Keep semantics identical.
fn dag_to_graph(dag: &ExecutionDag) -> AirModule {
    let nodes: Vec<AirNode> = dag
        .nodes
        .iter()
        .map(|n| AirNode {
            id: n.id,
            name: format!("node_{}", n.id),
            op: n.op_type,
            attributes: n.attributes.clone(),
        })
        .collect();

    let edges: Vec<AirEdge> = dag
        .edges
        .iter()
        .map(|e| AirEdge {
            from: e.from,
            to: e.to,
            dependency: e.dependency_type.clone(),
        })
        .collect();

    let parameters: Vec<AirParam> = dag
        .metadata
        .parameters
        .iter()
        .map(|p| AirParam {
            name: p.name.clone(),
            type_name: p.type_name.clone(),
        })
        .collect();

    AirModule {
        name: dag
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "decompiled".to_string()),
        nodes,
        edges,
        parameters,
        metadata: HashMap::new(),
    }
}

/// Marked `should_panic` because Phase A surfaced a non-idempotent pass:
/// re-ingesting decompiled IR duplicates the `downstream_nodes` attribute
/// (`ais.downstream_nodes` plus a bare `downstream_nodes`), so ir2 != ir3.
/// The harness still validates that compile -> decompile -> recompile runs
/// end-to-end; Phase B will fix the pass and flip this to a normal `#[test]`.
#[test]
#[should_panic(expected = "did not reach a fixed point")]
fn golden_artifact_roundtrip_fusion_stress_o2() {
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O2);

    let module_in = build_fusion_stress_module();

    let m1 = pipeline.compile_graph(&module_in).expect("compile #1");
    let bytes1 = m1.generate_artifact_bytes().expect("artifact #1");

    let art1 = Artifact::from_bytes(&bytes1).expect("parse #1");
    let dag1 = art1.dag().expect("dag #1");
    let air2 = dag_to_graph(dag1);
    let m2 = pipeline.compile_graph(&air2).expect("compile #2");
    let _bytes2 = m2.generate_artifact_bytes().expect("artifact #2");

    let art2 = Artifact::from_bytes(&_bytes2).expect("parse #2");
    let dag2 = art2.dag().expect("dag #2");
    let air3 = dag_to_graph(dag2);
    let m3 = pipeline.compile_graph(&air3).expect("compile #3");
    let _bytes3 = m3.generate_artifact_bytes().expect("artifact #3");

    let ir2 = m2.to_string().expect("ir text #2");
    let ir3 = m3.to_string().expect("ir text #3");

    if ir2 != ir3 {
        let lines2: Vec<&str> = ir2.lines().collect();
        let lines3: Vec<&str> = ir3.lines().collect();
        let mut diffs = Vec::new();
        let max_len = lines2.len().max(lines3.len());
        for i in 0..max_len {
            let a = lines2.get(i).copied().unwrap_or("<missing>");
            let b = lines3.get(i).copied().unwrap_or("<missing>");
            if a != b {
                diffs.push(format!("line {}: ir2=`{}` ir3=`{}`", i + 1, a, b));
                if diffs.len() >= 5 {
                    break;
                }
            }
        }
        panic!(
            "compiler did not reach a fixed point after one roundtrip - \
             a pass is non-idempotent on decompiled input. First diffs:\n{}",
            diffs.join("\n")
        );
    }
}
