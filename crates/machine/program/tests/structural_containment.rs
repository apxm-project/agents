use apxm_program::{FrontendGraph, frontend_graph_to_air, verify_frontend_graph_json};
use serde_json::{Value, json};

fn nested_sibling_graph() -> Value {
    json!({
        "schema_version": "apxm.frontend-graph",
        "source_language": "python",
        "program_definitions": [{
            "program_id": "NestedLoops",
            "entrypoint": "run",
            "input_type_ref": "Input",
            "output_type_ref": "Output",
            "has_default_context": true
        }],
        "imported_program_refs": [],
        "declarations": [],
        "functions": [{
            "function_id": "run",
            "parameters": [],
            "body_region_id": "region.root",
            "is_entrypoint": true
        }],
        "values": [],
        "blocks": [],
        "regions": [
            {"region_id": "region.root", "region_role": "function_body", "execution_order": 0},
            {
                "region_id": "loop.outer",
                "region_role": "loop_body",
                "parent_region_id": "region.root",
                "execution_order": 0
            },
            {
                "region_id": "loop.inner",
                "region_role": "loop_body",
                "parent_region_id": "loop.outer",
                "execution_order": 1
            },
            {
                "region_id": "loop.sibling",
                "region_role": "loop_body",
                "parent_region_id": "region.root",
                "execution_order": 1
            }
        ],
        "data_edges": [],
        "call_intents": [],
        "control_intents": [],
        "context_flow": [],
        "hook_bindings": [],
        "capability_requirements": [],
        "model_requirements": [],
        "source_map": {
            "schema_version": "apxm.source-map",
            "source_language": "python",
            "node_spans": [],
            "region_annotations": []
        }
    })
}

#[test]
fn typed_nested_and_sibling_containment_verifies_and_lowers() {
    let value = nested_sibling_graph();
    assert!(verify_frontend_graph_json(&value).is_accepted());
    let graph: FrontendGraph = serde_json::from_value(value).expect("typed graph");
    // Lowering reconstructs typed CFG/SSA AIR from these intents and regions and
    // carries the source map through unchanged.
    let air = frontend_graph_to_air(&graph).expect("lower graph");
    assert_eq!(air.source_map, graph.source_map);
}

#[test]
fn verifier_rejects_missing_parent_cycles_and_duplicate_sibling_order() {
    let mut missing_parent = nested_sibling_graph();
    missing_parent["regions"][1]["parent_region_id"] = json!("loop.missing");
    assert!(!verify_frontend_graph_json(&missing_parent).is_accepted());

    let mut cycle = nested_sibling_graph();
    cycle["regions"][0]["parent_region_id"] = json!("loop.inner");
    assert!(!verify_frontend_graph_json(&cycle).is_accepted());

    let mut duplicate_order = nested_sibling_graph();
    duplicate_order["regions"][3]["execution_order"] = json!(0);
    assert!(!verify_frontend_graph_json(&duplicate_order).is_accepted());
}
