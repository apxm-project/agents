use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let ais_src = manifest_dir.join("../apxm-ais/src");

    generate_contract_module(
        &ais_src.join("attrs.rs"),
        &out_dir.join("apxm_graph_attrs.rs"),
        &[],
    );
    generate_contract_module(
        &ais_src.join("operations/category.rs"),
        &out_dir.join("apxm_operation_category.rs"),
        &[],
    );
    generate_contract_module(
        &ais_src.join("operations/mlir_keywords.rs"),
        &out_dir.join("apxm_operation_mlir_keywords.rs"),
        &[],
    );
    generate_contract_module(
        &ais_src.join("operations/definitions.rs"),
        &out_dir.join("apxm_operation_definitions.rs"),
        &[("use crate::attrs;", "use crate::constants::graph::attrs;")],
    );
    generate_contract_module(
        &ais_src.join("validation.rs"),
        &out_dir.join("apxm_operation_validation.rs"),
        &[
            ("use crate::attrs;", "use crate::constants::graph::attrs;"),
            (
                "use crate::operations::{AISOperationType, get_operation_spec};",
                "use super::definitions::{AISOperationType, get_operation_spec};",
            ),
        ],
    );
    generate_contract_module(
        &ais_src.join("passes/mod.rs"),
        &out_dir.join("apxm_pass_catalog.rs"),
        &[
            ("mod tablegen;\n\n", ""),
            (
                "pub use tablegen::{generate_pass_descriptors, generate_pass_dispatch, generate_passes_tablegen};\n\n",
                "",
            ),
        ],
    );
    generate_contract_module(
        &ais_src.join("types.rs"),
        &out_dir.join("apxm_value_contract.rs"),
        &[("/// Type alias for node identifiers.\npub type NodeId = u64;\n\n", "")],
    );
}

fn generate_contract_module(source: &Path, destination: &Path, replacements: &[(&str, &str)]) {
    println!("cargo:rerun-if-changed={}", source.display());

    let mut content =
        fs::read_to_string(source).unwrap_or_else(|err| panic!("read {}: {err}", source.display()));
    for (from, to) in replacements {
        content = content.replace(from, to);
    }
    content = normalize_inner_doc_comments(&content);

    fs::write(destination, content)
        .unwrap_or_else(|err| panic!("write {}: {err}", destination.display()));
}

fn normalize_inner_doc_comments(content: &str) -> String {
    content
        .lines()
        .map(|line| line.strip_prefix("//!").map_or_else(|| line.to_string(), |rest| format!("//{rest}")))
        .collect::<Vec<_>>()
        .join("\n")
}
