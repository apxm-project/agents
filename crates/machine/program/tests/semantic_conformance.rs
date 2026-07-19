//! Conformance: the AIR, FrontendGraph, and source-map verifiers accept and
//! reject exactly the checked-in vectors, and the closed Rust enums do not drift
//! from the owning schemas.

mod common;

use apxm_program::air::{SemanticOpKind, StructuralKind};
use apxm_program::source_map::{RegionAnnotationKind, SourceLanguage};
use apxm_program::{verify_air_json, verify_frontend_graph_json, verify_source_map_json};
use common::{load_contract, load_vectors, schema_enum, Vector};
use serde_json::Value;

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
    check("apxm.air.v1.json", |v| verify_air_json(v).is_accepted());
}

#[test]
fn frontend_graph_vectors_match_verifier() {
    check("apxm.frontend-graph.v1.json", |v| {
        verify_frontend_graph_json(v).is_accepted()
    });
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
    let schema = load_contract("schemas/apxm.air.v1.json");
    let expected = schema_enum(&schema, "SemanticOp", "op");
    let actual = wire_members(&[
        SemanticOpKind::ModelCall,
        SemanticOpKind::CapabilityInvoke,
        SemanticOpKind::ProgramNew,
        SemanticOpKind::ProgramInvoke,
        SemanticOpKind::AwaitEvent,
    ]);
    assert_eq!(actual, expected, "AIR semantic op closure drifted from schema");
    assert_eq!(actual.len(), 5, "AIR exposes exactly five semantic operations");
}

#[test]
fn air_structural_kind_enum_does_not_drift() {
    let schema = load_contract("schemas/apxm.air.v1.json");
    let expected = schema_enum(&schema, "StructuralNode", "kind");
    let actual = wire_members(&[
        StructuralKind::Function,
        StructuralKind::Region,
        StructuralKind::Block,
        StructuralKind::Value,
        StructuralKind::Branch,
        StructuralKind::Switch,
        StructuralKind::Loop,
        StructuralKind::ParallelJoin,
        StructuralKind::Try,
        StructuralKind::Throw,
        StructuralKind::Catch,
        StructuralKind::Return,
        StructuralKind::Yield,
    ]);
    assert_eq!(actual, expected, "structural IR closure drifted from schema");
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
        wire_members(&[RegionAnnotationKind::None, RegionAnnotationKind::ConversationalLoop]),
        annotations,
        "region annotation closure drifted from schema",
    );

    let mut languages: Vec<String> = schema["$defs"]["SourceMapBody"]["properties"]["source_language"]
        ["enum"]
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
    let doc = load_vectors("apxm.air.v1.json")
        .into_iter()
        .find(|v| v.name == "valid-air-five-semantic-ops-and-structural-ir")
        .expect("named vector present");
    let first = verify_air_json(&doc.input).into_diagnostics();
    let second = verify_air_json(&doc.input).into_diagnostics();
    assert_eq!(first, second, "verifier output is not deterministic");
}
