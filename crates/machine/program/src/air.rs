//! `apxm.air.v1` — closed consumer types and the AIR verifier.
//!
//! AIR exposes exactly five public semantic operations. Branch, loop, task,
//! try, yield, and return are compiler-owned structural IR; a NOP is transient
//! compiler machinery and is never serialized. Both closures are enforced at
//! the decode boundary by closed enums, so a structural op cannot appear as a
//! public semantic op and a NOP cannot be serialized.

use std::collections::HashSet;

use apxm_ais::get_operation_spec;
use serde::{Deserialize, Serialize};

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

/// One typed SSA operand: a named input slot bound to a produced value id and its
/// type. The untyped operand bag is retired; every operand names its slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operand {
    pub slot: String,
    pub value_id: String,
    pub type_ref: String,
}

/// One typed SSA value produced by an operation or carried as a block argument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SsaValue {
    pub value_id: String,
    pub type_ref: String,
}

/// Closed typed comparison for runtime structural control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateComparator {
    Truthy,
    Equals,
    NotEquals,
}

/// Closed scalar literal for runtime structural comparison.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scalar_type", content = "value", rename_all = "snake_case")]
pub enum PredicateLiteral {
    Boolean(bool),
    String(String),
    Integer(i64),
    Null,
}

/// Typed runtime predicate over an SSA result and a bounded property path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPredicate {
    pub root_value_id: String,
    pub property_path: Vec<String>,
    pub comparator: PredicateComparator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub literal: Option<PredicateLiteral>,
}

/// One public semantic operation with typed SSA operands and an optional result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticOp {
    pub node_id: String,
    pub op: SemanticOpKind,
    pub parent_region_id: String,
    pub execution_order: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operands: Vec<Operand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SsaValue>,
}

/// One structural IR node carrying typed block arguments and typed SSA operands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralNode {
    pub region_id: String,
    pub kind: StructuralOpKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_region_id: Option<String>,
    pub execution_order: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub block_arguments: Vec<SsaValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operands: Vec<Operand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<ControlPredicate>,
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
        let mut seen_value_definitions: HashSet<&str> = HashSet::new();
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
            for operand in &op.operands {
                check_operand(&mut verdict, operand, &op.node_id);
            }
            validate_semantic_signature(&mut verdict, op);
            if let Some(result) = &op.result {
                check_value(
                    &mut verdict,
                    &result.value_id,
                    &result.type_ref,
                    &op.node_id,
                );
                if !seen_value_definitions.insert(result.value_id.as_str()) {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        result.value_id.clone(),
                        "SSA result value_id is defined more than once",
                    ));
                }
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
            for argument in &region.block_arguments {
                check_value(
                    &mut verdict,
                    &argument.value_id,
                    &argument.type_ref,
                    &region.region_id,
                );
                if !seen_value_definitions.insert(argument.value_id.as_str()) {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        argument.value_id.clone(),
                        "SSA block argument value_id is defined more than once",
                    ));
                }
            }
            for operand in &region.operands {
                check_operand(&mut verdict, operand, &region.region_id);
            }
            validate_control_predicate(&mut verdict, region);
        }
        for region in &self.structural_ir {
            if let Some(predicate) = &region.predicate
                && !seen_value_definitions.contains(predicate.root_value_id.as_str())
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    region.region_id.clone(),
                    "predicate root_value_id does not reference a defined SSA value",
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

fn validate_control_predicate(verdict: &mut Verdict, region: &StructuralNode) {
    let Some(predicate) = &region.predicate else {
        if matches!(
            region.kind,
            StructuralOpKind::Branch | StructuralOpKind::Switch
        ) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "branch and switch nodes require a typed predicate",
            ));
        }
        return;
    };
    if !matches!(
        region.kind,
        StructuralOpKind::Branch | StructuralOpKind::Switch | StructuralOpKind::Loop
    ) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            region.region_id.clone(),
            "only branch, switch, and loop nodes carry predicates",
        ));
    }
    if !is_identifier(&predicate.root_value_id) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidIdentifier,
            region.region_id.clone(),
            "predicate root_value_id is not a contract identifier",
        ));
    }
    if predicate.property_path.len() > 16
        || predicate.property_path.iter().any(|segment| {
            segment.is_empty()
                || segment.len() > 128
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
    {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            region.region_id.clone(),
            "predicate property_path has an empty, hostile, or overlong segment",
        ));
    }
    match (predicate.comparator, predicate.literal.as_ref()) {
        (PredicateComparator::Truthy, None)
        | (PredicateComparator::Equals | PredicateComparator::NotEquals, Some(_)) => {}
        (PredicateComparator::Truthy, Some(_)) => verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            region.region_id.clone(),
            "truthy predicate does not accept a literal",
        )),
        (PredicateComparator::Equals | PredicateComparator::NotEquals, None) => {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "equals predicate requires a typed scalar literal",
            ))
        }
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

/// Check one typed SSA operand's slot, value id, and type ref grammar.
fn check_operand(verdict: &mut Verdict, operand: &Operand, owner: &str) {
    if !is_identifier(&operand.slot) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidIdentifier,
            owner.to_string(),
            "operand slot is not a contract identifier",
        ));
    }
    check_value(verdict, &operand.value_id, &operand.type_ref, owner);
}

/// Verify that an AIR semantic operation exactly instantiates the field
/// signature owned by the AIS catalogue. The FrontendGraph selects source
/// intents, but once lowering has selected an AIS operation its required
/// operands and result cannot be dropped or replaced by an arbitrary slot.
fn validate_semantic_signature(verdict: &mut Verdict, operation: &SemanticOp) {
    let Some(spec) = get_operation_spec(operation.op) else {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            operation.node_id.clone(),
            "semantic operation is absent from the AIS catalogue",
        ));
        return;
    };

    let mut seen_slots = HashSet::new();
    for operand in &operation.operands {
        if !seen_slots.insert(operand.slot.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                operation.node_id.clone(),
                format!("semantic operand slot '{}' is duplicated", operand.slot),
            ));
        }
        if spec.get_field(&operand.slot).is_none() {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                operation.node_id.clone(),
                format!(
                    "semantic operand slot '{}' is not declared by {}",
                    operand.slot,
                    operation.op.wire()
                ),
            ));
        }
    }

    for field in spec.required_fields() {
        if !seen_slots.contains(field.name) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                operation.node_id.clone(),
                format!(
                    "semantic operation {} is missing required '{}' operand",
                    operation.op.wire(),
                    field.name
                ),
            ));
        }
    }

    if spec.produces_output && operation.result.is_none() {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            operation.node_id.clone(),
            format!(
                "semantic operation {} is missing its typed result",
                operation.op.wire()
            ),
        ));
    }
}

/// Check a typed value's id and type ref against the contract identifier grammar.
fn check_value(verdict: &mut Verdict, value_id: &str, type_ref: &str, owner: &str) {
    if !is_identifier(value_id) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidIdentifier,
            owner.to_string(),
            "operand value_id is not a contract identifier",
        ));
    }
    if !is_identifier(type_ref) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidIdentifier,
            owner.to_string(),
            "operand type_ref is not a contract identifier",
        ));
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
