use serde_json::Value;

use crate::common::load_contract;

#[path = "snapshots.rs"]
mod snapshots;

fn rebase(mut schema: Value) -> (String, Value) {
    let id = schema["$id"]
        .as_str()
        .expect("schema declares an $id")
        .to_string();
    let scoped = format!("json-schema:///{id}");
    schema["$id"] = Value::String(scoped.clone());
    (scoped, schema)
}

/// A supplied document's own `#/$defs/…` references are relative to that
/// document, not to the validator's root scope, so they are anchored to the
/// document's rebased `$id`.
fn anchor_self_references(node: &mut Value, scope: &str) {
    match node {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get_mut("$ref") {
                if let Some(fragment) = reference.strip_prefix('#') {
                    *reference = format!("{scope}#{fragment}");
                }
            }
            for value in map.values_mut() {
                anchor_self_references(value, scope);
            }
        }
        Value::Array(items) => {
            for item in items {
                anchor_self_references(item, scope);
            }
        }
        _ => {}
    }
}

fn supply(options: &mut jsonschema::CompilationOptions, document: Value) {
    let (scoped, mut document) = rebase(document);
    anchor_self_references(&mut document, &scoped);
    options.with_document(scoped, document);
}

fn options_with(
    referenced_snapshots: &[&str],
    referenced_contracts: &[&str],
) -> jsonschema::CompilationOptions {
    let mut options = jsonschema::JSONSchema::options();
    for reference in referenced_snapshots {
        supply(&mut options, snapshots::load_contract_snapshot(reference));
    }
    for reference in referenced_contracts {
        supply(&mut options, load_contract(reference));
    }
    options
}

/// Compile a published owner schema that also references sibling schemas this
/// repository publishes itself, given by their `contracts/`-relative paths.
#[must_use]
pub(crate) fn compile_schema_with(
    relative: &str,
    referenced_snapshots: &[&str],
    referenced_contracts: &[&str],
) -> jsonschema::JSONSchema {
    let (_, schema) = rebase(load_contract(relative));
    options_with(referenced_snapshots, referenced_contracts)
        .compile(&schema)
        .unwrap_or_else(|e| panic!("compile {relative}: {e}"))
}

/// Compile the subschema a published schema carries at one JSON Pointer — a
/// `$defs` envelope the root document never carries, so vectors validated
/// against the root can say nothing about it.
///
/// The schema is supplied to the validator whole and referenced by pointer, so
/// the pointed node keeps its own document's `$defs` and external references in
/// scope and is compiled exactly as the root compiles it.
#[must_use]
pub(crate) fn compile_pointer_schema_with(
    relative: &str,
    pointer: &str,
    referenced_snapshots: &[&str],
    referenced_contracts: &[&str],
) -> jsonschema::JSONSchema {
    assert!(
        pointer.starts_with('#'),
        "{relative}: a subschema pointer is a fragment, e.g. #/$defs/Name"
    );
    let document = load_contract(relative);
    assert!(
        !document
            .pointer(pointer.trim_start_matches('#'))
            .expect("the pointed subschema")
            .is_null(),
        "{relative}: publishes nothing at {pointer}"
    );
    let scoped = format!(
        "json-schema:///{}",
        document["$id"].as_str().expect("schema declares an $id")
    );
    let mut options = options_with(referenced_snapshots, referenced_contracts);
    supply(&mut options, document);
    options
        .compile(&serde_json::json!({ "$ref": format!("{scoped}{pointer}") }))
        .unwrap_or_else(|e| panic!("compile {relative}{pointer}: {e}"))
}
