//! C++ artifact wire contract generation.
//!
//! The Rust AIS definitions own the artifact operation-kind table. The native
//! compiler consumes these generated fragments instead of maintaining a second
//! handwritten wire table.

use super::definitions::{AISOperationType, WIRE_INDEXED_OPERATIONS};

pub const ARTIFACT_OPERATION_KIND_ENTRIES_FILE: &str = "OperationKind.generated.inc";
pub const ARTIFACT_OPERATION_KIND_CASES_FILE: &str = "OperationKindCases.generated.inc";

fn generated_header(file_name: &str) -> String {
    format!(
        "/*\n\
         * @file  {file_name}\n\
         * @brief GENERATED from Rust AIS operation definitions - DO NOT EDIT MANUALLY\n\
         *\n\
         * Source of truth: crates/core/apxm-ais/src/operations/definitions.rs\n\
         */\n\n"
    )
}

fn cpp_operation_name(op: AISOperationType) -> String {
    format!("{op:?}")
}

pub fn generate_artifact_operation_kind_entries() -> String {
    let mut output = generated_header(ARTIFACT_OPERATION_KIND_ENTRIES_FILE);
    for &(wire_index, op) in WIRE_INDEXED_OPERATIONS {
        let name = cpp_operation_name(op);
        output.push_str(&format!("  {name} = {wire_index},\n"));
    }
    output
}

pub fn generate_artifact_operation_kind_cases() -> String {
    let mut output = generated_header(ARTIFACT_OPERATION_KIND_CASES_FILE);
    for &(_, op) in WIRE_INDEXED_OPERATIONS {
        let name = cpp_operation_name(op);
        output.push_str(&format!(
            "      .Case<{name}Op>([](auto) {{ return OperationKind::{name}; }})\n"
        ));
    }
    output
}
