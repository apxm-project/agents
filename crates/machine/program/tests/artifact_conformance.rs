//! Conformance: the artifact/binding validation API accepts and rejects exactly
//! the checked-in vectors, the codec round-trips a valid artifact, and the
//! closed source-scope set does not drift from the owning contract.

mod common;

use apxm_program::artifact::PortSourceScope;
use apxm_program::{ExecutableArtifact, validate_artifact_json};
use common::{Vector, load_constitution, load_contract, load_vectors};

#[test]
fn artifact_vectors_match_validator() {
    for Vector {
        name,
        input,
        expected_valid,
    } in load_vectors("apxm.executable-artifact.v1.json")
    {
        let accepted = validate_artifact_json(&input).is_accepted();
        assert_eq!(
            accepted, expected_valid,
            "vector '{name}' expected valid={expected_valid} but validator returned {accepted}",
        );
    }
}

#[test]
fn only_artifact_semantic_requirements_pass() {
    // The non-artifact-semantic vectors are rejected specifically because of the
    // scope rule, not only the decode boundary.
    let vectors = load_vectors("apxm.executable-artifact.v1.json");
    for name in [
        "abstraction-deployment-infrastructure-requirement-rejected",
        "abstraction-invocation-authority-requirement-rejected",
    ] {
        let vector = vectors
            .iter()
            .find(|v| v.name == name)
            .expect("named vector");
        let verdict = validate_artifact_json(&vector.input);
        assert!(
            verdict.diagnostics().iter().any(
                |d| d.code == apxm_program::DiagnosticCode::RequirementScopeNotArtifactSemantic
            ),
            "vector '{name}' should be rejected by the artifact_semantic scope rule",
        );
    }
}

#[test]
fn codec_round_trips_valid_artifact() {
    let vector = load_vectors("apxm.executable-artifact.v1.json")
        .into_iter()
        .find(|v| v.name == "valid-artifact-emits-only-artifact-semantic-requirements")
        .expect("named vector present");
    let bytes = serde_json::to_vec(&vector.input).unwrap();
    let artifact = ExecutableArtifact::decode(&bytes).expect("decode valid artifact");
    assert!(artifact.validate().is_accepted());
    let reencoded = artifact.encode().expect("encode artifact");
    let redecoded = ExecutableArtifact::decode(&reencoded).expect("decode re-encoded artifact");
    assert_eq!(
        artifact, redecoded,
        "artifact codec is not a stable round-trip"
    );
}

#[test]
fn example_artifacts_carry_no_field_the_schema_rejects() {
    // The owner schema pins `additionalProperties: false`, so every top-level key
    // a repository example emits must be a declared property. This guards the
    // hook_bindings drift class: a field serialized by artifact.rs but absent
    // from the schema would be silently accepted by the serde validator yet
    // rejected by any strict JSON-schema consumer.
    let schema = load_contract("schemas/apxm.executable-artifact.v1.json");
    assert_eq!(
        schema["additionalProperties"],
        serde_json::Value::Bool(false),
        "schema must stay closed for this guard to be meaningful"
    );
    let declared: std::collections::HashSet<String> = schema["properties"]
        .as_object()
        .expect("schema properties")
        .keys()
        .cloned()
        .collect();

    for fixture in [
        "../crates/machine/program/tests/fixtures/example-artifacts/conversational-python.v1.json",
        "../crates/machine/program/tests/fixtures/example-artifacts/conversational-typescript.v1.json",
    ] {
        let artifact = load_contract(fixture);
        for key in artifact.as_object().expect("artifact object").keys() {
            assert!(
                declared.contains(key),
                "runtime-proof fixture '{fixture}' emits undeclared top-level field '{key}' \
                 that a strict schema consumer would reject",
            );
        }
    }
}

#[test]
fn source_scope_enum_does_not_drift() {
    let schema = load_constitution("schemas/port-requirement.v1.json");
    let mut expected: Vec<String> = schema["properties"]["source_scope"]["enum"]
        .as_array()
        .expect("source_scope enum")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    expected.sort();

    let mut actual: Vec<String> = [
        PortSourceScope::ArtifactSemantic,
        PortSourceScope::DeploymentInfrastructure,
        PortSourceScope::InvocationAuthority,
    ]
    .iter()
    .map(|v| {
        serde_json::to_value(v)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    })
    .collect();
    actual.sort();

    assert_eq!(
        actual, expected,
        "port source-scope closure drifted from contract"
    );
}
