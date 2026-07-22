//! Canonical five-operation AIR → deterministic MLIR lowering.
//!
//! The `apxm-program` verifier is the front gate: an AIR module is verified
//! before it is lowered, so a structural op posing as a public op, a serialized
//! NOP, a duplicate id, or any other rejected shape never reaches MLIR. The
//! emitter preserves input order and touches no map iteration, so the same AIR
//! always lowers to byte-identical MLIR text. The five public operations become
//! `apxm.<op>` operations and the compiler-owned structural regions become
//! `apxm.structural` operations inside one `func.func`.

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
    let mut out = String::new();
    out.push_str("module attributes {apxm.air = \"apxm.air.v1\", apxm.source_language = \"");
    out.push_str(air.source_map.source_language.wire());
    out.push_str("\"} {\n");
    out.push_str("  func.func @program() {\n");

    for op in &air.semantic_operations {
        out.push_str("    \"apxm.");
        out.push_str(op.op.wire());
        out.push_str("\"() {apxm.node_id = \"");
        push_escaped(&mut out, &op.node_id);
        out.push_str("\", apxm.parent_region_id = \"");
        push_escaped(&mut out, &op.parent_region_id);
        out.push_str("\", apxm.execution_order = ");
        out.push_str(&op.execution_order.to_string());
        out.push_str(" : i64} : () -> ()\n");
    }

    for (index, region) in air.structural_ir.iter().enumerate() {
        out.push_str("    %structural");
        out.push_str(&index.to_string());
        out.push_str(" = \"ais.");
        out.push_str(
            region
                .kind
                .wire()
                .strip_prefix("ais.")
                .unwrap_or(region.kind.wire()),
        );
        out.push_str("\"() {apxm.region_id = \"");
        push_escaped(&mut out, &region.region_id);
        out.push('"');
        if let Some(parent_region_id) = &region.parent_region_id {
            out.push_str(", apxm.parent_region_id = \"");
            push_escaped(&mut out, parent_region_id);
            out.push('"');
        }
        out.push_str(", apxm.execution_order = ");
        out.push_str(&region.execution_order.to_string());
        out.push_str(" : i64} : () -> !ais.token\n");
    }

    out.push_str("    func.return\n  }\n}\n");
    out
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
                {"node_id": "node.model.1", "op": "model.call", "parent_region_id": "region.loop.1", "execution_order": 0},
                {"node_id": "node.cap.1", "op": "capability.invoke", "parent_region_id": "region.loop.1", "execution_order": 1},
                {"node_id": "node.new.1", "op": "program.new", "parent_region_id": "region.loop.1", "execution_order": 2},
                {"node_id": "node.invoke.1", "op": "program.invoke", "parent_region_id": "region.loop.1", "execution_order": 3},
                {"node_id": "node.await.1", "op": "await.event", "parent_region_id": "region.loop.1", "execution_order": 4}
            ],
            "structural_ir": [
                {"region_id": "region.fn.1", "kind": "function", "execution_order": 0},
                {"region_id": "region.loop.1", "kind": "ais.loop", "parent_region_id": "region.fn.1", "execution_order": 0},
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
        assert!(first.contains("\"ais.loop\"()"));
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
        assert!(printed_once.contains("apxm.model.call"));
    }
}
