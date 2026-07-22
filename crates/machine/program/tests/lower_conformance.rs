//! Golden lowering for generic typed structural containment.

use apxm_program::{ExecutableArtifact, FrontendGraph, frontend_graph_to_air};
use serde_json::{Value, json};

fn generic_graph_value() -> Value {
    json!({
        "schema_version": "apxm.frontend-graph.v1",
        "source_language": "python",
        "program_definitions": [{
            "program_id": "Worker",
            "entrypoint": "run",
            "input_type_ref": "Input",
            "output_type_ref": "Output",
            "has_default_context": true
        }],
        "imported_program_refs": [],
        "semantic_operations": [
            {
                "node_id": "node.model",
                "op": "model.call",
                "parent_region_id": "loop.main",
                "execution_order": 0,
                "operands": {"model_target_ref": "model.default"}
            },
            {
                "node_id": "node.capability",
                "op": "capability.invoke",
                "parent_region_id": "loop.main",
                "execution_order": 1,
                "operands": {"capability_ref": "cap.search"}
            }
        ],
        "structural_regions": [
            {"region_id": "region.root", "kind": "region", "execution_order": 0},
            {
                "region_id": "loop.main",
                "kind": "ais.loop",
                "parent_region_id": "region.root",
                "execution_order": 0
            },
            {
                "region_id": "return.main",
                "kind": "return",
                "parent_region_id": "region.root",
                "execution_order": 1
            }
        ],
        "context_flow": [{
            "from_node": "node.model",
            "to_node": "node.capability",
            "context_type_ref": "Context"
        }],
        "hook_bindings": [],
        "capability_requirements": [{"capability_ref": "cap.search"}],
        "model_requirements": [{"model_target_ref": "model.default"}],
        "source_map": {
            "schema_version": "apxm.source-map.v1",
            "source_language": "python",
            "node_spans": [],
            "region_annotations": [
                {"region_id": "loop.main", "annotation": "structural_loop"}
            ]
        }
    })
}

fn generic_graph() -> FrontendGraph {
    serde_json::from_value(generic_graph_value()).expect("generic graph")
}

#[test]
fn generic_graph_lowers_without_synthetic_structure() {
    let graph = generic_graph();
    let air = frontend_graph_to_air(&graph).expect("lower graph");
    assert_eq!(air.structural_ir.len(), graph.structural_regions.len());
    assert_eq!(air.semantic_operations.len(), 2);
    assert_eq!(air.semantic_operations[0].parent_region_id, "loop.main");
    assert_eq!(air.semantic_operations[1].execution_order, 1);
    assert_eq!(
        serde_json::to_value(&air).unwrap()["structural_ir"][1],
        json!({
            "region_id": "loop.main",
            "kind": "ais.loop",
            "parent_region_id": "region.root",
            "execution_order": 0
        })
    );
}

#[test]
fn generic_graph_produces_validating_artifact() {
    let graph = generic_graph();
    let artifact = ExecutableArtifact::from_frontend_graph(&graph).expect("artifact");
    assert!(artifact.validate().is_accepted());
    assert_eq!(artifact.entrypoints[0].program_id, "Worker");
    assert_eq!(artifact.artifact_semantic_requirements.len(), 2);
}

#[test]
fn rejects_orphan_context_flow_edge() {
    let mut value = generic_graph_value();
    value["context_flow"] = json!([{
        "from_node": "node.missing",
        "to_node": "node.capability",
        "context_type_ref": "Context"
    }]);
    let graph: FrontendGraph = serde_json::from_value(value).expect("typed graph");
    let err = frontend_graph_to_air(&graph).expect_err("orphan edge");
    assert!(
        err.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.location == "node.missing")
    );
}
