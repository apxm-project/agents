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
use crate::source_map::{RegionAnnotationKind, SourceMap};

pub use apxm_ais::{SemanticOpKind, StructuralOpKind};

/// The single accepted `schema_version` for AIR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AirVersion {
    #[serde(rename = "apxm.air.v1")]
    V1,
}

/// One public semantic operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticOp {
    pub node_id: String,
    pub op: SemanticOpKind,
    pub parent_region_id: String,
    pub execution_order: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operands: Option<Map<String, serde_json::Value>>,
}

/// One structural IR region.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralNode {
    pub region_id: String,
    pub kind: StructuralOpKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_region_id: Option<String>,
    pub execution_order: u32,
}

/// One explicit typed Context edge retained in AIR.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextEdge {
    pub from_node: String,
    pub to_node: String,
    pub context_type_ref: String,
}

/// A decoded canonical AIR module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirModule {
    pub schema_version: AirVersion,
    pub semantic_operations: Vec<SemanticOp>,
    pub structural_ir: Vec<StructuralNode>,
    pub context_flow: Vec<ContextEdge>,
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

        collect_containment_diagnostics(
            &mut verdict,
            &self.semantic_operations,
            &self.structural_ir,
            &self.source_map,
        );
        self.source_map.collect(&mut verdict);
        verdict.finish()
    }
}

fn collect_containment_diagnostics(
    verdict: &mut Verdict,
    semantic_operations: &[SemanticOp],
    structural_ir: &[StructuralNode],
    source_map: &SourceMap,
) {
    let regions: HashSet<&str> = structural_ir
        .iter()
        .map(|region| region.region_id.as_str())
        .collect();
    let mut positions: HashSet<(Option<&str>, u32)> = HashSet::new();

    for region in structural_ir {
        if let Some(parent) = region.parent_region_id.as_deref()
            && !regions.contains(parent)
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "structural parent_region_id does not reference a declared region",
            ));
        }
        if !positions.insert((region.parent_region_id.as_deref(), region.execution_order)) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "structural siblings have duplicate execution_order",
            ));
        }

        let mut cursor = region.parent_region_id.as_deref();
        let mut ancestors = HashSet::new();
        ancestors.insert(region.region_id.as_str());
        while let Some(parent) = cursor {
            if !ancestors.insert(parent) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    region.region_id.clone(),
                    "structural containment contains a cycle",
                ));
                break;
            }
            cursor = structural_ir
                .iter()
                .find(|candidate| candidate.region_id == parent)
                .and_then(|candidate| candidate.parent_region_id.as_deref());
        }
    }

    for operation in semantic_operations {
        if !regions.contains(operation.parent_region_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                operation.node_id.clone(),
                "semantic parent_region_id does not reference a declared region",
            ));
        }
        if !positions.insert((
            Some(operation.parent_region_id.as_str()),
            operation.execution_order,
        )) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                operation.node_id.clone(),
                "structural and semantic siblings have duplicate execution_order",
            ));
        }
    }

    let loop_ids: HashSet<&str> = structural_ir
        .iter()
        .filter(|region| region.kind == StructuralOpKind::Loop)
        .map(|region| region.region_id.as_str())
        .collect();
    let mut annotated_loop_ids = HashSet::new();
    for annotation in &source_map.region_annotations {
        if annotation.annotation != RegionAnnotationKind::StructuralLoop {
            continue;
        }
        if !loop_ids.contains(annotation.region_id.as_str())
            || !annotated_loop_ids.insert(annotation.region_id.as_str())
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                annotation.region_id.clone(),
                "structural_loop annotation must identify exactly one declared loop",
            ));
        }
    }
    for loop_id in loop_ids {
        if !annotated_loop_ids.contains(loop_id) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                loop_id,
                "structural loop is missing its generic source-map identity",
            ));
        }
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
