//! `apxm.air` — closed consumer types and the AIR verifier.
//!
//! AIR exposes exactly five public semantic operations. Branch, loop, task,
//! try, yield, and return are compiler-owned structural IR; a NOP is transient
//! compiler machinery and is never serialized. Both closures are enforced at
//! the decode boundary by closed enums, so a structural op cannot appear as a
//! public semantic op and a NOP cannot be serialized.

use std::collections::HashSet;

const MAX_SAFE_PREDICATE_INTEGER: i64 = 9_007_199_254_740_991;

use apxm_ais::permissions::LayerDecisions;
use apxm_ais::{SLOT_CAPABILITY_REF, SLOT_CARRIED, SLOT_INITIAL, get_operation_spec};
use serde::{Deserialize, Serialize};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::frontend_graph::{HookBinding, ValueExpression};
use crate::grammar::is_identifier;
use crate::source_map::{RegionAnnotationKind, SourceMap};

pub use apxm_ais::{SemanticOpKind, StructuralOpKind};

/// The single accepted `schema_version` for AIR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AirVersion {
    #[serde(rename = "apxm.air")]
    V2,
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
    /// Present only on the `region` node holding one Hook's captured body.
    ///
    /// Lowering used to keep four of a binding's fields and let the artifact
    /// re-attach the rest from the graph, which left an artifact's Hooks and
    /// its AIR as two copies nothing reconciled. Carrying the binding on the
    /// node it describes makes them one fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<HookBinding>,
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
    /// The permission the program's own source requested for a Capability it
    /// invokes, keyed by `capability_ref`.
    ///
    /// AIR already says *which* Capabilities a program invokes — that is what a
    /// `capability.invoke` node is. What it could not say is what the author
    /// asked for when invoking one, so every composition root handed bare AIR
    /// had to assume the widest possible request. This carries the authored
    /// request across that boundary, and only the authored request: a
    /// `capability_ref` absent here is one whose declaration stated no
    /// permission, which is an unqualified ask for the capability, not a
    /// silently narrowed one.
    ///
    /// It is a *request*, never a grant. The resolution stack in
    /// [`apxm_ais::permissions`] takes it as the code layer, and every layer
    /// above may only narrow it.
    #[serde(default, skip_serializing_if = "LayerDecisions::is_empty")]
    pub capability_permission_requests: LayerDecisions,
    pub source_map: SourceMap,
}

impl AirModule {
    /// Every `capability_ref` this module's `capability.invoke` operations
    /// name, deduplicated and in reference order.
    ///
    /// This is the set of Capabilities the program asks to invoke, which is the
    /// keyspace of [`Self::capability_permission_requests`] and of the code
    /// layer any composition root resolves from bare AIR.
    #[must_use]
    pub fn invoked_capability_refs(&self) -> std::collections::BTreeSet<&str> {
        self.semantic_operations
            .iter()
            .filter(|operation| operation.op == SemanticOpKind::CapabilityInvoke)
            .filter_map(|operation| {
                operation
                    .operands
                    .iter()
                    .find(|operand| operand.slot == SLOT_CAPABILITY_REF)
                    .map(|operand| operand.value_id.as_str())
            })
            .collect()
    }

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
        validate_assembly_cycles(&mut verdict, &self.value_assemblies);
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

        validate_capability_permission_requests(&mut verdict, self);

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
            validate_hook_node(&mut verdict, region);
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
        let context_endpoint_locations = self
            .semantic_operations
            .iter()
            .map(|operation| {
                (
                    operation.node_id.as_str(),
                    AirSsaLocation {
                        region_id: operation.parent_region_id.clone(),
                        execution_order: operation.execution_order,
                        entry: false,
                    },
                )
            })
            .chain(self.structural_ir.iter().map(|region| {
                (
                    region.region_id.as_str(),
                    AirSsaLocation {
                        region_id: region
                            .parent_region_id
                            .clone()
                            .unwrap_or_else(|| region.region_id.clone()),
                        execution_order: region.execution_order,
                        entry: region.parent_region_id.is_none(),
                    },
                )
            }))
            .collect::<std::collections::HashMap<_, _>>();
        let context_regions = self
            .structural_ir
            .iter()
            .map(|region| (region.region_id.as_str(), region))
            .collect::<std::collections::HashMap<_, _>>();
        for edge in &self.context_flow {
            let ordered = context_endpoint_locations
                .get(edge.from_node.as_str())
                .zip(context_endpoint_locations.get(edge.to_node.as_str()))
                .is_some_and(|(from, to)| {
                    (from.region_id != to.region_id
                        || from.execution_order != to.execution_order
                        || from.entry != to.entry)
                        && air_dominates_location(from, to, &context_regions)
                });
            if !(seen_nodes.contains(edge.from_node.as_str())
                || seen_regions.contains(edge.from_node.as_str()))
                || !(seen_nodes.contains(edge.to_node.as_str())
                    || seen_regions.contains(edge.to_node.as_str()))
                || !seen_value_definitions.contains(edge.value_id.as_str())
                || !is_identifier(&edge.context_type_ref)
                || !ordered
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    edge.to_node.clone(),
                    "Context edge requires a typed context coordinate, declared endpoints, and source-before-destination ordering",
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

struct AirSsaValidationContext<'a> {
    results: &'a std::collections::HashMap<&'a str, AirSsaLocation>,
    blocks: &'a std::collections::HashMap<&'a str, AirSsaLocation>,
    assemblies: &'a std::collections::HashMap<&'a str, &'a ValueExpression>,
    regions: &'a std::collections::HashMap<&'a str, &'a StructuralNode>,
    resume_values: &'a HashSet<&'a str>,
}

/// Validate authored executable-invocation operands against AIR def-use order
/// as a second closed boundary after FrontendGraph verification. This prevents
/// a decoded AIR payload from smuggling a future or sibling-branch SSA value
/// into a model, capability, or program request, even when the value id is
/// globally declared.
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
    let resume_values: HashSet<&str> = air
        .structural_ir
        .iter()
        .filter(|region| region.kind == StructuralOpKind::Yield)
        .flat_map(|region| {
            region
                .block_arguments
                .iter()
                .map(|value| value.value_id.as_str())
        })
        .collect();
    let context = AirSsaValidationContext {
        results: &results,
        blocks: &blocks,
        assemblies: &assemblies,
        regions: &regions,
        resume_values: &resume_values,
    };

    for operation in &air.semantic_operations {
        let use_location = AirSsaLocation {
            region_id: operation.parent_region_id.clone(),
            execution_order: operation.execution_order,
            entry: false,
        };
        for operand in operation
            .operands
            .iter()
            .filter(|operand| is_authored_invocation_operand(operation.op, operand))
        {
            if operation.op == SemanticOpKind::CapabilityInvoke
                && resume_values.contains(operand.value_id.as_str())
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    operation.node_id.clone(),
                    format!(
                        "resume input '{}' cannot become an authored capability argument",
                        operand.value_id
                    ),
                ));
                continue;
            }
            let mut visiting = HashSet::new();
            if !air_value_dominates(
                &operand.value_id,
                &use_location,
                operation.op == SemanticOpKind::ModelCall,
                &context,
                &mut visiting,
            ) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    operation.node_id.clone(),
                    format!(
                        "authored invocation operand '{}' does not dominate its effect site",
                        operand.value_id
                    ),
                ));
            }
        }
    }
}

fn is_authored_invocation_operand(op: SemanticOpKind, operand: &Operand) -> bool {
    match op {
        SemanticOpKind::ModelCall => operand.slot != "model_ref",
        SemanticOpKind::CapabilityInvoke => operand.slot != "capability_ref",
        SemanticOpKind::ProgramNew => operand.slot != "program_ref",
        SemanticOpKind::ProgramInvoke => operand.slot != "receiver",
        SemanticOpKind::AwaitEvent => operand.slot != "event_ref",
    }
}

fn air_value_dominates(
    value_id: &str,
    use_location: &AirSsaLocation,
    allow_resume_value: bool,
    context: &AirSsaValidationContext<'_>,
    visiting: &mut HashSet<String>,
) -> bool {
    if !visiting.insert(value_id.to_string()) {
        return false;
    }
    let location = context
        .results
        .get(value_id)
        .or_else(|| context.blocks.get(value_id));
    // AIR operands may be supplied as invocation-entry values by the runtime
    // host when no AIR definition or authored assembly claims the id. Those
    // external values are entry-dominating by construction; any value that is
    // declared as a result or block argument still goes through the strict
    // lexical dominance check above.
    let base_ok = if context.resume_values.contains(value_id) {
        allow_resume_value
    } else {
        match location {
            Some(definition) => air_dominates_location(definition, use_location, context.regions),
            None => {
                context.assemblies.contains_key(value_id)
                    || (!context.results.contains_key(value_id)
                        && !context.blocks.contains_key(value_id))
            }
        }
    };
    if !base_ok {
        return false;
    }
    let expression_ok = context.assemblies.get(value_id).is_none_or(|expression| {
        let mut references = Vec::new();
        collect_expression_references(expression, &mut references);
        references.into_iter().all(|dependency| {
            air_value_dominates(
                &dependency,
                use_location,
                allow_resume_value,
                context,
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
    if is_air_ancestor(&definition.region_id, &use_location.region_id, regions) {
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
        return definition.entry;
    }
    if is_air_ancestor(&use_location.region_id, &definition.region_id, regions) {
        let mut child = definition.region_id.as_str();
        while let Some(region) = regions.get(child) {
            let Some(parent) = region.parent_region_id.as_deref() else {
                break;
            };
            if parent == use_location.region_id {
                return region.execution_order < use_location.execution_order;
            }
            child = parent;
        }
    }
    false
}

fn is_air_ancestor(
    ancestor: &str,
    descendant: &str,
    regions: &std::collections::HashMap<&str, &StructuralNode>,
) -> bool {
    let mut cursor = Some(descendant);
    while let Some(region_id) = cursor {
        if region_id == ancestor {
            return true;
        }
        cursor = regions
            .get(region_id)
            .and_then(|region| region.parent_region_id.as_deref());
    }
    false
}

fn validate_assembly_cycles(verdict: &mut Verdict, assemblies: &[ValueAssembly]) {
    let by_id = assemblies
        .iter()
        .map(|assembly| (assembly.value_id.as_str(), &assembly.expression))
        .collect::<std::collections::HashMap<_, _>>();
    let mut state = std::collections::HashMap::<&str, u8>::new();
    for value_id in by_id.keys().copied() {
        if assembly_cycle(value_id, &by_id, &mut state) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                value_id.to_string(),
                "value assembly expressions contain a cycle",
            ));
        }
    }
}

fn assembly_cycle<'a>(
    value_id: &'a str,
    by_id: &std::collections::HashMap<&'a str, &'a ValueExpression>,
    state: &mut std::collections::HashMap<&'a str, u8>,
) -> bool {
    match state.get(value_id).copied() {
        Some(1) => return true,
        Some(2) => return false,
        _ => {}
    }
    state.insert(value_id, 1);
    let cycle = by_id.get(value_id).is_some_and(|expression| {
        let mut references = Vec::new();
        collect_expression_references(expression, &mut references);
        references.into_iter().any(|reference| {
            by_id
                .keys()
                .copied()
                .find(|key| *key == reference)
                .is_some_and(|key| assembly_cycle(key, by_id, state))
        })
    });
    state.insert(value_id, 2);
    cycle
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

/// A permission request may only be stated for a Capability this module
/// actually invokes, and it must say something when it says why.
///
/// Both halves close the same hole from opposite sides. A request naming a
/// `capability_ref` no `capability.invoke` names would sit in the code layer of
/// every resolution forever, narrowing an effect that does not exist and
/// looking, to a reader, like authority the program holds. An empty reason
/// would put a blank explanation into the digest — the same check the
/// FrontendGraph already runs on the authored requirement it lowers from, held
/// again here because AIR is decoded from bytes nothing upstream saw.
fn validate_capability_permission_requests(verdict: &mut Verdict, air: &AirModule) {
    if air.capability_permission_requests.is_empty() {
        return;
    }
    let invoked = air.invoked_capability_refs();
    for (capability_ref, decision) in &air.capability_permission_requests {
        if !invoked.contains(capability_ref.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                capability_ref.clone(),
                "a capability permission request names a Capability no capability.invoke names",
            ));
        }
        if decision
            .reason()
            .is_some_and(|reason| reason.trim().is_empty())
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                capability_ref.clone(),
                "a capability permission request carries an empty reason",
            ));
        }
    }
}

/// A carried Hook binding describes the node it rides on and nothing else: it
/// belongs to a plain region, and that region is the binding's captured body.
fn validate_hook_node(verdict: &mut Verdict, region: &StructuralNode) {
    let Some(hook) = &region.hook else {
        return;
    };
    if region.kind != StructuralOpKind::Region {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            region.region_id.clone(),
            "a Hook binding rides on the plain region holding its captured body",
        ));
    }
    if hook.body_region_id != region.region_id {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            region.region_id.clone(),
            "carried Hook binding names a different captured body region",
        ));
    }
}

fn validate_loop_signature(verdict: &mut Verdict, region: &StructuralNode) {
    if region.kind != StructuralOpKind::Loop {
        return;
    }
    let initial: Vec<_> = region
        .operands
        .iter()
        .filter(|operand| operand.slot == SLOT_INITIAL)
        .collect();
    let carried: Vec<_> = region
        .operands
        .iter()
        .filter(|operand| operand.slot == SLOT_CARRIED)
        .collect();
    let only_phi_slots = region
        .operands
        .iter()
        .all(|operand| operand.slot == SLOT_INITIAL || operand.slot == SLOT_CARRIED);
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
            ));
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
