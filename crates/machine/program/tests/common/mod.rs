//! Shared helpers for loading checked-in contract snapshots and conformance
//! vectors. External owner snapshots are immutable test inputs under this
//! crate's fixture root, so tests do not resolve a hidden sibling checkout.

use std::path::PathBuf;

use serde_json::Value;

fn agents_root() -> PathBuf {
    // crates/machine/program -> agents repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

/// Load a JSON document from `agents/contracts/<relative>`.
#[must_use]
pub fn load_contract(relative: &str) -> Value {
    load_json(agents_root().join("contracts").join(relative))
}

/// Load an immutable external-owner schema snapshot by its published path.
#[must_use]
pub fn load_contract_snapshot(relative: &str) -> Value {
    let filename = std::path::Path::new(relative)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("contract snapshot filename");
    let filename = if filename.starts_with("apxm.") {
        filename.to_owned()
    } else {
        format!("apxm.{filename}")
    };
    load_json(
        agents_root()
            .join("crates/machine/program/tests/fixtures/contracts")
            .join(filename),
    )
}

fn load_json(path: PathBuf) -> Value {
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// One conformance vector: a name, an input document, and its expected verdict.
pub struct Vector {
    pub name: String,
    pub input: Value,
    pub expected_valid: bool,
}

/// Load the vector array published at `agents/contracts/vectors/<file>`.
#[must_use]
pub fn load_vectors(file: &str) -> Vec<Vector> {
    let doc = load_contract(&format!("vectors/{file}"));
    let array = doc.as_array().expect("vectors file is a JSON array");
    array
        .iter()
        .map(|entry| Vector {
            name: entry["name"].as_str().expect("vector name").to_string(),
            input: entry["input"].clone(),
            expected_valid: entry["expected_valid"].as_bool().expect("expected_valid"),
        })
        .collect()
}

/// Compile a published owner schema into an executable validator.
///
/// The checked-in schemas address each other by bare `$id` (`apxm.…v1`), which
/// is a relative URI reference. Both the schema under test and every schema it
/// references are rebased onto the validator's default `json-schema:///` scope
/// so the published `$ref`s resolve without a network fetch and without editing
/// the checked-in bytes. Only the `$id` is rebased; every constraint the schema
/// states is compiled exactly as published.
#[must_use]
pub fn compile_schema(relative: &str, referenced_snapshots: &[&str]) -> jsonschema::JSONSchema {
    fn rebase(mut schema: Value) -> (String, Value) {
        let id = schema["$id"]
            .as_str()
            .expect("schema declares an $id")
            .to_string();
        let scoped = format!("json-schema:///{id}");
        schema["$id"] = Value::String(scoped.clone());
        (scoped, schema)
    }

    let (_, schema) = rebase(load_contract(relative));
    let mut options = jsonschema::JSONSchema::options();
    for reference in referenced_snapshots {
        let (scoped, document) = rebase(load_contract_snapshot(reference));
        options.with_document(scoped, document);
    }
    options
        .compile(&schema)
        .unwrap_or_else(|e| panic!("compile {relative}: {e}"))
}

/// The closed string members of a schema `enum` at a `$defs` path.
#[must_use]
pub fn schema_enum(schema: &Value, def: &str, property: &str) -> Vec<String> {
    let mut members: Vec<String> = schema["$defs"][def]["properties"][property]["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("enum at $defs.{def}.properties.{property}"))
        .iter()
        .map(|v| v.as_str().expect("enum member is a string").to_string())
        .collect();
    members.sort();
    members
}
