//! Shared helpers for loading the checked-in contract schemas and conformance
//! vectors from the owning `contracts` and `agents` trees. Paths are workspace
//! relative, resolved from this crate's manifest directory.

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
    let path = agents_root().join("contracts").join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
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
