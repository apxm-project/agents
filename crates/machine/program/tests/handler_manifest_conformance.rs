//! Conformance: the published `apxm.handler-manifest` schema is the sole
//! authority for handler manifests. The checked-in vectors, the Rust decode and
//! validation path, and every manifest a repository producer emits are all held
//! against the same published schema bytes, so the three cannot diverge without
//! a test failing.

mod common;

use apxm_core::types::{
    HANDLER_MANIFEST_HANDLER_ID_HEX_LENGTH, HANDLER_MANIFEST_HANDLER_ID_PREFIX,
    HANDLER_MANIFEST_VERSION, HandlerLanguage, HandlerManifest, HandlerManifestError,
};
use common::{Vector, compile_schema, load_contract, load_vectors};
use serde_json::Value;

/// The published schema, compiled from its checked-in bytes. `Identifier` is
/// owned by the common contract snapshot and referenced by `module`, `qualname`,
/// and `name`, so it is supplied to the validator rather than restated here.
fn manifest_schema() -> jsonschema::JSONSchema {
    compile_schema(
        "schemas/apxm.handler-manifest.json",
        &["schemas/contract-common.v1.json"],
    )
}

/// The Rust admission verdict: a manifest is admitted only if it both decodes
/// into the closed type and passes the type's own validation. This is exactly
/// the path `load_typescript_tools_manifest` takes before trusting a manifest.
fn rust_admits(document: &Value) -> Result<HandlerManifest, String> {
    let bytes = serde_json::to_vec(document).expect("serialize candidate manifest");
    let manifest = HandlerManifest::from_json_slice(&bytes).map_err(|e| e.to_string())?;
    manifest
        .validate()
        .map(|()| manifest)
        .map_err(|e: HandlerManifestError| e.to_string())
}

#[test]
fn handler_manifest_vectors_match_schema_and_decode_path() {
    let schema = manifest_schema();
    for Vector {
        name,
        input,
        expected_valid,
    } in load_vectors("apxm.handler-manifest.json")
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

#[test]
fn valid_vector_round_trips_byte_for_byte() {
    let vector = load_vectors("apxm.handler-manifest.json")
        .into_iter()
        .find(|v| v.name == "valid-tool-manifest")
        .expect("named vector present");
    let manifest = rust_admits(&vector.input).expect("the valid vector is admitted");
    let reencoded = serde_json::to_value(&manifest).expect("re-encode manifest");
    assert_eq!(
        reencoded, vector.input,
        "the manifest codec is not a stable round-trip: re-encoding the published \
         vector must reproduce it exactly, adding no field the vector omits and \
         dropping no field it carries",
    );
    assert!(
        manifest_schema().is_valid(&reencoded),
        "a re-encoded manifest must still satisfy the published schema",
    );
}

/// Every constraint the schema states is enforced by the Rust decode path.
///
/// Each candidate is a manifest that differs from an admitted one in exactly one
/// published constraint. Asserting both verdicts on the same document is what
/// makes this a drift gate rather than a restatement of either side: a schema
/// constraint the Rust type stops enforcing, or a Rust rule the schema stops
/// stating, fails here.
#[test]
fn schema_constraints_are_each_enforced_by_the_decode_path() {
    // `handler_id` is a content address the schema pins to `sha256:` + 64
    // lowercase hex. These fixture identities are not digests of any bytes;
    // they are fixed placeholders chosen only to satisfy that shape.
    const TOOL_ID_FIXTURE: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    let tool = || {
        serde_json::json!({
            "kind": "tool",
            "language": "typescript",
            "handler_id": TOOL_ID_FIXTURE,
            "module": "capabilities/echo/handler",
            "qualname": "echo",
            "name": "echo",
            "source": {
                "artifact_path": "handlers/echo.mjs",
                "content": "export function echo() {}\n"
            },
            "schema": {"type": "object"}
        })
    };
    let manifest = |descriptor: Value| serde_json::json!({"version": "apxm.handler-manifest", "handlers": [descriptor]});
    // Apply one field mutation; a JSON null removes the field.
    let with = |mut descriptor: Value, field: &str, value: Value| {
        if value.is_null() {
            descriptor
                .as_object_mut()
                .expect("descriptor")
                .remove(field);
        } else {
            descriptor[field] = value;
        }
        manifest(descriptor)
    };

    let candidates: Vec<(&str, Value, bool)> = vec![
        // Baseline: the published tool descriptor is admitted.
        ("tool baseline", manifest(tool()), true),
        // `version` is a schema `const`.
        (
            "manifest version not the published constant",
            serde_json::json!({"version": "apxm.handler-manifest-unpublished", "handlers": []}),
            false,
        ),
        (
            "manifest version absent",
            serde_json::json!({"handlers": []}),
            false,
        ),
        // `additionalProperties: false`, at both levels.
        (
            "undeclared top-level field",
            serde_json::json!({
                "version": "apxm.handler-manifest",
                "handlers": [],
                "runtime_profile": "server"
            }),
            false,
        ),
        (
            "undeclared descriptor field",
            with(tool(), "runtime_profile", serde_json::json!("server")),
            false,
        ),
        // The `HandlerId` pattern: `sha256:` + exactly 64 lowercase hex.
        (
            "handler_id without the sha256 prefix",
            with(
                tool(),
                "handler_id",
                serde_json::json!(
                    "1111111111111111111111111111111111111111111111111111111111111111"
                ),
            ),
            false,
        ),
        (
            "handler_id with uppercase hex",
            with(
                tool(),
                "handler_id",
                serde_json::json!(
                    "sha256:AAAA111111111111111111111111111111111111111111111111111111111111"
                ),
            ),
            false,
        ),
        (
            "handler_id one hex digit short",
            with(
                tool(),
                "handler_id",
                serde_json::json!(
                    "sha256:111111111111111111111111111111111111111111111111111111111111111"
                ),
            ),
            false,
        ),
        // Required tool fields.
        (
            "tool without schema",
            with(tool(), "schema", Value::Null),
            false,
        ),
        (
            "tool without source",
            with(tool(), "source", Value::Null),
            false,
        ),
        // Lifecycle hooks are captured bodies in AIR, never manifest
        // descriptors, so a lifecycle field here is an unknown field.
        (
            "descriptor carrying a lifecycle field",
            with(tool(), "mode", serde_json::json!("observe")),
            false,
        ),
        // `HandlerLanguage` is closed to `typescript`.
        (
            "python package-local handler language",
            with(tool(), "language", serde_json::json!("python")),
            false,
        ),
        // `HandlerSource.artifact_path` is artifact-confined and non-empty.
        (
            "build-host absolute artifact path",
            with(
                tool(),
                "source",
                serde_json::json!({
                    "artifact_path": "/build/echo.mjs",
                    "content": "export function echo() {}\n"
                }),
            ),
            false,
        ),
        (
            "artifact path escaping the artifact",
            with(
                tool(),
                "source",
                serde_json::json!({
                    "artifact_path": "handlers/../../escape.mjs",
                    "content": "export function echo() {}\n"
                }),
            ),
            false,
        ),
        (
            "empty artifact-local source content",
            with(
                tool(),
                "source",
                serde_json::json!({"artifact_path": "handlers/echo.mjs", "content": ""}),
            ),
            false,
        ),
        // `module`, `qualname`, and `name` are constitution `Identifier`s.
        (
            "module is not an identifier",
            with(
                tool(),
                "module",
                serde_json::json!("capabilities/echo handler"),
            ),
            false,
        ),
        (
            "qualname is not an identifier",
            with(tool(), "qualname", serde_json::json!("echo!")),
            false,
        ),
        (
            "name does not start alphanumeric",
            with(tool(), "name", serde_json::json!(".echo")),
            false,
        ),
        (
            "name exceeds the identifier length ceiling",
            with(tool(), "name", serde_json::json!("a".repeat(257))),
            false,
        ),
        (
            "name at the identifier length ceiling",
            with(tool(), "name", serde_json::json!("a".repeat(256))),
            true,
        ),
    ];

    let schema = manifest_schema();
    for (label, document, expected) in candidates {
        let by_schema = schema.is_valid(&document);
        assert_eq!(
            by_schema, expected,
            "'{label}': the published schema returned {by_schema}, expected {expected}",
        );
        let by_rust = rust_admits(&document);
        assert_eq!(
            by_rust.is_ok(),
            expected,
            "'{label}': the published schema returned {by_schema} but the Rust decode \
             path returned {by_rust:?} — the schema and the type have diverged",
        );
    }
}

/// The Rust constants that name the contract are read out of the published
/// schema bytes, not restated. Each assertion compares a Rust constant to the
/// schema text that defines it, so changing either side alone fails.
#[test]
fn manifest_constants_are_read_from_the_published_schema() {
    let schema = load_contract("schemas/apxm.handler-manifest.json");

    assert_eq!(
        schema["properties"]["version"]["const"].as_str(),
        Some(HANDLER_MANIFEST_VERSION),
        "the version constant drifted from the schema `const`",
    );
    assert_eq!(
        schema["$id"].as_str(),
        Some(HANDLER_MANIFEST_VERSION),
        "the schema publishes the version constant as its own `$id`",
    );

    // The handler-id prefix and hex width are stated once, as a schema pattern.
    let pattern = schema["$defs"]["HandlerId"]["pattern"]
        .as_str()
        .expect("HandlerId pattern");
    assert_eq!(
        pattern,
        format!(
            "^{HANDLER_MANIFEST_HANDLER_ID_PREFIX}[0-9a-f]{{{HANDLER_MANIFEST_HANDLER_ID_HEX_LENGTH}}}$"
        ),
        "the handler-id prefix or hex width drifted from the schema pattern",
    );

    // The closed language set is read off the Rust enum's own wire form, so a
    // variant added to either side without the other fails here.
    let mut languages: Vec<String> = [HandlerLanguage::TypeScript]
        .iter()
        .map(|v| {
            serde_json::to_value(v)
                .expect("serialize language variant")
                .as_str()
                .expect("language serializes to a string")
                .to_string()
        })
        .collect();
    languages.sort();
    let mut published: Vec<String> = schema["$defs"]["HandlerLanguage"]["enum"]
        .as_array()
        .expect("HandlerLanguage enum")
        .iter()
        .map(|v| v.as_str().expect("enum member is a string").to_string())
        .collect();
    published.sort();
    assert_eq!(
        languages, published,
        "the closed handler-language set drifted from the schema enum",
    );
}

/// Every handler manifest a repository producer emits satisfies the schema it
/// names. A producer writing a manifest the contract rejects fails here, at
/// production, rather than at some downstream consumer.
#[test]
fn produced_example_manifests_satisfy_the_published_schema() {
    let schema = manifest_schema();
    for produced in ["../examples/agents/coder/capabilities/handlers/tools.json"] {
        let manifest = load_contract(produced);
        if let Err(error) = schema.validate(&manifest) {
            let reasons: Vec<String> = error.map(|e| format!("{}: {e}", e.instance_path)).collect();
            panic!("produced manifest '{produced}' violates its own contract: {reasons:?}");
        }
        rust_admits(&manifest)
            .unwrap_or_else(|e| panic!("produced manifest '{produced}' is not admitted: {e}"));
    }
}
