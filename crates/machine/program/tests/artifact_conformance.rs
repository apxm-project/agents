//! Conformance: the artifact/binding validation API accepts and rejects exactly
//! the checked-in vectors, the codec round-trips a valid artifact, and the
//! closed source-scope set does not drift from the owner schema snapshot.

mod common;
#[path = "common/snapshots.rs"]
mod snapshots;

use apxm_program::artifact::PortSourceScope;
use apxm_program::{ExecutableArtifact, validate_artifact_json};
use common::{Vector, load_contract, load_vectors};
use snapshots::load_contract_snapshot;

#[test]
fn artifact_vectors_match_validator() {
    for Vector {
        name,
        input,
        expected_valid,
    } in load_vectors("apxm.executable-artifact.json")
    {
        let verdict = validate_artifact_json(&input);
        let accepted = verdict.is_accepted();
        assert_eq!(
            accepted,
            expected_valid,
            "vector '{name}' expected valid={expected_valid} but validator returned {accepted}: {:?}",
            verdict.diagnostics(),
        );
    }
}

#[test]
fn only_artifact_semantic_requirements_pass() {
    // The non-artifact-semantic vectors are rejected specifically because of the
    // scope rule, not only the decode boundary.
    let vectors = load_vectors("apxm.executable-artifact.json");
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
    let vector = load_vectors("apxm.executable-artifact.json")
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
    let schema = load_contract("schemas/apxm.executable-artifact.json");
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
        "../crates/machine/program/tests/fixtures/example-artifacts/conversational-python.json",
        "../crates/machine/program/tests/fixtures/example-artifacts/conversational-typescript.json",
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
fn conversational_example_artifacts_pin_typed_tool_control_and_hooks() {
    for fixture in [
        "../crates/machine/program/tests/fixtures/example-artifacts/conversational-python.json",
        "../crates/machine/program/tests/fixtures/example-artifacts/conversational-typescript.json",
    ] {
        let artifact = load_contract(fixture);
        let hooks = artifact["hook_bindings"]
            .as_array()
            .expect("example hook bindings");
        assert_eq!(hooks.len(), 2, "{fixture}");
        assert_eq!(hooks[0]["phase"], "before", "{fixture}");
        assert_eq!(hooks[1]["phase"], "after", "{fixture}");
        assert_eq!(hooks[0]["scope"], "capability", "{fixture}");
        assert_eq!(hooks[1]["scope"], "capability", "{fixture}");
        assert_eq!(
            hooks[0]["target_selector"], hooks[1]["target_selector"],
            "{fixture}",
        );
        let all = artifact["air"]["semantic_operations"]
            .as_array()
            .expect("example semantic operations");
        let in_hook = |operation: &&serde_json::Value| {
            operation["parent_region_id"]
                .as_str()
                .is_some_and(|region| region.starts_with("hook."))
        };

        // A hook body carries its own effects, so the Agent's own calls are the
        // ones outside every hook region.
        let operations: Vec<_> = all.iter().filter(|node| !in_hook(node)).collect();
        assert_eq!(
            operations
                .iter()
                .filter(|operation| operation["op"] == "model.call")
                .count(),
            2,
            "{fixture}",
        );

        // The before-Hook measures the context and hands it to a second Model,
        // so both effects are captured inside the hook rather than referenced.
        let hook_ops: Vec<_> = all.iter().filter(in_hook).collect();
        for op in ["capability.invoke", "model.call"] {
            assert!(
                hook_ops.iter().any(|node| node["op"] == op),
                "{fixture}: hook body carries {op}",
            );
        }

        // The Tool is the one sharing a region with the re-entry Model; the
        // other invocations in the body are the Skills the program loads.
        let tool = operations
            .iter()
            .find(|operation| {
                operation["op"] == "capability.invoke"
                    && operations.iter().any(|other| {
                        other["op"] == "model.call"
                            && other["parent_region_id"] == operation["parent_region_id"]
                    })
            })
            .expect("declared Tool operation");
        let reentry_model = operations
            .iter()
            .find(|operation| {
                operation["op"] == "model.call"
                    && operation["parent_region_id"] == tool["parent_region_id"]
            })
            .expect("model re-entry in the selected Tool arm");
        let initial_model = operations
            .iter()
            .find(|operation| {
                operation["op"] == "model.call"
                    && operation["parent_region_id"] != tool["parent_region_id"]
            })
            .expect("initial model call outside the Tool arm");
        assert_ne!(
            initial_model["parent_region_id"], tool["parent_region_id"],
            "{fixture}",
        );
        assert_eq!(
            tool["parent_region_id"], reentry_model["parent_region_id"],
            "{fixture}",
        );
        assert!(
            tool["execution_order"]
                .as_u64()
                .expect("Tool execution order")
                < reentry_model["execution_order"]
                    .as_u64()
                    .expect("model re-entry execution order"),
            "{fixture}",
        );
        assert_eq!(hooks[0]["target_selector"], tool["node_id"], "{fixture}");

        let tool_result_id = tool["result"]["value_id"]
            .as_str()
            .expect("Tool result SSA id");
        let reentry_request_id = reentry_model["operands"]
            .as_array()
            .expect("re-entry operands")
            .iter()
            .find(|operand| operand["slot"] == "request")
            .and_then(|operand| operand["value_id"].as_str())
            .expect("re-entry request SSA id");
        let reentry_assembly = artifact["air"]["value_assemblies"]
            .as_array()
            .expect("request assemblies")
            .iter()
            .find(|assembly| assembly["value_id"] == reentry_request_id)
            .expect("re-entry request assembly");
        let tool_result_field = reentry_assembly["expression"]["fields"]
            .as_array()
            .expect("authored re-entry object fields")
            .iter()
            .find(|field| field["name"] == "tool_result")
            .expect("authored tool_result field");
        assert_eq!(
            tool_result_field["value"],
            serde_json::json!({"kind": "ssa", "value_id": tool_result_id}),
            "{fixture}: Hooks must not mask the authored Tool-result -> Model-request SSA edge",
        );

        let structural = artifact["air"]["structural_ir"]
            .as_array()
            .expect("example structural AIR");
        assert_eq!(
            structural
                .iter()
                .filter(|node| node["kind"] == "ais.loop")
                .count(),
            2,
            "{fixture}",
        );
        assert!(
            structural.iter().any(|node| node["kind"] == "yield"),
            "{fixture}",
        );
        let tool_loop = structural
            .iter()
            .find(|node| {
                node["kind"] == "ais.loop"
                    && node["predicate"]["property_path"] == serde_json::json!(["kind"])
            })
            .expect("typed Tool loop");
        assert_eq!(
            tool_loop["predicate"]["literal"]["value"], "tool_request",
            "{fixture}",
        );
        assert_eq!(
            tool_loop["block_arguments"]
                .as_array()
                .expect("loop block arguments")
                .len(),
            1,
            "{fixture}",
        );
        assert_eq!(
            tool_loop["operands"]
                .as_array()
                .expect("loop-carried operands")
                .len(),
            2,
            "{fixture}",
        );
        let declared_tool_branch = structural
            .iter()
            .find(|node| {
                node["kind"] == "branch"
                    && node["predicate"]["property_path"]
                        == serde_json::json!(["tool_request", "kind"])
            })
            .expect("closed declared-Tool branch");
        assert_eq!(
            declared_tool_branch["predicate"]["literal"]["value"], "search_web",
            "{fixture}",
        );
        assert!(
            structural.iter().any(|node| node["kind"] == "throw"),
            "{fixture} must fail closed for undeclared Tool requests",
        );
    }
}

#[test]
fn source_scope_enum_does_not_drift() {
    let schema = load_contract_snapshot("schemas/port-requirement.v1.json");
    let mut expected: Vec<String> = schema["properties"]["source_scope"]["enum"]
        .as_array()
        .expect("source_scope enum")
        .iter()
        .map(|v| v.as_str().expect("source scope enum member").to_string())
        .collect();
    expected.sort();

    let mut actual: Vec<String> = PortSourceScope::ALL
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
        "port source-scope closure drifted from owner schema"
    );
}
