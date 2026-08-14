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
    check("apxm.air.json", |v| verify_air_json(v).is_accepted());
}

#[test]
fn frontend_graph_vectors_match_verifier() {
    check("apxm.frontend-graph.json", |v| {
        verify_frontend_graph_json(v).is_accepted()
    });
}

#[test]
fn tool_argument_must_be_defined_before_its_effect_site() {
    let mut input = load_vectors("apxm.frontend-graph.json")
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
fn every_executable_invocation_operand_must_be_dominated() {
    let base = load_vectors("apxm.frontend-graph.json")
        .into_iter()
        .find(|vector| vector.name == "valid-frontend-graph-typed-intents")
        .expect("typed frontend graph vector")
        .input;

    let mut future_model = base.clone();
    future_model["call_intents"]
        .as_array_mut()
        .expect("call intents")
        .iter_mut()
        .for_each(|call| {
            if call["node_id"] == json!("node.model.1") {
                call["operand_values"] = json!(["value.search.out"]);
            }
        });
    future_model["data_edges"][0]["from_value"] = json!("value.search.out");
    assert!(!verify_frontend_graph_json(&future_model).is_accepted());

    let mut missing_data_edges = base.clone();
    missing_data_edges
        .as_object_mut()
        .expect("frontend graph object")
        .remove("data_edges");
    missing_data_edges["call_intents"]
        .as_array_mut()
        .expect("call intents")
        .iter_mut()
        .for_each(|call| {
            if call["node_id"] == json!("node.model.1") {
                call["operand_values"] = json!(["value.search.out"]);
            }
        });
    assert!(!verify_frontend_graph_json(&missing_data_edges).is_accepted());

    let mut sibling = base.clone();
    sibling["regions"].as_array_mut().expect("regions").extend([
        json!({
            "region_id": "region.sibling.a",
            "region_role": "conditional_arm",
            "parent_region_id": "region.body",
            "execution_order": 0
        }),
        json!({
            "region_id": "region.sibling.b",
            "region_role": "conditional_arm",
            "parent_region_id": "region.body",
            "execution_order": 1
        }),
    ]);
    sibling["call_intents"]
        .as_array_mut()
        .expect("call intents")
        .iter_mut()
        .for_each(|call| match call["node_id"].as_str() {
            Some("node.model.1") => {
                call["parent_region_id"] = json!("region.sibling.a");
                call["execution_order"] = json!(3);
            }
            Some("node.cap.1") => {
                call["parent_region_id"] = json!("region.sibling.b");
                call["execution_order"] = json!(4);
            }
            _ => {}
        });
    assert!(!verify_frontend_graph_json(&sibling).is_accepted());

    let mut completed_nested = base;
    completed_nested["call_intents"]
        .as_array_mut()
        .expect("call intents")
        .iter_mut()
        .for_each(|call| {
            if call["node_id"] == json!("node.cap.1") {
                call["parent_region_id"] = json!("region.body");
                call["execution_order"] = json!(3);
            }
        });
    let nested_verdict = verify_frontend_graph_json(&completed_nested);
    assert!(
        nested_verdict.is_accepted(),
        "{:?}",
        nested_verdict.into_diagnostics()
    );
}

#[test]
fn air_invocation_operands_cover_model_and_nested_completion() {
    let mut future = load_vectors("apxm.air.json")
        .into_iter()
        .find(|vector| vector.name == "valid-air-five-semantic-ops-and-structural-ir")
        .expect("valid AIR vector")
        .input;
    future["semantic_operations"][0]["operands"][1]["value_id"] = json!("value.cap.out");
    assert!(!verify_air_json(&future).is_accepted());

    let mut future_options = load_vectors("apxm.air.json")
        .into_iter()
        .find(|vector| vector.name == "valid-air-five-semantic-ops-and-structural-ir")
        .expect("valid AIR vector")
        .input;
    future_options["semantic_operations"][0]["operands"]
        .as_array_mut()
        .expect("model operands")
        .push(json!({
            "slot": "options",
            "value_id": "value.cap.out",
            "type_ref": "ModelCallOptions"
        }));
    assert!(!verify_air_json(&future_options).is_accepted());

    let mut nested = future.clone();
    nested["semantic_operations"][0]["operands"][1]["value_id"] = json!("value.request");
    nested["semantic_operations"][0]["parent_region_id"] = json!("region.loop.1");
    nested["semantic_operations"][1]["parent_region_id"] = json!("region.root");
    nested["semantic_operations"][1]["execution_order"] = json!(3);
    let nested_verdict = verify_air_json(&nested);
    assert!(
        nested_verdict.is_accepted(),
        "{:?}",
        nested_verdict.into_diagnostics()
    );
}

#[test]
fn source_map_vectors_match_verifier() {
    check("apxm.source-map.json", |v| {
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
    let schema = load_contract("schemas/apxm.air.json");
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
    let schema = load_contract("schemas/apxm.air.json");
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
    let schema = load_contract("schemas/apxm.source-map.json");
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
    let doc = load_vectors("apxm.air.json")
        .into_iter()
        .find(|v| v.name == "valid-air-five-semantic-ops-and-structural-ir")
        .expect("named vector present");
    let first = verify_air_json(&doc.input).into_diagnostics();
    let second = verify_air_json(&doc.input).into_diagnostics();
    assert_eq!(first, second, "verifier output is not deterministic");
}

fn predicate_air() -> Value {
    json!({
        "schema_version": "apxm.air",
        "semantic_operations": [{
            "node_id": "node.model",
            "op": "model.call",
            "parent_region_id": "region.fn",
            "execution_order": 0,
            "operands": [
                {"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"},
                {"slot": "request", "value_id": "value.request", "type_ref": "ModelRequest"}
            ],
            "result": {"value_id": "value.response", "type_ref": "ModelResponse"}
        }],
        "structural_ir": [
            {"region_id": "region.fn", "kind": "function", "execution_order": 0,
             "block_arguments": [{"value_id": "value.request", "type_ref": "ModelRequest"}]},
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
        "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
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
