//! Shared helpers for loading checked-in contract snapshots and conformance
//! vectors. External owner snapshots are immutable test inputs under this
//! crate's fixture root, so tests do not resolve a hidden sibling checkout.

use std::path::PathBuf;

use serde_json::Value;

pub(crate) fn agents_root() -> PathBuf {
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

pub(crate) fn load_json(path: PathBuf) -> Value {
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
