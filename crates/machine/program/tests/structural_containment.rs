use apxm_program::{FrontendGraph, frontend_graph_to_air, verify_frontend_graph_json};
use serde_json::{Value, json};

fn nested_sibling_graph() -> Value {
    json!({
        "schema_version": "apxm.frontend-graph.v1",
        "source_language": "python",
        "program_definitions": [{
            "program_id": "NestedLoops",
            "entrypoint": "run",
            "input_type_ref": "Input",
            "output_type_ref": "Output",
            "has_default_context": true
        }],
        "imported_program_refs": [],
        "semantic_operations": [
            {
                "node_id": "node.outer.before",
                "op": "model.call",
                "parent_region_id": "loop.outer",
                "execution_order": 0
            },
            {
                "node_id": "node.inner",
                "op": "capability.invoke",
                "parent_region_id": "loop.inner",
                "execution_order": 0
            },
            {
                "node_id": "node.outer.after",
                "op": "model.call",
                "parent_region_id": "loop.outer",
                "execution_order": 2
            },
            {
                "node_id": "node.sibling",
                "op": "await.event",
                "parent_region_id": "loop.sibling",
                "execution_order": 0
            }
        ],
        "structural_regions": [
            {
                "region_id": "region.root",
                "kind": "region",
                "execution_order": 0
            },
            {
                "region_id": "loop.outer",
                "kind": "ais.loop",
                "parent_region_id": "region.root",
                "execution_order": 0
            },
            {
                "region_id": "loop.inner",
                "kind": "ais.loop",
                "parent_region_id": "loop.outer",
                "execution_order": 1
            },
            {
                "region_id": "loop.sibling",
                "kind": "ais.loop",
                "parent_region_id": "region.root",
                "execution_order": 1
            }
        ],
        "context_flow": [],
        "hook_bindings": [],
        "capability_requirements": [],
        "model_requirements": [],
        "source_map": {
            "schema_version": "apxm.source-map.v1",
            "source_language": "python",
            "node_spans": [],
            "region_annotations": [
                {"region_id": "loop.outer", "annotation": "structural_loop"},
                {"region_id": "loop.inner", "annotation": "structural_loop"},
                {"region_id": "loop.sibling", "annotation": "structural_loop"}
            ]
        }
    })
}

#[test]
fn lowering_preserves_nested_and_sibling_loop_containment() {
    let value = nested_sibling_graph();
    assert!(verify_frontend_graph_json(&value).is_accepted());
    let graph: FrontendGraph = serde_json::from_value(value).expect("typed graph");
    let air = frontend_graph_to_air(&graph).expect("lower graph");

    let outer_before = air
        .semantic_operations
        .iter()
        .find(|operation| operation.node_id == "node.outer.before")
        .unwrap();
    let inner = air
        .semantic_operations
        .iter()
        .find(|operation| operation.node_id == "node.inner")
        .unwrap();
    let sibling = air
        .semantic_operations
        .iter()
        .find(|operation| operation.node_id == "node.sibling")
        .unwrap();
    assert_eq!(outer_before.parent_region_id, "loop.outer");
    assert_eq!(outer_before.execution_order, 0);
    assert_eq!(inner.parent_region_id, "loop.inner");
    assert_eq!(sibling.parent_region_id, "loop.sibling");

    let inner_loop = air
        .structural_ir
        .iter()
        .find(|region| region.region_id == "loop.inner")
        .unwrap();
    assert_eq!(inner_loop.parent_region_id.as_deref(), Some("loop.outer"));
    assert_eq!(inner_loop.execution_order, 1);
}

#[test]
fn verifier_rejects_missing_parent_cycles_and_duplicate_sibling_order() {
    let mut missing_parent = nested_sibling_graph();
    missing_parent["semantic_operations"][0]["parent_region_id"] = json!("loop.missing");
    assert!(!verify_frontend_graph_json(&missing_parent).is_accepted());

    let mut cycle = nested_sibling_graph();
    cycle["structural_regions"][0]["parent_region_id"] = json!("loop.inner");
    assert!(!verify_frontend_graph_json(&cycle).is_accepted());

    let mut duplicate_order = nested_sibling_graph();
    duplicate_order["structural_regions"][3]["execution_order"] = json!(0);
    assert!(!verify_frontend_graph_json(&duplicate_order).is_accepted());

    let mut missing_loop_identity = nested_sibling_graph();
    missing_loop_identity["source_map"]["region_annotations"] = json!([]);
    assert!(!verify_frontend_graph_json(&missing_loop_identity).is_accepted());
}
