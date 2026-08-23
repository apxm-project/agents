use serde_json::Value;

use crate::common::load_contract;

#[path = "snapshots.rs"]
mod snapshots;

/// Compile a published owner schema that also references sibling schemas this
/// repository publishes itself, given by their `contracts/`-relative paths.
#[must_use]
pub(crate) fn compile_schema_with(
    relative: &str,
    referenced_snapshots: &[&str],
    referenced_contracts: &[&str],
) -> jsonschema::JSONSchema {
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

    let (_, schema) = rebase(load_contract(relative));
    let mut options = jsonschema::JSONSchema::options();
    for reference in referenced_snapshots {
        supply(&mut options, snapshots::load_contract_snapshot(reference));
    }
    for reference in referenced_contracts {
        supply(&mut options, load_contract(reference));
    }
    options
        .compile(&schema)
        .unwrap_or_else(|e| panic!("compile {relative}: {e}"))
}
