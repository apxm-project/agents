//! Canonical five-operation AIR → deterministic MLIR lowering.
//!
//! The `apxm-program` verifier is the front gate: an AIR module is verified
//! before it is lowered, so a structural op posing as a public op, a serialized
//! NOP, a duplicate id, or any other rejected shape never reaches MLIR. The
//! emitter preserves input order and touches no map iteration, so the same AIR
//! always lowers to byte-identical MLIR text. The five public operations become
//! registered `ais.*` operations with real SSA operands/results, while the
//! compiler-owned structural family owns nested MLIR regions and block
//! arguments inside one `func.func`.

use std::collections::{BTreeMap, HashMap};

use apxm_ais::get_operation_spec;
use apxm_program::{AirModule, Verdict};

use crate::api::{Context, Module};
use apxm_core::error::compiler::{CompilerError, Result};

/// Why a canonical lowering did not produce verified MLIR.
#[derive(Debug)]
pub enum LoweringError {
    /// The AIR was rejected by the semantic verifier before lowering.
    Rejected(Verdict),
    /// The emitted MLIR failed to parse or verify against the real toolchain.
    Mlir(CompilerError),
}

impl std::fmt::Display for LoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(verdict) => {
                write!(
                    f,
                    "AIR rejected before lowering: {} diagnostic(s)",
                    verdict.diagnostics().len()
                )
            }
            Self::Mlir(error) => write!(f, "MLIR lowering failed: {error}"),
        }
    }
}

impl std::error::Error for LoweringError {}

/// Lower verified canonical AIR to deterministic MLIR text.
///
/// # Errors
///
/// Returns the rejecting [`Verdict`] when the AIR does not verify.
pub fn lower_air_to_mlir_text(air: &AirModule) -> std::result::Result<String, Verdict> {
    let verdict = air.verify();
    if !verdict.is_accepted() {
        return Err(verdict);
    }
    Ok(emit_module(air))
}

/// Parse and verify MLIR text through the real compiler context, returning the
/// context's canonical printed form.
///
/// # Errors
///
/// Returns a [`CompilerError`] when the text does not parse or verify.
pub fn verify_mlir_text(text: &str) -> Result<String> {
    let context = Context::new()?;
    let module = Module::parse(&context, text)?;
    module.verify()?;
    module.to_string()
}

/// Lower canonical AIR and prove the result is valid MLIR through the real
/// toolchain. Returns the emitted deterministic text on success.
///
/// # Errors
///
/// Returns [`LoweringError::Rejected`] if the AIR does not verify, or
/// [`LoweringError::Mlir`] if the emitted MLIR does not parse or verify.
pub fn lower_and_verify(air: &AirModule) -> std::result::Result<String, LoweringError> {
    let text = lower_air_to_mlir_text(air).map_err(LoweringError::Rejected)?;
    verify_mlir_text(&text).map_err(LoweringError::Mlir)?;
    Ok(text)
}

fn emit_module(air: &AirModule) -> String {
    let values = MlirValues::from_air(air);
    let mut out = String::new();
    out.push_str("module attributes {apxm.air = \"apxm.air.v1\", apxm.source_language = \"");
    out.push_str(air.source_map.source_language.wire());
    out.push_str("\"} {\n");
    out.push_str("  func.func @program(");
    for (index, (name, type_ref)) in values.entry_arguments.iter().enumerate() {
        if index != 0 {
            out.push_str(", ");
        }
        out.push_str(name);
        out.push_str(": ");
        emit_token_type(&mut out, type_ref);
    }
    out.push_str(") {\n");
    emit_children(&mut out, air, &values, None, 4);

    out.push_str("    func.return\n  }\n}\n");
    out
}

/// Stable MLIR SSA names assigned from AIR value ownership. External source
/// values are function arguments; semantic results and region block arguments
/// retain their AIR identities as SSA dependencies without materializing an
/// unregistered constant operation.
struct MlirValues {
    names: HashMap<String, String>,
    entry_arguments: Vec<(String, String)>,
}

impl MlirValues {
    fn from_air(air: &AirModule) -> Self {
        let mut names = HashMap::new();
        for (index, operation) in air.semantic_operations.iter().enumerate() {
            if let Some(result) = &operation.result {
                names.insert(result.value_id.clone(), format!("%sem{index}"));
            }
        }
        for (region_index, region) in air.structural_ir.iter().enumerate() {
            for (argument_index, argument) in region.block_arguments.iter().enumerate() {
                names.insert(
                    argument.value_id.clone(),
                    format!("%arg_{region_index}_{argument_index}"),
                );
            }
        }

        let mut external = BTreeMap::new();
        for operation in &air.semantic_operations {
            for operand in &operation.operands {
                if !names.contains_key(&operand.value_id) {
                    external.insert(operand.value_id.clone(), operand.type_ref.clone());
                }
            }
        }
        for region in &air.structural_ir {
            for operand in &region.operands {
                if !names.contains_key(&operand.value_id) {
                    external.insert(operand.value_id.clone(), operand.type_ref.clone());
                }
            }
        }
        let entry_arguments: Vec<(String, String)> = external
            .into_iter()
            .enumerate()
            .map(|(index, (value_id, type_ref))| {
                let name = format!("%input{index}");
                names.insert(value_id, name.clone());
                (name, type_ref)
            })
            .collect();
        Self {
            names,
            entry_arguments,
        }
    }

    fn operand_name(&self, value_id: &str) -> &str {
        self.names
            .get(value_id)
            .map(String::as_str)
            .expect("AIR verifier requires every operand to have an SSA definition")
    }
}

fn emit_children(
    out: &mut String,
    air: &AirModule,
    values: &MlirValues,
    parent_region_id: Option<&str>,
    indent: usize,
) {
    let mut children: Vec<Child<'_>> = air
        .structural_ir
        .iter()
        .enumerate()
        .filter(|(_, region)| region.parent_region_id.as_deref() == parent_region_id)
        .map(|(index, region)| Child::Structural(index, region.execution_order, &region.region_id))
        .chain(
            air.semantic_operations
                .iter()
                .enumerate()
                .filter(|(_, operation)| {
                    Some(operation.parent_region_id.as_str()) == parent_region_id
                })
                .map(|(index, operation)| {
                    Child::Semantic(index, operation.execution_order, &operation.node_id)
                }),
        )
        .collect();
    children.sort_by(|left, right| {
        left.order()
            .cmp(&right.order())
            .then(left.id().cmp(right.id()))
    });

    for child in children {
        match child {
            Child::Structural(index, _, _) => emit_structural(out, air, values, index, indent),
            Child::Semantic(index, _, _) => emit_semantic(out, air, values, index, indent),
        }
    }
}

enum Child<'a> {
    Structural(usize, u32, &'a str),
    Semantic(usize, u32, &'a str),
}

impl Child<'_> {
    fn order(&self) -> u32 {
        match self {
            Self::Structural(_, order, _) | Self::Semantic(_, order, _) => *order,
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::Structural(_, _, id) | Self::Semantic(_, _, id) => id,
        }
    }
}

fn emit_semantic(
    out: &mut String,
    air: &AirModule,
    values: &MlirValues,
    index: usize,
    indent: usize,
) {
    let operation = &air.semantic_operations[index];
    let spec = get_operation_spec(operation.op).expect("AIR operation has an AIS specification");
    let mut operands = Vec::new();
    for field in spec.fields {
        if let Some(operand) = operation
            .operands
            .iter()
            .find(|operand| operand.slot == field.name)
        {
            operands.push(operand);
        }
    }
    push_indent(out, indent);
    let result = operation
        .result
        .as_ref()
        .expect("AIR verifier requires semantic result");
    out.push_str(values.operand_name(&result.value_id));
    out.push_str(" = \"ais.");
    out.push_str(&operation.op.wire().replace('.', "_"));
    out.push_str("\"(");
    emit_operand_uses(out, values, operands.iter().copied());
    out.push_str(") {apxm.node_id = \"");
    push_escaped(out, &operation.node_id);
    out.push_str("\", apxm.parent_region_id = \"");
    push_escaped(out, &operation.parent_region_id);
    out.push_str("\", apxm.execution_order = ");
    out.push_str(&operation.execution_order.to_string());
    out.push_str(" : i64, apxm.result_value_id = \"");
    push_escaped(out, &result.value_id);
    out.push('"');
    if spec.fields.iter().filter(|field| !field.required).count() >= 2 {
        out.push_str(", operand_segment_sizes = array<i32: ");
        for (field_index, field) in spec.fields.iter().enumerate() {
            if field_index != 0 {
                out.push_str(", ");
            }
            let present = field.required
                || operation
                    .operands
                    .iter()
                    .any(|operand| operand.slot == field.name);
            out.push_str(if present { "1" } else { "0" });
        }
        out.push('>');
    }
    out.push_str("} : (");
    emit_token_types(out, operands.iter().copied());
    out.push_str(") -> ");
    emit_token_type(out, &result.type_ref);
    out.push('\n');
}

fn emit_structural(
    out: &mut String,
    air: &AirModule,
    values: &MlirValues,
    index: usize,
    indent: usize,
) {
    let region = &air.structural_ir[index];
    push_indent(out, indent);
    out.push_str("%structural");
    out.push_str(&index.to_string());
    out.push_str(" = \"ais.");
    out.push_str(
        region
            .kind
            .wire()
            .strip_prefix("ais.")
            .unwrap_or(region.kind.wire()),
    );
    out.push_str("\"(");
    emit_operand_uses(out, values, &region.operands);
    out.push_str(") ({\n");
    push_indent(out, indent + 2);
    out.push_str("^bb0");
    if !region.block_arguments.is_empty() {
        out.push('(');
        for (argument_index, argument) in region.block_arguments.iter().enumerate() {
            if argument_index != 0 {
                out.push_str(", ");
            }
            out.push_str(values.operand_name(&argument.value_id));
            out.push_str(": ");
            emit_token_type(out, &argument.type_ref);
        }
        out.push(')');
    }
    out.push_str(":\n");
    emit_children(out, air, values, Some(&region.region_id), indent + 4);
    push_indent(out, indent);
    out.push_str("}) {apxm.region_id = \"");
    push_escaped(out, &region.region_id);
    out.push('"');
    if let Some(parent_region_id) = &region.parent_region_id {
        out.push_str(", apxm.parent_region_id = \"");
        push_escaped(out, parent_region_id);
        out.push('"');
    }
    out.push_str(", apxm.execution_order = ");
    out.push_str(&region.execution_order.to_string());
    out.push_str(" : i64} : (");
    emit_token_types(out, region.operands.iter());
    out.push_str(") -> ");
    emit_token_type(out, "StructuralControl");
    out.push('\n');
}

fn emit_operand_uses<'a>(
    out: &mut String,
    values: &MlirValues,
    operands: impl IntoIterator<Item = &'a apxm_program::air::Operand>,
) {
    for (index, operand) in operands.into_iter().enumerate() {
        if index != 0 {
            out.push_str(", ");
        }
        out.push_str(values.operand_name(&operand.value_id));
    }
}

fn emit_token_types<'a>(
    out: &mut String,
    operands: impl IntoIterator<Item = &'a apxm_program::air::Operand>,
) {
    for (index, operand) in operands.into_iter().enumerate() {
        if index != 0 {
            out.push_str(", ");
        }
        emit_token_type(out, &operand.type_ref);
    }
}

fn emit_token_type(out: &mut String, type_ref: &str) {
    out.push_str("!ais.token<!ais.type_ref<\"");
    push_escaped(out, type_ref);
    out.push_str("\">>");
}

fn push_indent(out: &mut String, spaces: usize) {
    out.push_str(&" ".repeat(spaces));
}

fn push_escaped(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_program::verify_air_json;
    use serde_json::json;

    fn sample_air() -> serde_json::Value {
        json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "node.model.1", "op": "model.call", "parent_region_id": "region.loop.1", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.loop.input", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.out", "type_ref": "ModelOutput"}},
                {"node_id": "node.cap.1", "op": "capability.invoke", "parent_region_id": "region.loop.1", "execution_order": 1, "operands": [{"slot": "capability_ref", "value_id": "cap.search", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.args", "type_ref": "SearchRequest"}], "result": {"value_id": "value.cap.out", "type_ref": "SearchResult"}},
                {"node_id": "node.new.1", "op": "program.new", "parent_region_id": "region.loop.1", "execution_order": 2, "operands": [{"slot": "program_ref", "value_id": "child", "type_ref": "ProgramRef"}], "result": {"value_id": "value.child.instance", "type_ref": "ProgramInstanceRef"}},
                {"node_id": "node.invoke.1", "op": "program.invoke", "parent_region_id": "region.loop.1", "execution_order": 3, "operands": [{"slot": "receiver", "value_id": "value.child.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.in", "type_ref": "ChildInput"}], "result": {"value_id": "value.child.output", "type_ref": "ChildOutput"}},
                {"node_id": "node.await.1", "op": "await.event", "parent_region_id": "region.loop.1", "execution_order": 4, "operands": [{"slot": "event_ref", "value_id": "evt.ready", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}}
            ],
            "structural_ir": [
                {"region_id": "region.fn.1", "kind": "function", "execution_order": 0},
                {"region_id": "region.loop.1", "kind": "ais.loop", "parent_region_id": "region.fn.1", "execution_order": 0, "block_arguments": [{"value_id": "value.loop.input", "type_ref": "ModelRequest"}]},
                {"region_id": "region.return.1", "kind": "return", "parent_region_id": "region.fn.1", "execution_order": 1}
            ],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": [
                    {"region_id": "region.loop.1", "annotation": "structural_loop"}
                ]
            }
        })
    }

    fn decode(value: &serde_json::Value) -> AirModule {
        assert!(verify_air_json(value).is_accepted());
        serde_json::from_value(value.clone()).expect("decode AIR")
    }

    #[test]
    fn lowering_is_byte_deterministic() {
        let air = decode(&sample_air());
        let first = lower_air_to_mlir_text(&air).expect("lower");
        let second = lower_air_to_mlir_text(&air).expect("lower");
        assert_eq!(
            first, second,
            "canonical MLIR lowering is not byte-identical"
        );
        assert!(first.contains("apxm.parent_region_id = \"region.loop.1\""));
        assert!(first.contains("apxm.execution_order = 0 : i64"));
        assert!(first.contains("\"ais.loop\"() ({"));
        // Semantic operations emit as registered ais.* ops with exact token
        // payloads, including external source references and block arguments.
        assert!(first.contains("\"ais.model_call\"("));
        assert!(first.contains(
            "(!ais.token<!ais.type_ref<\"ModelTargetRef\">>, !ais.token<!ais.type_ref<\"ModelRequest\">>) -> !ais.token<!ais.type_ref<\"ModelOutput\">>"
        ));
        assert!(first.contains("^bb0(%arg_1_0: !ais.token<!ais.type_ref<\"ModelRequest\">>):"));
        assert!(!first.contains("apxm.result_type_ref"));
        assert!(!first.contains("apxm.operand_type_refs"));
        assert!(!first.contains("apxm.block_argument_type_refs"));
        assert!(!first.contains("\"apxm.model.call\""));
    }

    #[test]
    fn rejected_air_never_lowers() {
        // A structural op posing as a public op is rejected by the verifier and
        // must not reach MLIR.
        let bad = json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "node.dup", "op": "model.call", "parent_region_id": "region.root", "execution_order": 0},
                {"node_id": "node.dup", "op": "model.call", "parent_region_id": "region.root", "execution_order": 1}
            ],
            "structural_ir": [
                {"region_id": "region.root", "kind": "region", "execution_order": 0}
            ],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        });
        let air: AirModule = serde_json::from_value(bad).expect("decode");
        assert!(
            lower_air_to_mlir_text(&air).is_err(),
            "duplicate node id must be rejected"
        );
    }

    #[cfg(feature = "mlir")]
    #[test]
    fn lowered_mlir_parses_and_verifies() {
        let air = decode(&sample_air());
        let text = lower_and_verify(&air).expect("lower and verify through real MLIR");
        // The verified text is the deterministic emitter output.
        assert_eq!(text, lower_air_to_mlir_text(&air).unwrap());
        // The real toolchain re-prints the same module deterministically.
        let printed_once = verify_mlir_text(&text).expect("verify once");
        let printed_twice = verify_mlir_text(&text).expect("verify twice");
        assert_eq!(
            printed_once, printed_twice,
            "MLIR round-trip is not deterministic"
        );
        assert!(printed_once.contains("ais.model_call"));
    }

    #[cfg(feature = "mlir")]
    #[test]
    fn registered_model_call_missing_required_operand_fails_verification() {
        // A registered ais.model_call without its required model_ref/request
        // operands must fail MLIR verification, so a dropped operand cannot
        // reach an artifact. Unregistered dialects are disabled, so an unknown
        // operation also cannot pass.
        let missing_operand = "module {\n  func.func @program() {\n    \
             %s0 = \"ais.model_call\"() {apxm.node_id = \"n\"} : () -> !ais.token<!ais.type_ref<\"Output\">>\n    \
             func.return\n  }\n}\n";
        assert!(
            verify_mlir_text(missing_operand).is_err(),
            "model_call without required operands must fail verification"
        );

        let unregistered = "module {\n  func.func @program() {\n    \
             \"ais.not_a_real_op\"() : () -> ()\n    func.return\n  }\n}\n";
        assert!(
            verify_mlir_text(unregistered).is_err(),
            "an unregistered operation must fail with unregistered dialects disabled"
        );
    }

    #[cfg(feature = "mlir")]
    #[test]
    fn tokens_require_and_round_trip_dialect_owned_type_refs() {
        let typed = "module {\n  func.func @program(%input: !ais.token<!ais.type_ref<\"Input\">>) {\n    func.return\n  }\n}\n";
        let printed = verify_mlir_text(typed).expect("typed token must parse and verify");
        assert!(printed.contains("!ais.token<!ais.type_ref<\"Input\">>"));

        let bare_token =
            "module {\n  func.func @program(%input: !ais.token) {\n    func.return\n  }\n}\n";
        assert!(
            verify_mlir_text(bare_token).is_err(),
            "a token without a payload must be rejected"
        );

        let builtin_payload =
            "module {\n  func.func @program(%input: !ais.token<i64>) {\n    func.return\n  }\n}\n";
        assert!(
            verify_mlir_text(builtin_payload).is_err(),
            "a token payload must be an AIS TypeRefType"
        );
    }
}
