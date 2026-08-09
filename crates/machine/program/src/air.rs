//! `apxm.air.v1` — closed consumer types and the AIR verifier.
//!
//! AIR exposes exactly five public semantic operations. Branch, loop, task,
//! try, yield, and return are compiler-owned structural IR; a NOP is transient
//! compiler machinery and is never serialized. Both closures are enforced at
//! the decode boundary by closed enums, so a structural op cannot appear as a
//! public semantic op and a NOP cannot be serialized.

use std::collections::HashSet;

const MAX_SAFE_PREDICATE_INTEGER: i64 = 9_007_199_254_740_991;

use apxm_ais::get_operation_spec;
use serde::{Deserialize, Serialize};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::frontend_graph::ValueExpression;
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

/// A pure authored request/argument value assembled from prior SSA values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueAssembly {
    pub value_id: String,
    pub expression: ValueExpression,
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
    pub value_id: String,
}

/// A decoded canonical AIR module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirModule {
    pub schema_version: AirVersion,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub value_assemblies: Vec<ValueAssembly>,
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
        for assembly in &self.value_assemblies {
            if !is_identifier(&assembly.value_id)
                || !seen_value_definitions.insert(assembly.value_id.as_str())
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    assembly.value_id.clone(),
                    "value assembly value_id must be a unique contract identifier",
                ));
            }
            if !validate_value_expression(&assembly.expression, &assembly.value_id, 0) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    assembly.value_id.clone(),
                    "value assembly expression is malformed, unsafe, or exceeds bounds",
                ));
            }
        }
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
            validate_loop_signature(&mut verdict, region);
        }
        for assembly in &self.value_assemblies {
            let mut references = Vec::new();
            collect_expression_references(&assembly.expression, &mut references);
            for dependency in references {
                if dependency == assembly.value_id
                    || !seen_value_definitions.contains(dependency.as_str())
                {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        assembly.value_id.clone(),
                        "value assembly expression does not reference a distinct defined SSA value",
                    ));
                }
            }
        }
        for edge in &self.context_flow {
            if !(seen_nodes.contains(edge.from_node.as_str())
                || seen_regions.contains(edge.from_node.as_str()))
                || !(seen_nodes.contains(edge.to_node.as_str())
                    || seen_regions.contains(edge.to_node.as_str()))
                || !seen_value_definitions.contains(edge.value_id.as_str())
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    edge.to_node.clone(),
                    "Context edge requires declared endpoints and an exact assembled value",
                ));
            }
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
        validate_air_ssa_dominance(&mut verdict, self);
        self.source_map.collect(&mut verdict);
        verdict.finish()
    }
}

#[derive(Clone)]
struct AirSsaLocation {
    region_id: String,
    execution_order: u32,
    entry: bool,
}

/// Validate authored Tool arguments against AIR def-use order as a second
/// closed boundary after FrontendGraph verification. This prevents a decoded
/// AIR payload from smuggling a future or sibling-branch SSA value into a
/// capability request, even when the value id is globally declared.
fn validate_air_ssa_dominance(verdict: &mut Verdict, air: &AirModule) {
    let regions: std::collections::HashMap<&str, &StructuralNode> = air
        .structural_ir
        .iter()
        .map(|region| (region.region_id.as_str(), region))
        .collect();
    let blocks: std::collections::HashMap<&str, AirSsaLocation> = air
        .structural_ir
        .iter()
        .flat_map(|region| {
            region.block_arguments.iter().map(move |argument| {
                (
                    argument.value_id.as_str(),
                    AirSsaLocation {
                        region_id: region.region_id.clone(),
                        execution_order: 0,
                        entry: true,
                    },
                )
            })
        })
        .collect();
    let results: std::collections::HashMap<&str, AirSsaLocation> = air
        .semantic_operations
        .iter()
        .filter_map(|operation| {
            operation.result.as_ref().map(|result| {
                (
                    result.value_id.as_str(),
                    AirSsaLocation {
                        region_id: operation.parent_region_id.clone(),
                        execution_order: operation.execution_order,
                        entry: false,
                    },
                )
            })
        })
        .collect();
    let assemblies: std::collections::HashMap<&str, &ValueExpression> = air
        .value_assemblies
        .iter()
        .map(|assembly| (assembly.value_id.as_str(), &assembly.expression))
        .collect();

    for operation in &air.semantic_operations {
        if operation.op != SemanticOpKind::CapabilityInvoke {
            continue;
        }
        let Some(arguments) = operation
            .operands
            .iter()
            .find(|operand| operand.slot == "arguments")
        else {
            continue;
        };
        let use_location = AirSsaLocation {
            region_id: operation.parent_region_id.clone(),
            execution_order: operation.execution_order,
            entry: false,
        };
        let mut visiting = HashSet::new();
        if !air_value_dominates(
            &arguments.value_id,
            &use_location,
            &results,
            &blocks,
            &assemblies,
            &regions,
            &mut visiting,
        ) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                operation.node_id.clone(),
                format!(
                    "authored capability argument '{}' does not dominate its effect site",
                    arguments.value_id
                ),
            ));
        }
    }
}

fn air_value_dominates(
    value_id: &str,
    use_location: &AirSsaLocation,
    results: &std::collections::HashMap<&str, AirSsaLocation>,
    blocks: &std::collections::HashMap<&str, AirSsaLocation>,
    assemblies: &std::collections::HashMap<&str, &ValueExpression>,
    regions: &std::collections::HashMap<&str, &StructuralNode>,
    visiting: &mut HashSet<String>,
) -> bool {
    if !visiting.insert(value_id.to_string()) {
        return false;
    }
    let location = results.get(value_id).or_else(|| blocks.get(value_id));
    let base_ok =
        location.is_none_or(|definition| air_dominates_location(definition, use_location, regions));
    if !base_ok {
        return false;
    }
    let expression_ok = assemblies.get(value_id).is_none_or(|expression| {
        let mut references = Vec::new();
        collect_expression_references(expression, &mut references);
        references.into_iter().all(|dependency| {
            air_value_dominates(
                &dependency,
                use_location,
                results,
                blocks,
                assemblies,
                regions,
                visiting,
            )
        })
    });
    visiting.remove(value_id);
    expression_ok
}

fn air_dominates_location(
    definition: &AirSsaLocation,
    use_location: &AirSsaLocation,
    regions: &std::collections::HashMap<&str, &StructuralNode>,
) -> bool {
    if definition.region_id == use_location.region_id {
        return definition.entry || definition.execution_order < use_location.execution_order;
    }
    let mut child = use_location.region_id.as_str();
    while let Some(region) = regions.get(child) {
        let Some(parent) = region.parent_region_id.as_deref() else {
            break;
        };
        if parent == definition.region_id {
            return definition.entry || definition.execution_order < region.execution_order;
        }
        child = parent;
    }
    false
}

fn valid_property_path(path: &[String]) -> bool {
    path.len() <= 16
        && path.iter().all(|segment| {
            !segment.is_empty()
                && segment.len() <= 128
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

fn validate_value_expression(expression: &ValueExpression, owner: &str, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    match expression {
        ValueExpression::Ssa { value_id } => is_identifier(value_id) && value_id != owner,
        ValueExpression::Context { property_path } => valid_property_path(property_path),
        ValueExpression::Projection {
            root,
            property_path,
        } => {
            !property_path.is_empty()
                && valid_property_path(property_path)
                && validate_value_expression(root, owner, depth + 1)
        }
        ValueExpression::Object { fields } => {
            fields.len() <= 64
                && fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<HashSet<_>>()
                    .len()
                    == fields.len()
                && fields.iter().all(|field| {
                    !field.name.is_empty()
                        && field.name.len() <= 128
                        && field
                            .name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                        && validate_value_expression(&field.value, owner, depth + 1)
                })
        }
        ValueExpression::Array { items } => {
            items.len() <= 256
                && items
                    .iter()
                    .all(|item| validate_value_expression(item, owner, depth + 1))
        }
        ValueExpression::Integer { value } => {
            value.unsigned_abs() <= MAX_SAFE_PREDICATE_INTEGER as u64
        }
        ValueExpression::String { value } => value.len() <= 1_048_576,
        ValueExpression::Boolean { .. } | ValueExpression::Null => true,
    }
}

fn collect_expression_references(expression: &ValueExpression, out: &mut Vec<String>) {
    match expression {
        ValueExpression::Ssa { value_id } => out.push(value_id.clone()),
        ValueExpression::Projection { root, .. } => collect_expression_references(root, out),
        ValueExpression::Object { fields } => fields
            .iter()
            .for_each(|field| collect_expression_references(&field.value, out)),
        ValueExpression::Array { items } => items
            .iter()
            .for_each(|item| collect_expression_references(item, out)),
        ValueExpression::Context { .. }
        | ValueExpression::String { .. }
        | ValueExpression::Integer { .. }
        | ValueExpression::Boolean { .. }
        | ValueExpression::Null => {}
    }
}

fn validate_loop_signature(verdict: &mut Verdict, region: &StructuralNode) {
    if region.kind != StructuralOpKind::Loop {
        return;
    }
    let initial: Vec<_> = region
        .operands
        .iter()
        .filter(|operand| operand.slot == "initial")
        .collect();
    let carried: Vec<_> = region
        .operands
        .iter()
        .filter(|operand| operand.slot == "carried")
        .collect();
    let only_phi_slots = region
        .operands
        .iter()
        .all(|operand| operand.slot == "initial" || operand.slot == "carried");
    let signature_matches = only_phi_slots
        && initial.len() == region.block_arguments.len()
        && carried.len() == region.block_arguments.len()
        && region
            .block_arguments
            .iter()
            .zip(initial.iter().zip(carried.iter()))
            .all(|(argument, (initial, carried))| {
                argument.type_ref == initial.type_ref && argument.type_ref == carried.type_ref
            });
    if !signature_matches {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            region.region_id.clone(),
            "loop block arguments require exact type-matched initial/carried operand pairs",
        ));
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
    if let Some(PredicateLiteral::Integer(value)) = predicate.literal
        && !(-MAX_SAFE_PREDICATE_INTEGER..=MAX_SAFE_PREDICATE_INTEGER).contains(&value)
    {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            region.region_id.clone(),
            "integer predicate literal exceeds the shared safe-integer domain",
        ));
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
