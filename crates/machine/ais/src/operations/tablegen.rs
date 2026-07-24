//! TableGen declarations for the canonical AIS operation families.

use super::definitions::{OperationSpec, StructuralOpKind, get_all_operations};
use std::fmt::Write;

/// Filename for the checked-in semantic TableGen declarations consumed by MLIR.
pub const SEMANTIC_TABLEGEN_DECLARATIONS_FILE: &str = "AISOps.semantic.generated.td";
/// Filename for the checked-in structural TableGen declarations consumed by MLIR.
pub const STRUCTURAL_TABLEGEN_DECLARATIONS_FILE: &str = "AISOps.structural.generated.td";

/// Render the canonical semantic declarations consumed by MLIR TableGen.
pub fn generate_semantic_tablegen_declarations() -> String {
    let mut output = String::from(
        "/*\n\
         * @file  AISOps.semantic.generated.td\n\
         * @brief GENERATED from Rust AIS semantic operation definitions - DO NOT EDIT MANUALLY\n\
         *\n\
         * Source of truth: crates/machine/ais/src/operations/definitions.rs\n\
         */\n\n\
         // Canonical five public semantic operations.\n\n",
    );
    append_semantic_ops(&mut output);
    with_single_trailing_newline(output)
}

/// Render the closed compiler-emitted structural declarations consumed by MLIR.
pub fn generate_structural_tablegen_declarations() -> String {
    let mut output = String::from(
        "/*\n\
         * @file  AISOps.structural.generated.td\n\
         * @brief GENERATED from Rust AIS structural operation definitions - DO NOT EDIT MANUALLY\n\
         *\n\
         * Source of truth: crates/machine/ais/src/operations/definitions.rs\n\
         */\n\n\
         // Closed compiler-emitted structural operations.\n\n",
    );
    for kind in StructuralOpKind::all() {
        output.push_str(&generate_structural_op_def(kind));
        output.push_str("\n\n");
    }
    with_single_trailing_newline(output)
}

/// Render the standalone canonical AIS TableGen document.
pub fn generate_tablegen() -> String {
    format!(
        "{}\n{}",
        generate_semantic_tablegen_declarations(),
        generate_structural_tablegen_declarations()
    )
}

fn generate_structural_op_def(kind: StructuralOpKind) -> String {
    let op_class = kind.mlir_cpp_class();
    let mnemonic = kind.wire().strip_prefix("ais.").unwrap_or(kind.wire());
    format!(
        "def AIS_{op_class} : AIS_Op<\"{mnemonic}\", [NoTerminator]> {{\n  let summary = \"Compiler-emitted {kind} structural operation\";\n  let arguments = (ins Variadic<AIS_TokenType>:$operands);\n  let regions = (region AnyRegion:$body);\n  let results = (outs AIS_TokenType:$result);\n}}\n"
    )
}

fn with_single_trailing_newline(mut output: String) -> String {
    output.truncate(output.trim_end_matches('\n').len());
    output.push('\n');
    output
}

fn append_semantic_ops(output: &mut String) {
    let _ = write!(
        output,
        "\n//===----------------------------------------------------------------------===//\n\
         // Canonical Semantic Operations (generated from definitions.rs)\n\
         //===----------------------------------------------------------------------===//\n\n"
    );
    for spec in get_all_operations() {
        output.push_str(&generate_semantic_op_def(spec));
        output.push_str("\n\n");
    }
}

fn generate_semantic_op_def(spec: &OperationSpec) -> String {
    let op_class = spec.op_type.mlir_cpp_class();
    let op_name = op_class
        .strip_suffix("Op")
        .expect("MLIR class name ends in Op");
    let mnemonic = spec.op_type.wire().replace('.', "_");
    let arguments = render_operand_arguments(spec);
    let traits = if spec.fields.iter().filter(|field| !field.required).count() >= 2 {
        "AttrSizedOperandSegments"
    } else {
        ""
    };
    format!(
        "def AIS_{op_name}Op : AIS_Op<\"{mnemonic}\", [{traits}]> {{\n  let summary = \"{summary}\";\n{arguments}  let results = (outs AIS_TokenType:$result);\n}}\n",
        summary = spec.description.replace('"', "\\\"")
    )
}

/// Render the typed SSA operand list for one semantic operation. Each contract
/// field becomes a named AIS token operand; required fields are required
/// operands and optional fields use the corresponding optional operand form.
/// This makes the registered operation signature carry every value dependency
/// instead of serializing source value ids as untyped string attributes.
fn render_operand_arguments(spec: &OperationSpec) -> String {
    if spec.fields.is_empty() {
        return String::new();
    }
    let rendered: Vec<String> = spec
        .fields
        .iter()
        .map(|field| {
            if field.required {
                format!("AIS_TokenType:${}", field.name)
            } else {
                format!("Optional<AIS_TokenType>:${}", field.name)
            }
        })
        .collect();
    format!("  let arguments = (ins {});\n", rendered.join(", "))
}

#[cfg(test)]
mod tests {
    use super::{
        generate_semantic_tablegen_declarations, generate_structural_tablegen_declarations,
        generate_tablegen,
    };

    #[test]
    fn generated_declarations_expose_only_canonical_operations() {
        let declarations = generate_semantic_tablegen_declarations();
        for op in [
            "model_call",
            "capability_invoke",
            "program_new",
            "program_invoke",
            "await_event",
        ] {
            assert!(declarations.contains(op));
        }
        assert!(generate_tablegen().starts_with(&declarations));
        assert!(generate_structural_tablegen_declarations().contains("AIS_LoopOp"));
    }

    #[test]
    fn semantic_declarations_carry_typed_ssa_operands() {
        let declarations = generate_semantic_tablegen_declarations();
        assert!(declarations.contains("AIS_TokenType:$model_ref"));
        assert!(declarations.contains("AIS_TokenType:$request"));
        assert!(declarations.contains("Optional<AIS_TokenType>:$options"));
        assert!(declarations.contains("AIS_TokenType:$capability_ref"));
        assert!(declarations.contains("AIS_TokenType:$event_ref"));
        let structural = generate_structural_tablegen_declarations();
        assert!(structural.contains("Variadic<AIS_TokenType>:$operands"));
        assert!(structural.contains("region AnyRegion:$body"));
        assert!(structural.contains("[NoTerminator]"));
    }
}
