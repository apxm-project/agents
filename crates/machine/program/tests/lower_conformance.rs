//! Golden lowering for generic typed structural containment.
//!
//! A typed graph verifies, lowers to canonical AIR whose semantic operations are
//! selected from the call intents and carry typed operands, and yields a
//! validating digest-bound artifact whose requirements come from the declared
//! graph requirements.

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
        "declarations": [
            {
                "decl_id": "decl.model.target.v1",
                "decl_kind": "model_binding",
                "input_type_ref": "ModelRequest",
                "output_type_ref": "ModelResponse",
                "target_ref": "model.target.v1"
            },
            {
                "decl_id": "decl.cap.search",
                "decl_kind": "capability_binding",
                "input_type_ref": "SearchArguments",
                "output_type_ref": "SearchResult",
                "target_ref": "cap.search"
            }
        ],
        "functions": [{
            "function_id": "run",
            "parameters": [
                {"value_id": "value.input", "type_ref": "Input", "role": "input"}
            ],
            "result_type_ref": "Output",
            "body_region_id": "region.root",
            "is_entrypoint": true
        }],
        "values": [
            {"value_id": "value.input", "type_ref": "Input", "origin": "parameter", "origin_id": "run"},
            {"value_id": "value.model.out", "type_ref": "ModelResponse", "origin": "call_result", "origin_id": "node.model"},
            {"value_id": "value.cap.out", "type_ref": "SearchResult", "origin": "call_result", "origin_id": "node.capability"}
        ],
        "blocks": [],
        "regions": [
            {"region_id": "region.root", "region_role": "function_body", "execution_order": 0},
            {
                "region_id": "loop.main",
                "region_role": "loop_body",
                "parent_region_id": "region.root",
                "execution_order": 0
            }
        ],
        "data_edges": [
            {"from_value": "value.input", "to_consumer": "node.model", "consumer_slot": "request"},
            {"from_value": "value.model.out", "to_consumer": "node.capability", "consumer_slot": "arguments"}
        ],
        "call_intents": [
            {
                "node_id": "node.model",
                "intent_kind": "model_invocation",
                "parent_region_id": "loop.main",
                "execution_order": 0,
                "binding_ref": "decl.model.target.v1",
                "operand_values": ["value.input"],
                "result_value": "value.model.out"
            },
            {
                "node_id": "node.capability",
                "intent_kind": "capability_invocation",
                "parent_region_id": "loop.main",
                "execution_order": 1,
                "binding_ref": "decl.cap.search",
                "operand_values": ["value.model.out"],
                "result_value": "value.cap.out"
            }
        ],
        "control_intents": [{
            "node_id": "node.loop",
            "control_kind": "loop",
            "parent_region_id": "region.root",
            "execution_order": 0,
            "body_region_ids": ["loop.main"]
        }],
        "context_flow": [{
            "from_node": "node.model",
            "to_node": "node.capability",
            "context_type_ref": "Context"
        }],
        "hook_bindings": [],
        "capability_requirements": [{"capability_ref": "cap.search"}],
        "model_requirements": [{"model_target_ref": "model.target.v1"}],
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
fn generic_graph_lowers_and_carries_source_map() {
    let graph = generic_graph();
    let air = frontend_graph_to_air(&graph).expect("lower graph");
    assert_eq!(air.source_map, graph.source_map);
    assert_eq!(air.context_flow.len(), graph.context_flow.len());

    // The typed model invocation intent selects model.call and the typed
    // capability invocation intent selects capability.invoke, each carrying
    // typed SSA operands whose slot names come from the AIS operand catalogue.
    let model = air
        .semantic_operations
        .iter()
        .find(|op| op.node_id == "node.model")
        .expect("model.call lowered");
    assert_eq!(model.op.wire(), "model.call");
    assert!(model.operands.iter().any(|o| o.slot == "request"));
    assert!(model.operands.iter().any(|operand| {
        operand.slot == "model_ref"
            && operand.type_ref == "ModelTargetRef"
            && operand.value_id == "model.target.v1"
    }));
    assert_eq!(
        model.result.as_ref().map(|r| r.value_id.as_str()),
        Some("value.model.out")
    );

    let cap = air
        .semantic_operations
        .iter()
        .find(|op| op.node_id == "node.capability")
        .expect("capability.invoke lowered");
    assert_eq!(cap.op.wire(), "capability.invoke");
    assert!(cap.operands.iter().any(|o| o.slot == "arguments"));
}

#[test]
fn frontend_graph_rejects_an_air_model_ref_before_lowering() {
    let mut value = generic_graph_value();
    value["model_requirements"][0]["model_ref"] = json!("model.target.v1");
    let error = serde_json::from_value::<FrontendGraph>(value)
        .expect_err("FrontendGraph has no AIR model_ref field");
    assert!(error.to_string().contains("model_ref"));
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
fn hook_wrapper_is_a_non_colliding_sibling_of_its_call_target() {
    let mut value = generic_graph_value();
    value["hook_bindings"] = json!([{
        "hook_id": "hook.before.model",
        "scope": "model",
        "phase": "before",
        "target_selector": "node.model",
        "declaration_order": 0,
        "handler_ref": "hooks.before_model",
        "handler_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "input_type_ref": "ModelContext",
        "output_type_ref": "ModelContext",
        "return_mode": "observe"
    }]);
    let graph: FrontendGraph = serde_json::from_value(value).expect("hook graph");
    let air = frontend_graph_to_air(&graph).expect("lower hook graph");
    let wrapper = air
        .structural_ir
        .iter()
        .find(|node| node.region_id == "hook.before.model")
        .expect("hook wrapper");
    let model = air
        .semantic_operations
        .iter()
        .find(|operation| operation.node_id == "node.model")
        .expect("model operation");
    assert_eq!(wrapper.parent_region_id.as_deref(), Some("loop.main"));
    assert!(wrapper.execution_order < model.execution_order);
    assert!(air.verify().is_accepted());
}

#[test]
fn yield_resume_value_is_owned_once_by_its_lexical_block() {
    let mut value = generic_graph_value();
    value["values"]
        .as_array_mut()
        .expect("values array")
        .push(json!({
            "value_id": "value.resume",
            "type_ref": "Input",
            "origin": "resume_input",
            "origin_id": "node.yield"
        }));
    value["blocks"] = json!([{
        "block_id": "block.loop",
        "region_id": "loop.main",
        "block_arguments": ["value.resume"],
        "execution_order": 0
    }]);
    value["control_intents"]
        .as_array_mut()
        .expect("control intent array")
        .push(json!({
            "node_id": "node.yield",
            "control_kind": "yield",
            "parent_region_id": "loop.main",
            "execution_order": 2,
            "result_value": "value.resume"
        }));

    let graph: FrontendGraph = serde_json::from_value(value).expect("yield graph");
    let air = frontend_graph_to_air(&graph).expect("yield graph lowers without duplicate SSA");
    let owners: Vec<&str> = air
        .structural_ir
        .iter()
        .filter(|node| {
            node.block_arguments
                .iter()
                .any(|argument| argument.value_id == "value.resume")
        })
        .map(|node| node.region_id.as_str())
        .collect();
    assert_eq!(owners, ["loop.main"]);
    assert!(air.verify().is_accepted());
}
