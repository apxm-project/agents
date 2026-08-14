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
    compile_schema_with(relative, referenced_snapshots, &[])
}

/// Compile a published owner schema that also references sibling schemas this
/// repository publishes itself, given by their `contracts/`-relative paths.
#[must_use]
pub fn compile_schema_with(
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
    /// document's rebased `$id`. Without this, a supplied schema whose `$defs`
    /// reference each other resolves them against the schema under test and
    /// fails on a reference the published bytes state correctly.
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
        supply(&mut options, load_contract_snapshot(reference));
    }
    for reference in referenced_contracts {
        supply(&mut options, load_contract(reference));
    }
    options
        .compile(&schema)
        .unwrap_or_else(|e| panic!("compile {relative}: {e}"))
}

/// Every conformance vector file published under `contracts/vectors/`.
#[must_use]
pub fn published_vector_files() -> Vec<String> {
    published_contract_files("vectors")
}

/// Every JSON document published under `contracts/<directory>/`, sorted.
#[must_use]
pub fn published_contract_files(directory: &str) -> Vec<String> {
    let root = agents_root().join("contracts").join(directory);
    let mut files: Vec<String> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("read {}: {e}", root.display()))
        .map(|entry| entry.expect("directory entry").file_name())
        .filter_map(|name| name.to_str().map(str::to_owned))
        .filter(|name| name.ends_with(".json"))
        .collect();
    files.sort();
    files
}

/// The SHA-256 content address of a published contract document's exact bytes,
/// in the `sha256:<hex>` form the Port Contracts use.
#[must_use]
pub fn contract_file_digest(relative: &str) -> String {
    use sha2::{Digest, Sha256};

    let path = agents_root().join("contracts").join(relative);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Whether a schema is published at `contracts/schemas/<schema_id>.json`.
#[must_use]
pub fn contract_file_exists(relative: &str) -> bool {
    agents_root().join("contracts").join(relative).is_file()
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
