//! Conformance: the skill verifiers accept and reject exactly the checked-in
//! vectors, and the closed Rust types do not drift from the owning schemas.
//!
//! Before this file existed, `apxm.skill-package` and
//! `apxm.skill-discovery-root` were published with vectors and read by no code
//! in any language — the schemas and their vectors were a description of a
//! contract nothing enforced. These tests are the enforcement.

mod common;

use apxm_program::skill::{
    DiscoveryRootSkill, PackageLocalSkill, SkillDiscoveryRoot, verify_package_local_skill_json,
    verify_skill_discovery_root_json, verify_skill_package_json,
};
use common::{Vector, load_contract, load_vectors};
use serde_json::{Value, json};

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
fn skill_package_vectors_match_verifier() {
    check("apxm.skill-package.json", |v| {
        verify_skill_package_json(v).is_accepted()
    });
}

#[test]
fn package_local_skill_vectors_match_verifier() {
    check("apxm.package-local-skill.json", |v| {
        verify_package_local_skill_json(v).is_accepted()
    });
}

#[test]
fn skill_discovery_root_vectors_match_verifier() {
    check("apxm.skill-discovery-root.json", |v| {
        verify_skill_discovery_root_json(v).is_accepted()
    });
}

/// The two carriers are a split, not one definition stretched over two uses.
/// The split is visible in the published constraint blocks: the standalone form
/// denies being an executable package, the carried form asserts that an
/// executable package carries it, and neither block's key set is a subset of
/// the other's. Holding that here means a later "simplification" that merges
/// them fails a test instead of quietly reintroducing the contradiction this
/// split resolved — a package-local skill cannot satisfy
/// `executable_package: false`, and a `resources` list required to be non-empty
/// cannot describe a skill that carries only its own instructions.
#[test]
fn the_two_skill_carriers_state_different_constraint_blocks() {
    let discovery_root = load_contract("schemas/apxm.skill-package.json");
    let package_local = load_contract("schemas/apxm.package-local-skill.json");

    let keys = |schema: &Value| -> Vec<String> {
        schema["properties"]["constraints"]["properties"]
            .as_object()
            .expect("a constraints block")
            .keys()
            .cloned()
            .collect()
    };
    let discovery_keys = keys(&discovery_root);
    let package_keys = keys(&package_local);
    assert_ne!(discovery_keys, package_keys);
    assert!(discovery_keys.contains(&"executable_package".to_string()));
    assert!(!discovery_keys.contains(&"carried_by_executable_package".to_string()));
    assert!(package_keys.contains(&"carried_by_executable_package".to_string()));
    assert!(!package_keys.contains(&"executable_package".to_string()));

    assert_eq!(
        discovery_root["properties"]["constraints"]["properties"]["executable_package"]["const"],
        json!(false),
        "a discovery-root skill is not an executable package"
    );
    assert_eq!(
        package_local["properties"]["constraints"]["properties"]["carried_by_executable_package"]["const"],
        json!(true),
        "a package-local skill is carried by an executable package"
    );

    // The `resources` contradiction the split exists to resolve: the standalone
    // form's list is its complete inventory and cannot be empty; the carried
    // form's list is only the extras and can be.
    assert_eq!(
        discovery_root["properties"]["resources"]["minItems"],
        json!(1)
    );
    assert_eq!(
        package_local["properties"]["resources"]["minItems"],
        json!(0)
    );
}

/// `instruction_source` is discriminated, and exactly one branch may be given.
/// The discovery-root carrier admits only the file-carried branch because a
/// standalone skill has no Agent Program source bundle for inline instructions
/// to already be inside.
#[test]
fn the_instruction_source_discriminant_is_published_as_two_exclusive_branches() {
    let discovery_root = load_contract("schemas/apxm.skill-package.json");
    let package_local = load_contract("schemas/apxm.package-local-skill.json");

    let entry = &discovery_root["$defs"]["InstructionEntrySource"];
    let inline = &discovery_root["$defs"]["InstructionInlineSource"];
    assert_eq!(entry["properties"]["kind"]["const"], json!("entry"));
    assert_eq!(inline["properties"]["kind"]["const"], json!("inline"));
    assert_eq!(entry["additionalProperties"], json!(false));
    assert_eq!(inline["additionalProperties"], json!(false));

    // The integrity anchors differ, which is the whole reason for the split: a
    // package path digest for the file, the source-bundle digest for inline.
    assert!(entry["properties"].get("path").is_some());
    assert!(entry["properties"].get("digest").is_some());
    assert!(inline["properties"].get("path").is_none());
    assert!(inline["properties"].get("digest").is_none());
    assert!(inline["properties"].get("source_bundle_digest").is_some());

    // The standalone carrier names the entry branch directly; the carried
    // carrier offers both under a `oneOf`, so exactly one may match.
    assert_eq!(
        discovery_root["properties"]["instruction_source"]["$ref"],
        json!("#/$defs/InstructionEntrySource")
    );
    let branches = package_local["properties"]["instruction_source"]["oneOf"]
        .as_array()
        .expect("the carried form publishes both branches");
    assert_eq!(branches.len(), 2);
}

/// Each document type is closed on both sides — `additionalProperties: false`
/// in the published schema, `deny_unknown_fields` in Rust — so a field added to
/// one side only either fails decode or is silently dropped.
#[test]
fn skill_document_field_sets_are_closed_identically_in_schema_and_rust() {
    for (relative, valid_vector) in [
        ("schemas/apxm.skill-package.json", "apxm.skill-package.json"),
        (
            "schemas/apxm.package-local-skill.json",
            "apxm.package-local-skill.json",
        ),
        (
            "schemas/apxm.skill-discovery-root.json",
            "apxm.skill-discovery-root.json",
        ),
    ] {
        let schema = load_contract(relative);
        assert_eq!(
            schema["additionalProperties"],
            json!(false),
            "{relative} must stay closed"
        );
        let mut declared: Vec<String> = schema["properties"]
            .as_object()
            .expect("published properties")
            .keys()
            .cloned()
            .collect();
        declared.sort();

        // Re-encoding a decoded valid vector names exactly the fields Rust emits.
        let input = load_vectors(valid_vector)
            .into_iter()
            .find(|vector| vector.expected_valid)
            .expect("each shape publishes a valid vector")
            .input;
        let encoded = match relative {
            "schemas/apxm.skill-package.json" => serde_json::to_value(
                serde_json::from_value::<DiscoveryRootSkill>(input).expect("decode"),
            ),
            "schemas/apxm.package-local-skill.json" => serde_json::to_value(
                serde_json::from_value::<PackageLocalSkill>(input).expect("decode"),
            ),
            _ => serde_json::to_value(
                serde_json::from_value::<SkillDiscoveryRoot>(input).expect("decode"),
            ),
        }
        .expect("re-encode");
        let mut emitted: Vec<String> = encoded
            .as_object()
            .expect("document object")
            .keys()
            .cloned()
            .collect();
        emitted.sort();
        assert_eq!(
            emitted, declared,
            "{relative}: the Rust field set drifted from the published schema"
        );
    }
}

/// A discovery root publishes cards, never bodies. The constraint is stated in
/// the schema and enforced at decode; this holds the two together so relaxing
/// either alone fails.
#[test]
fn a_discovery_card_can_never_carry_an_instruction_body() {
    let schema = load_contract("schemas/apxm.skill-discovery-root.json");
    assert_eq!(
        schema["properties"]["discovery_mode"]["const"],
        json!("metadata_only")
    );
    assert_eq!(
        schema["properties"]["constraints"]["properties"]["body_loaded_on_call"]["const"],
        json!(true)
    );
    assert_eq!(
        schema["$defs"]["SkillCard"]["additionalProperties"],
        json!(false)
    );

    let mut root = load_vectors("apxm.skill-discovery-root.json")
        .into_iter()
        .find(|vector| vector.expected_valid)
        .expect("a valid discovery root vector")
        .input;
    assert!(verify_skill_discovery_root_json(&root).is_accepted());
    root["skill_cards"][0]["instruction_body"] = json!("# smuggled");
    assert!(
        !verify_skill_discovery_root_json(&root).is_accepted(),
        "a body smuggled into a card must be refused"
    );
}

#[test]
fn skill_verifiers_are_deterministic() {
    let vector = load_vectors("apxm.skill-package.json")
        .into_iter()
        .find(|vector| !vector.expected_valid)
        .expect("a rejecting vector");
    let first = verify_skill_package_json(&vector.input).into_diagnostics();
    let second = verify_skill_package_json(&vector.input).into_diagnostics();
    assert_eq!(first, second, "skill verification is not deterministic");
    assert!(!first.is_empty());
}
