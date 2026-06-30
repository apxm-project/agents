use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const AIS_SRC_SEGMENTS: &[&str] = &["..", "ais", "src"];
const MLIR_CONSTANTS_SEGMENTS: &[&str] = &[
    "..",
    "..",
    "compiler",
    "pipeline",
    "mlir",
    "include",
    "ais",
    "Common",
    "Constants.h",
];
const MLIR_CONSTANTS_FILE_NAME: &str = "Constants.h";
const INCLUDE_STR_PREFIX: &str = "include_str!(\"";
const INCLUDE_STR_SUFFIX: &str = "\")";

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let ais_src = join_segments(&manifest_dir, AIS_SRC_SEGMENTS);
    let mlir_constants_h = join_segments(&manifest_dir, MLIR_CONSTANTS_SEGMENTS);

    println!("cargo:rerun-if-changed={}", mlir_constants_h.display());

    generate_contract_module(
        &ais_src.join("attrs.rs"),
        &out_dir.join("apxm_graph_attrs.rs"),
        &[],
        Some(&mlir_constants_h),
    );
    generate_contract_module(
        &ais_src.join("operations/category.rs"),
        &out_dir.join("apxm_operation_category.rs"),
        &[],
        None,
    );
    generate_contract_module(
        &ais_src.join("operations/mlir_keywords.rs"),
        &out_dir.join("apxm_operation_mlir_keywords.rs"),
        &[],
        None,
    );
    generate_contract_module(
        &ais_src.join("operations/definitions.rs"),
        &out_dir.join("apxm_operation_definitions.rs"),
        &[("use crate::attrs;", "use crate::constants::graph::attrs;")],
        None,
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
        None,
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
        None,
    );
    generate_contract_module(
        &ais_src.join("types.rs"),
        &out_dir.join("apxm_value_contract.rs"),
        &[(
            "/// Type alias for node identifiers.\npub type NodeId = u64;\n\n",
            "",
        )],
        None,
    );
}

fn join_segments(base: &Path, segments: &[&str]) -> PathBuf {
    segments
        .iter()
        .fold(base.to_path_buf(), |path, segment| path.join(segment))
}

fn generate_contract_module(
    source: &Path,
    destination: &Path,
    replacements: &[(&str, &str)],
    include_rewrite_path: Option<&Path>,
) {
    println!("cargo:rerun-if-changed={}", source.display());

    let mut content =
        fs::read_to_string(source).unwrap_or_else(|err| panic!("read {}: {err}", source.display()));
    for (from, to) in replacements {
        content = content.replace(from, to);
    }
    if let Some(path) = include_rewrite_path {
        content = rewrite_include_str_target(&content, path);
    }
    content = normalize_inner_doc_comments(&content);

    fs::write(destination, content)
        .unwrap_or_else(|err| panic!("write {}: {err}", destination.display()));
}

fn rewrite_include_str_target(content: &str, include_target: &Path) -> String {
    let Some(include_start) = content.find(INCLUDE_STR_PREFIX) else {
        return content.to_string();
    };
    let path_start = include_start + INCLUDE_STR_PREFIX.len();
    let Some(path_len) = content[path_start..].find(INCLUDE_STR_SUFFIX) else {
        return content.to_string();
    };
    let path_end = path_start + path_len;
    let current_path = &content[path_start..path_end];
    if !current_path.ends_with(MLIR_CONSTANTS_FILE_NAME) {
        return content.to_string();
    }

    let mut rewritten = content.to_string();
    rewritten.replace_range(
        path_start..path_end,
        &rust_string_literal_path(include_target),
    );
    rewritten
}

fn rust_string_literal_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

fn normalize_inner_doc_comments(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            line.strip_prefix("//!")
                .map_or_else(|| line.to_string(), |rest| format!("//{rest}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
