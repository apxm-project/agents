//! `apxm.air.v1` — closed consumer types and the AIR verifier.
//!
//! AIR exposes exactly five public semantic operations. Branch, loop, task,
//! try, yield, and return are compiler-owned structural IR; a NOP is transient
//! compiler machinery and is never serialized. Both closures are enforced at
//! the decode boundary by closed enums, so a structural op cannot appear as a
//! public semantic op and a NOP cannot be serialized.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Map;

use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::grammar::is_identifier;
use crate::source_map::SourceMap;

/// The single accepted `schema_version` for AIR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AirVersion {
    #[serde(rename = "apxm.air.v1")]
    V1,
}

/// The five — and only five — public semantic operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SemanticOpKind {
    #[serde(rename = "model.call")]
    ModelCall,
    #[serde(rename = "capability.invoke")]
    CapabilityInvoke,
    #[serde(rename = "program.new")]
    ProgramNew,
    #[serde(rename = "program.invoke")]
    ProgramInvoke,
    #[serde(rename = "await.event")]
    AwaitEvent,
}

/// The closed compiler-owned structural IR kinds. `nop` is intentionally absent:
/// it is transient machinery and never serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralKind {
    Function,
    Region,
    Block,
    Value,
    Branch,
    Switch,
    Loop,
    ParallelJoin,
    Try,
    Throw,
    Catch,
    Return,
    Yield,
}

/// One public semantic operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticOp {
    pub node_id: String,
    pub op: SemanticOpKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operands: Option<Map<String, serde_json::Value>>,
}

/// One structural IR region.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralNode {
    pub region_id: String,
    pub kind: StructuralKind,
}

/// A decoded canonical AIR module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirModule {
    pub schema_version: AirVersion,
    pub semantic_operations: Vec<SemanticOp>,
    pub structural_ir: Vec<StructuralNode>,
    pub source_map: SourceMap,
}

impl AirModule {
    /// Verify a decoded AIR module. The op and kind closures are already
    /// guaranteed by decode; this adds identifier grammar, unique-id, and
    /// source-map checks, producing deterministic closed diagnostics.
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();

        let mut seen_nodes: HashSet<&str> = HashSet::new();
        for op in &self.semantic_operations {
            if !is_identifier(&op.node_id) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::InvalidIdentifier,
                    op.node_id.clone(),
                    "semantic operation node_id is not a contract identifier",
                ));
            }
            if !seen_nodes.insert(op.node_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::DuplicateNodeId,
                    op.node_id.clone(),
                    "semantic operation node_id is not unique",
                ));
            }
        }

        let mut seen_regions: HashSet<&str> = HashSet::new();
        for region in &self.structural_ir {
            if !is_identifier(&region.region_id) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::InvalidIdentifier,
                    region.region_id.clone(),
                    "structural region region_id is not a contract identifier",
                ));
            }
            if !seen_regions.insert(region.region_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::DuplicateRegionId,
                    region.region_id.clone(),
                    "structural region region_id is not unique",
                ));
            }
        }

        self.source_map.collect(&mut verdict);
        verdict.finish()
    }
}

/// Verify AIR presented as JSON, failing closed on decode errors. An op or kind
/// outside the closed sets, an unknown field, or a serialized NOP is rejected
/// here.
#[must_use]
pub fn verify_air_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<AirModule>(value.clone()) {
        Ok(module) => module.verify(),
        Err(error) => schema_violation("air", &error),
    }
}
