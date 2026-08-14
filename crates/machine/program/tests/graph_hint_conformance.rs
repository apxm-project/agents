//! Conformance: the published `apxm.inference-graph-hints` schema is the sole
//! authority for the APXM graph-hint envelope. The checked-in vectors, the
//! published schema bytes, and the Rust decode + validation path are all held
//! against each other, so a limit can not drift out of one of the three.

mod common;

use apxm_core::types::{
    ApxmGraphHints, GraphHintCapabilities, GraphHintField, GraphHintPlan, GraphHintProjector,
    ProjectionOutcome,
};
use common::{Vector, compile_schema, load_vectors};
use serde_json::Value;

const VECTORS: &str = "apxm.inference-graph-hints.json";

fn hints_schema() -> jsonschema::JSONSchema {
    compile_schema("schemas/apxm.inference-graph-hints.json", &[])
}

/// The Rust admission verdict: an envelope is admitted only when it both
/// decodes into the closed type and passes the type's own validation. This is
/// exactly the path a projector takes before it renders anything.
fn rust_admits(document: &Value) -> Result<ApxmGraphHints, String> {
    let hints: ApxmGraphHints =
        serde_json::from_value(document.clone()).map_err(|e| e.to_string())?;
    hints.validate().map(|()| hints)
}

#[test]
fn graph_hint_vectors_match_schema_and_decode_path() {
    let schema = hints_schema();
    for Vector {
        name,
        input,
        expected_valid,
    } in load_vectors(VECTORS)
    {
        let by_schema = schema.is_valid(&input);
        assert_eq!(
            by_schema, expected_valid,
            "vector '{name}' expected valid={expected_valid} but the published schema \
             returned {by_schema}",
        );
        let by_rust = rust_admits(&input);
        assert_eq!(
            by_rust.is_ok(),
            expected_valid,
            "vector '{name}' expected valid={expected_valid} but the Rust decode path \
             returned {by_rust:?}",
        );
    }
}

/// The graph-hint capability digest is optional binding evidence, and it must
/// be a digest. Held against the published driver-binding schema so the
/// binding contract and the capability surface cannot drift apart.
///
/// Only the two capability vectors are checked here: the rest of that file
/// states cross-field digest-consistency verdicts a JSON schema cannot express,
/// and the Rust binding path owns those (`apxm-inference`).
#[test]
fn driver_binding_vectors_admit_the_optional_capability_digest() {
    let schema = compile_schema(
        "schemas/apxm.inference-driver-binding.json",
        &["schemas/contract-common.v1.json"],
    );
    let capability_vectors = [
        "valid-binding-carrying-a-graph-hint-capability-digest",
        "reject-non-digest-graph-hint-capability",
    ];
    let mut checked = 0;
    for Vector {
        name,
        input,
        expected_valid,
    } in load_vectors("apxm.inference-driver-binding.json")
    {
        if !capability_vectors.contains(&name.as_str()) {
            continue;
        }
        checked += 1;
        let by_schema = schema.is_valid(&input);
        assert_eq!(
            by_schema, expected_valid,
            "vector '{name}' expected valid={expected_valid} but the published schema \
             returned {by_schema}",
        );
    }
    assert_eq!(checked, capability_vectors.len());
}

#[test]
fn the_vector_file_exercises_both_verdicts() {
    let vectors = load_vectors(VECTORS);
    assert!(vectors.iter().any(|vector| vector.expected_valid));
    assert!(vectors.iter().any(|vector| !vector.expected_valid));
}

/// Phase A exit gate, across the JSON boundary: the construction path does not
/// change the digest. A document written by hand, a value decoded from it, and
/// that value re-encoded and decoded again all carry one identity.
#[test]
fn admitted_vectors_digest_identically_across_construction_paths() {
    for Vector {
        name,
        input,
        expected_valid,
    } in load_vectors(VECTORS)
    {
        if !expected_valid {
            continue;
        }
        let decoded = rust_admits(&input).expect("an admitted vector decodes");
        let round_tripped: ApxmGraphHints =
            serde_json::from_str(&serde_json::to_string(&decoded).expect("encode"))
                .expect("decode the re-encoded envelope");
        assert_eq!(
            decoded.digest(),
            round_tripped.digest(),
            "vector '{name}' digest depends on how the value was built",
        );
        assert_eq!(decoded.canonical_json(), round_tripped.canonical_json());
        assert!(decoded.digest().starts_with("sha256:"));
    }
}

/// Semantics decide the digest, not spelling. An empty successor list and an
/// absent one are the same envelope, so they carry one identity; two vectors
/// that state different facts do not.
#[test]
fn digest_identity_follows_semantics_not_spelling() {
    let digest_of = |name: &str| {
        let vector = load_vectors(VECTORS)
            .into_iter()
            .find(|vector| vector.name == name)
            .unwrap_or_else(|| panic!("vector {name} present"));
        rust_admits(&vector.input)
            .expect("an admitted vector decodes")
            .digest()
    };
    assert_eq!(
        digest_of("valid-scope-only-envelope"),
        digest_of("valid-unproven-facts-stay-absent"),
        "an empty reference list must not be a different envelope from an absent one",
    );
    assert_ne!(
        digest_of("valid-scope-only-envelope"),
        digest_of("valid-full-facts-and-intents"),
    );
    assert_ne!(
        digest_of("valid-full-facts-and-intents"),
        digest_of("valid-boundary-estimates-at-the-published-maxima"),
    );
}

/// Phase B exit gate, contract half: a backend with zero graph capabilities
/// produces a complete explicit report and adds nothing to the request.
#[test]
fn a_zero_capability_projector_reports_every_field_and_sends_none() {
    struct ZeroCapabilityBackend;
    impl GraphHintProjector for ZeroCapabilityBackend {}

    let hints = load_vectors(VECTORS)
        .into_iter()
        .find(|vector| vector.name == "valid-full-facts-and-intents")
        .map(|vector| rust_admits(&vector.input).expect("the full vector is admitted"))
        .expect("named vector present");

    let projected = ZeroCapabilityBackend
        .project_graph_hints(Some(&hints), 0)
        .expect("a zero-capability projection is a conforming projection");

    assert_eq!(projected.plan.outcomes.len(), GraphHintField::ALL.len());
    for field in GraphHintField::ALL {
        assert_eq!(
            projected.plan.outcomes.get(field),
            Some(&ProjectionOutcome::OmittedUnsupported),
            "field {field:?} has no explicit outcome",
        );
    }
    assert!(
        projected.provider_fields.is_empty(),
        "an unsupported field reached the provider request",
    );
    assert_eq!(
        projected.plan.capability_digest,
        GraphHintCapabilities::none().digest(),
    );
    assert_eq!(
        projected.plan.graph_hints_digest.as_deref(),
        Some(hints.digest().as_str()),
    );

    let evidence = projected.to_evidence_json();
    let rendered = evidence.to_string();
    for absent in ["prompt", "messages", "api_key", "extra_body"] {
        assert!(!rendered.contains(absent), "evidence carries {absent}");
    }
}

/// Absent hints leave the ordinary admitted request untouched: no digest, no
/// outcomes, no provider fields.
#[test]
fn absent_hints_produce_no_projection_at_all() {
    struct ZeroCapabilityBackend;
    impl GraphHintProjector for ZeroCapabilityBackend {}

    let projected = ZeroCapabilityBackend
        .project_graph_hints(None, 0)
        .expect("absent hints project");
    assert!(projected.plan.graph_hints_digest.is_none());
    assert!(projected.plan.outcomes.is_empty());
    assert!(projected.provider_fields.is_empty());
    assert_eq!(
        projected.plan,
        GraphHintPlan::absent(&GraphHintCapabilities::none()),
    );
}
