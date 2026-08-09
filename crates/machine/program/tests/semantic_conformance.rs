//! Conformance: the AIR, FrontendGraph, and source-map verifiers accept and
//! reject exactly the checked-in vectors, and the closed Rust enums do not drift
//! from the owning schemas.

mod common;

use apxm_program::air::{SemanticOpKind, StructuralOpKind};
use apxm_program::source_map::{RegionAnnotationKind, SourceLanguage};
use apxm_program::{verify_air_json, verify_frontend_graph_json, verify_source_map_json};
use common::{Vector, load_contract, load_vectors, schema_enum};
use serde_json::Value;
use serde_json::json;

fn check(file: &str, verify: impl Fn(&Value) -> bool) {
    for Vector {
        name,
        input,
        expected_valid,
    } in load_vectors(file)
    {
        let accepted = verify(&input);
        assert_eq!(
            accepted, expected_valid,
            "{file}: vector '{name}' expected valid={expected_valid} but verifier returned {accepted}",
        );
    }
}

#[test]
fn air_vectors_match_verifier() {
    check("apxm.air.v2.json", |v| verify_air_json(v).is_accepted());
}

#[test]
fn frontend_graph_vectors_match_verifier() {
    check("apxm.frontend-graph.v2.json", |v| {
        verify_frontend_graph_json(v).is_accepted()
    });
}

#[test]
fn retired_v1_vectors_are_rejected_without_migration() {
    check("apxm.air.v1-rejection.json", |v| {
        verify_air_json(v).is_accepted()
    });
    check("apxm.frontend-graph.v1-rejection.json", |v| {
        verify_frontend_graph_json(v).is_accepted()
    });
}

#[test]
fn tool_argument_must_be_defined_before_its_effect_site() {
    let mut input = load_vectors("apxm.frontend-graph.v2.json")
        .into_iter()
        .find(|vector| vector.name == "valid-frontend-graph-typed-intents")
        .expect("typed frontend graph vector")
        .input;
    let calls = input
        .get_mut("call_intents")
        .and_then(Value::as_array_mut)
        .expect("call intents array");
    for call in calls {
        if call.get("node_id") == Some(&json!("node.model.1")) {
            call["execution_order"] = json!(2);
        }
        if call.get("node_id") == Some(&json!("node.cap.1")) {
            call["execution_order"] = json!(1);
        }
    }
    assert!(!verify_frontend_graph_json(&input).is_accepted());
}

#[test]
fn every_authored_model_operand_requires_dominance() {
    let valid = load_vectors("apxm.frontend-graph.v2.json")
        .into_iter()
        .find(|vector| vector.name == "valid-frontend-graph-typed-intents")
        .expect("typed frontend graph vector")
        .input;
    assert!(verify_frontend_graph_json(&valid).is_accepted());

    let mut future_request = valid.clone();
    future_request["call_intents"][0]["operand_values"] = json!(["value.search.out"]);
    future_request["data_edges"][0]["from_value"] = json!("value.search.out");
    assert!(!verify_frontend_graph_json(&future_request).is_accepted());

    let mut sibling_request = valid.clone();
    sibling_request["regions"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "region_id": "region.sibling",
            "region_role": "task_scope",
            "parent_region_id": "region.body",
            "execution_order": 0
        }));
    sibling_request["call_intents"][0]["parent_region_id"] = json!("region.sibling");
    sibling_request["call_intents"][0]["operand_values"] = json!(["value.search.out"]);
    sibling_request["data_edges"][0]["from_value"] = json!("value.search.out");
    assert!(!verify_frontend_graph_json(&sibling_request).is_accepted());
}

#[test]
fn every_structural_operand_and_predicate_requires_dominance() {
    let valid = load_vectors("apxm.frontend-graph.v2.json")
        .into_iter()
        .find(|vector| vector.name == "valid-frontend-graph-typed-intents")
        .expect("typed frontend graph vector")
        .input;

    let mut future_structural_operand = valid.clone();
    future_structural_operand["regions"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "region_id": "region.sibling",
            "region_role": "task_scope",
            "parent_region_id": "region.body",
            "execution_order": 0
        }));
    future_structural_operand["regions"][1]["execution_order"] = json!(1);
    future_structural_operand["control_intents"][1]["parent_region_id"] = json!("region.sibling");
    future_structural_operand["control_intents"][1]["operand_values"] =
        json!(["value.event.output"]);
    future_structural_operand["data_edges"][3]["from_value"] = json!("value.event.output");
    assert!(!verify_frontend_graph_json(&future_structural_operand).is_accepted());

    let mut valid_predicate = valid.clone();
    valid_predicate["control_intents"][0]["predicate"] = json!({
        "root_value_id": "value.input",
        "property_path": ["state"],
        "comparator": "equals",
        "literal": {"scalar_type": "string", "value": "ready"}
    });
    assert!(verify_frontend_graph_json(&valid_predicate).is_accepted());

    let mut future_predicate = valid_predicate;
    future_predicate["control_intents"][0]["predicate"]["root_value_id"] =
        json!("value.event.output");
    assert!(!verify_frontend_graph_json(&future_predicate).is_accepted());
}

#[test]
fn every_authored_air_structural_use_requires_dominance() {
    let valid = load_vectors("apxm.air.v2.json")
        .into_iter()
        .find(|vector| vector.name == "valid-air-five-semantic-ops-and-structural-ir")
        .expect("typed AIR vector")
        .input;
    assert!(verify_air_json(&valid).is_accepted());

    let mut future_operand = valid.clone();
    future_operand["structural_ir"][1]["operands"][0]["value_id"] = json!("value.event.output");
    assert!(!verify_air_json(&future_operand).is_accepted());

    let mut future_predicate = valid;
    future_predicate["structural_ir"][1]["predicate"] = json!({
        "root_value_id": "value.event.output",
        "property_path": ["kind"],
        "comparator": "equals",
        "literal": {"scalar_type": "string", "value": "ready"}
    });
    assert!(!verify_air_json(&future_predicate).is_accepted());
}

#[test]
fn source_map_vectors_match_verifier() {
    check("apxm.source-map.v1.json", |v| {
        verify_source_map_json(v).is_accepted()
    });
}

/// Serialize each closed Rust variant and collect its wire string.
fn wire_members<T: serde::Serialize>(variants: &[T]) -> Vec<String> {
    let mut out: Vec<String> = variants
        .iter()
        .map(|v| {
            serde_json::to_value(v)
                .expect("serialize variant")
                .as_str()
                .expect("variant serializes to a string")
                .to_string()
        })
        .collect();
    out.sort();
    out
}

#[test]
fn air_semantic_op_enum_does_not_drift() {
    let schema = load_contract("schemas/apxm.air.v2.json");
    let expected = schema_enum(&schema, "SemanticOp", "op");
    let actual = wire_members(&[
        SemanticOpKind::ModelCall,
        SemanticOpKind::CapabilityInvoke,
        SemanticOpKind::ProgramNew,
        SemanticOpKind::ProgramInvoke,
        SemanticOpKind::AwaitEvent,
    ]);
    assert_eq!(
        actual, expected,
        "AIR semantic op closure drifted from schema"
    );
    assert_eq!(
        actual.len(),
        5,
        "AIR exposes exactly five semantic operations"
    );
}

#[test]
fn air_structural_kind_enum_does_not_drift() {
    let schema = load_contract("schemas/apxm.air.v2.json");
    let expected = schema_enum(&schema, "StructuralNode", "kind");
    let actual = wire_members(&[
        StructuralOpKind::Function,
        StructuralOpKind::Region,
        StructuralOpKind::Block,
        StructuralOpKind::Value,
        StructuralOpKind::Branch,
        StructuralOpKind::Switch,
        StructuralOpKind::Loop,
        StructuralOpKind::ParallelJoin,
        StructuralOpKind::Try,
        StructuralOpKind::Throw,
        StructuralOpKind::Catch,
        StructuralOpKind::Return,
        StructuralOpKind::Yield,
    ]);
    assert_eq!(
        actual, expected,
        "structural IR closure drifted from schema"
    );
    assert!(
        !actual.iter().any(|k| k == "nop"),
        "a NOP is never serialized structural IR",
    );
}

#[test]
fn source_map_enums_do_not_drift() {
    let schema = load_contract("schemas/apxm.source-map.v1.json");
    let annotations = schema_enum(&schema, "RegionAnnotation", "annotation");
    assert_eq!(
        wire_members(&[
            RegionAnnotationKind::None,
            RegionAnnotationKind::StructuralLoop
        ]),
        annotations,
        "region annotation closure drifted from schema",
    );

    let mut languages: Vec<String> =
        schema["$defs"]["SourceMapBody"]["properties"]["source_language"]["enum"]
            .as_array()
            .expect("source_language enum")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
    languages.sort();
    assert_eq!(
        wire_members(&[SourceLanguage::Python, SourceLanguage::Typescript]),
        languages,
        "source language closure drifted from schema",
    );
}

#[test]
fn verifier_is_deterministic() {
    let doc = load_vectors("apxm.air.v2.json")
        .into_iter()
        .find(|v| v.name == "valid-air-five-semantic-ops-and-structural-ir")
        .expect("named vector present");
    let first = verify_air_json(&doc.input).into_diagnostics();
    let second = verify_air_json(&doc.input).into_diagnostics();
    assert_eq!(first, second, "verifier output is not deterministic");
}

fn predicate_air() -> Value {
    json!({
        "schema_version": "apxm.air.v2",
        "semantic_operations": [{
            "node_id": "node.model",
            "op": "model.call",
            "parent_region_id": "region.fn",
            "execution_order": 0,
            "operands": [
                {"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"},
                {"slot": "request", "value_id": "value.request", "type_ref": "ModelRequest"}
            ],
            "result": {"value_id": "value.response", "type_ref": "ModelResponse"}
        }],
        "structural_ir": [
            {"region_id": "region.fn", "kind": "function", "execution_order": 0},
            {
                "region_id": "region.branch",
                "kind": "branch",
                "parent_region_id": "region.fn",
                "execution_order": 1,
                "predicate": {
                    "root_value_id": "value.response",
                    "property_path": ["kind"],
                    "comparator": "equals",
                    "literal": {"scalar_type": "string", "value": "final"}
                }
            },
            {"region_id": "region.then", "kind": "region", "parent_region_id": "region.branch", "execution_order": 0}
        ],
        "context_flow": [],
        "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
    })
}

#[test]
fn control_predicate_negative_vectors_fail_closed() {
    let valid = predicate_air();
    assert!(verify_air_json(&valid).is_accepted());

    let mut missing_root = valid.clone();
    missing_root["structural_ir"][1]["predicate"]["root_value_id"] = json!("value.missing");
    assert!(!verify_air_json(&missing_root).is_accepted());

    let mut empty_segment = valid.clone();
    empty_segment["structural_ir"][1]["predicate"]["property_path"] = json!([""]);
    assert!(!verify_air_json(&empty_segment).is_accepted());

    let mut hostile_segment = valid.clone();
    hostile_segment["structural_ir"][1]["predicate"]["property_path"] = json!(["../kind"]);
    assert!(!verify_air_json(&hostile_segment).is_accepted());

    let mut missing_literal = valid.clone();
    missing_literal["structural_ir"][1]["predicate"]
        .as_object_mut()
        .unwrap()
        .remove("literal");
    assert!(!verify_air_json(&missing_literal).is_accepted());

    let mut literal_mismatch = valid.clone();
    literal_mismatch["structural_ir"][1]["predicate"]["literal"] =
        json!({"scalar_type": "boolean", "value": "true"});
    assert!(!verify_air_json(&literal_mismatch).is_accepted());

    let mut unsafe_integer = valid.clone();
    unsafe_integer["structural_ir"][1]["predicate"]["literal"] =
        json!({"scalar_type": "integer", "value": 9_007_199_254_740_992_i64});
    assert!(!verify_air_json(&unsafe_integer).is_accepted());

    let mut unknown_field = valid;
    unknown_field["structural_ir"][1]["predicate"]["source_expression"] = json!("hidden");
    assert!(!verify_air_json(&unknown_field).is_accepted());
}

#[test]
fn loop_carried_signatures_reject_arity_slot_and_type_drift() {
    let mut valid = predicate_air();
    valid["structural_ir"][1]["kind"] = json!("ais.loop");
    valid["structural_ir"][1]["block_arguments"] =
        json!([{"value_id": "value.current", "type_ref": "ModelResponse"}]);
    valid["structural_ir"][1]["operands"] = json!([
        {"slot": "initial", "value_id": "value.response", "type_ref": "ModelResponse"},
        {"slot": "carried", "value_id": "value.response", "type_ref": "ModelResponse"}
    ]);
    valid["structural_ir"][1]["predicate"]["root_value_id"] = json!("value.current");
    valid["source_map"]["region_annotations"] =
        json!([{"region_id": "region.branch", "annotation": "structural_loop"}]);
    let verdict = verify_air_json(&valid);
    assert!(verdict.is_accepted(), "{:?}", verdict.into_diagnostics());

    let mut missing_carried = valid.clone();
    missing_carried["structural_ir"][1]["operands"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert!(!verify_air_json(&missing_carried).is_accepted());

    let mut wrong_slot = valid.clone();
    wrong_slot["structural_ir"][1]["operands"][1]["slot"] = json!("condition");
    assert!(!verify_air_json(&wrong_slot).is_accepted());

    let mut wrong_type = valid;
    wrong_type["structural_ir"][1]["operands"][1]["type_ref"] = json!("OtherResponse");
    assert!(!verify_air_json(&wrong_type).is_accepted());
}
