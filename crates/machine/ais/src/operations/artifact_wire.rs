//! C++ artifact wire contract generation for canonical semantic operations.

use super::definitions::{SemanticOpKind, WIRE_INDEXED_OPERATIONS};
use std::fmt::Write;

pub const ARTIFACT_OPERATION_KIND_ENTRIES_FILE: &str = "OperationKind.generated.inc";
pub const ARTIFACT_OPERATION_KIND_CASES_FILE: &str = "OperationKindCases.generated.inc";

fn generated_header(file_name: &str) -> String {
    format!(
        "/*\n\
         * @file  {file_name}\n\
         * @brief GENERATED from Rust AIS semantic operation definitions - DO NOT EDIT MANUALLY\n\
         *\n\
         * Source of truth: crates/machine/ais/src/operations/definitions.rs\n\
         */\n\n"
    )
}

fn operation_kind_name(op: SemanticOpKind) -> String {
    format!("{op:?}")
}

pub fn generate_artifact_operation_kind_entries() -> String {
    let mut output = generated_header(ARTIFACT_OPERATION_KIND_ENTRIES_FILE);
    for &(wire_index, op) in WIRE_INDEXED_OPERATIONS {
        let name = operation_kind_name(op);
        let _ = writeln!(output, "  {name} = {wire_index},");
    }
    output
}

pub fn generate_artifact_operation_kind_cases() -> String {
    let mut output = generated_header(ARTIFACT_OPERATION_KIND_CASES_FILE);
    for &(_, op) in WIRE_INDEXED_OPERATIONS {
        let name = operation_kind_name(op);
        let _ = writeln!(
            output,
            "      .Case<{}>([](auto) {{ return OperationKind::{name}; }})",
            op.mlir_cpp_class(),
        );
    }
    output
}
