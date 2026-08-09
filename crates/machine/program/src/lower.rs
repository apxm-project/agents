//! Deterministic FrontendGraph → AIR lowering.
//!
//! This is the one canonical FrontendGraph→AIR owner. It consumes the typed
//! discriminated intents of `apxm.frontend-graph.v1` and reconstructs canonical
//! AIR:
//!
//! - each typed [`CallIntent`] selects exactly one of the five semantic AIS
//!   operations (a Tool invocation and an advanced Capability invocation both
//!   converge to `capability.invoke`), and its ordered `operand_values` become
//!   typed SSA [`Operand`]s whose slot names come from the AIS operand catalogue
//!   and whose types come from the graph's value table;
//! - each typed [`ControlIntent`] and the lexical [`Region`]s it owns become
//!   structural AIS nodes carrying typed block arguments (CFG joins, loop-carried
//!   Context, resume input) and typed operands (branch condition, switch
//!   scrutinee, carried/yielded/returned values); and
//! - static Hooks expand into ordered structural wrapper regions around the
//!   selected target in declaration order, adding no operation.
//!
//! Canonical ordering is a contract field, not an accident: declarations use
//! canonical source identity, structural siblings and operations use the
//! frontend's `execution_order`, block arguments use stable value order, and Hook
//! wrappers use declared scope/phase/order. Python and TypeScript goldens
//! therefore converge after canonicalization even when their host ASTs differ.
//!
//! The native bridges and `dekk agents canonical-air` submit FrontendGraph to
//! this owner path. No alternative graph-to-AIR builder is reachable.

use std::collections::{BTreeMap, HashMap, HashSet};

use apxm_ais::{SemanticOpKind, StructuralOpKind};

use crate::air::{
    AirModule, AirVersion, ContextEdge as AirContextEdge, ControlPredicate as AirControlPredicate,
    Operand, PredicateComparator as AirPredicateComparator,
    PredicateLiteral as AirPredicateLiteral, SemanticOp, SsaValue, StructuralNode, ValueAssembly,
};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict};
use crate::frontend_graph::{
    CallIntent, ControlIntent, ControlKind, ControlPredicate, FrontendGraph, HookBinding,
    HookPhase, IntentKind, PredicateComparator, PredicateLiteral, Region, RegionRole, Value,
};

const EXECUTION_ORDER_STRIDE: u32 = 1_000_000;
const STRUCTURAL_ORDER_OFFSET: u32 = 500_000;

/// Lower a FrontendGraph to canonical AIR, failing closed with diagnostics when
/// verification or lowering preconditions fail.
///
/// # Errors
///
/// Returns the rejecting [`Verdict`] when the graph does not verify or cannot
/// lower deterministically.
pub fn frontend_graph_to_air(graph: &FrontendGraph) -> Result<AirModule, Verdict> {
    let verdict = graph.verify();
    if !verdict.is_accepted() {
        return Err(verdict);
    }
    let mut lowering = Verdict::accepted();
    validate_lowering(graph, &mut lowering);
    if !lowering.is_accepted() {
        return Err(lowering.finish());
    }

    let values: HashMap<&str, &Value> = graph
        .values
        .iter()
        .map(|value| (value.value_id.as_str(), value))
        .collect();

    // Declaration lookup: an intent's binding_ref resolves to the exact external
    // reference the runtime needs (model target, capability, event, program).
    let declarations: HashMap<&str, &crate::frontend_graph::Declaration> = graph
        .declarations
        .iter()
        .map(|decl| (decl.decl_id.as_str(), decl))
        .collect();

    // Slot lookup for each consuming node: (consumer node id, value id) -> slot.
    let mut slot_of: HashMap<(&str, &str), &str> = HashMap::new();
    for edge in &graph.data_edges {
        slot_of.insert(
            (edge.to_consumer.as_str(), edge.from_value.as_str()),
            edge.consumer_slot.as_str(),
        );
    }

    let mut semantic_operations = Vec::with_capacity(graph.call_intents.len());
    for intent in &graph.call_intents {
        semantic_operations.push(lower_call_intent(
            intent,
            &values,
            &slot_of,
            &declarations,
            graph,
            &mut lowering,
        ));
    }
    // Canonical operation order: structural containment plus frontend
    // execution_order. Ordering by (parent_region_id, execution_order) is stable
    // and independent of host AST emission order.
    semantic_operations.sort_by(|a, b| {
        a.parent_region_id
            .cmp(&b.parent_region_id)
            .then(a.execution_order.cmp(&b.execution_order))
    });

    let mut structural_ir = lower_structural_ir(graph, &values, &slot_of, &mut lowering);

    if !lowering.is_accepted() {
        return Err(lowering.finish());
    }

    add_entry_block_arguments(
        &mut structural_ir,
        &semantic_operations,
        &graph.values,
        &mut lowering,
    );
    if !lowering.is_accepted() {
        return Err(lowering.finish());
    }

    let air = AirModule {
        schema_version: AirVersion::V1,
        value_assemblies: graph
            .values
            .iter()
            .filter(|value| value.origin == crate::frontend_graph::ValueOrigin::Literal)
            .map(|value| ValueAssembly {
                value_id: value.value_id.clone(),
                dependencies: value.dependencies.clone(),
            })
            .collect(),
        semantic_operations,
        structural_ir,
        context_flow: graph
            .context_flow
            .iter()
            .map(|edge| AirContextEdge {
                from_node: edge.from_node.clone(),
                to_node: edge.to_node.clone(),
                context_type_ref: edge.context_type_ref.clone(),
            })
            .collect(),
        source_map: graph.source_map.clone(),
    };
    let air_verdict = air.verify();
    if !air_verdict.is_accepted() {
        return Err(air_verdict);
    }
    Ok(air)
}

/// Select the AIS semantic operation for a typed call intent. Tool and advanced
/// Capability invocations both converge to `capability.invoke`.
fn select_semantic_op(kind: IntentKind) -> SemanticOpKind {
    match kind {
        IntentKind::ModelInvocation => SemanticOpKind::ModelCall,
        IntentKind::ToolInvocation | IntentKind::CapabilityInvocation => {
            SemanticOpKind::CapabilityInvoke
        }
        IntentKind::AgentCreation => SemanticOpKind::ProgramNew,
        IntentKind::AgentInvocation => SemanticOpKind::ProgramInvoke,
        IntentKind::EventWait => SemanticOpKind::AwaitEvent,
    }
}

fn lower_call_intent(
    intent: &CallIntent,
    values: &HashMap<&str, &Value>,
    slot_of: &HashMap<(&str, &str), &str>,
    declarations: &HashMap<&str, &crate::frontend_graph::Declaration>,
    graph: &FrontendGraph,
    verdict: &mut Verdict,
) -> SemanticOp {
    let op = select_semantic_op(intent.intent_kind);
    let mut operands = Vec::new();

    // The exact external reference the runtime binds is carried as the first
    // operand, taken from the resolved binding declaration or imported program.
    if let Some((slot, reference, type_ref)) = reference_operand(intent, declarations, graph) {
        operands.push(Operand {
            slot: slot.to_string(),
            value_id: reference,
            type_ref,
        });
    }

    operands.extend(resolve_operands(
        &intent.node_id,
        &intent.operand_values,
        values,
        slot_of,
        verdict,
    ));

    let result = intent.result_value.as_deref().map(|value_id| {
        let type_ref = values
            .get(value_id)
            .map_or_else(|| value_id.to_string(), |value| value.type_ref.clone());
        SsaValue {
            value_id: value_id.to_string(),
            type_ref,
        }
    });

    SemanticOp {
        node_id: intent.node_id.clone(),
        op,
        parent_region_id: intent.parent_region_id.clone(),
        execution_order: canonical_execution_order(intent.execution_order, STRUCTURAL_ORDER_OFFSET),
        operands,
        result,
    }
}

/// The exact external reference operand for an effect intent: which model,
/// capability, event, or program the runtime binds. Its value is the target
/// reference string carried by the resolved declaration or imported program.
fn reference_operand(
    intent: &CallIntent,
    declarations: &HashMap<&str, &crate::frontend_graph::Declaration>,
    graph: &FrontendGraph,
) -> Option<(&'static str, String, String)> {
    let binding_ref = intent.binding_ref.as_deref()?;
    match intent.intent_kind {
        IntentKind::ModelInvocation => {
            let target = declarations.get(binding_ref)?.target_ref.clone()?;
            Some(("model_ref", target, "ModelTargetRef".to_string()))
        }
        IntentKind::ToolInvocation | IntentKind::CapabilityInvocation => {
            let target = declarations.get(binding_ref)?.target_ref.clone()?;
            Some(("capability_ref", target, "CapabilityRef".to_string()))
        }
        IntentKind::EventWait => {
            let target = declarations
                .get(binding_ref)
                .and_then(|decl| decl.target_ref.clone())
                .unwrap_or_else(|| binding_ref.to_string());
            Some(("event_ref", target, "EventRef".to_string()))
        }
        IntentKind::AgentCreation => {
            let target = resolve_program_ref(binding_ref, graph);
            Some(("program_ref", target, "ProgramRef".to_string()))
        }
        IntentKind::AgentInvocation => {
            let target = resolve_program_ref(binding_ref, graph);
            let type_ref = match intent.receiver_kind {
                Some(crate::frontend_graph::ReceiverKind::ProgramInstanceRef) => {
                    "ProgramInstanceRef"
                }
                _ => "ProgramRef",
            };
            Some(("receiver", target, type_ref.to_string()))
        }
    }
}

/// Materialize every AIR operand that is not produced by a semantic operation
/// or declared as a lexical block argument as an entry block argument. Source
/// references (model targets, capability refs, imported programs) are typed
/// values in the canonical function signature, never string-only pseudo
/// operands in the MLIR emitter.
fn add_entry_block_arguments(
    structural_ir: &mut [StructuralNode],
    semantic_operations: &[SemanticOp],
    values: &[Value],
    verdict: &mut Verdict,
) {
    let mut definitions: HashSet<&str> = semantic_operations
        .iter()
        .filter_map(|operation| operation.result.as_ref())
        .map(|result| result.value_id.as_str())
        .collect();
    definitions.extend(
        values
            .iter()
            .filter(|value| value.origin == crate::frontend_graph::ValueOrigin::Literal)
            .map(|value| value.value_id.as_str()),
    );
    for node in structural_ir.iter() {
        for argument in &node.block_arguments {
            definitions.insert(argument.value_id.as_str());
        }
    }

    let mut entry_values = BTreeMap::new();
    for operation in semantic_operations {
        for operand in &operation.operands {
            if !definitions.contains(operand.value_id.as_str()) {
                entry_values
                    .entry(operand.value_id.clone())
                    .or_insert_with(|| operand.type_ref.clone());
            }
        }
    }
    for assembly in values
        .iter()
        .filter(|value| value.origin == crate::frontend_graph::ValueOrigin::Literal)
    {
        for dependency_id in &assembly.dependencies {
            if definitions.contains(dependency_id.as_str()) {
                continue;
            }
            let Some(dependency) = values.iter().find(|value| value.value_id == *dependency_id)
            else {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    dependency_id,
                    "value assembly dependency is not declared by the frontend graph",
                ));
                continue;
            };
            entry_values
                .entry(dependency_id.clone())
                .or_insert_with(|| dependency.type_ref.clone());
        }
    }
    let Some(entry) = structural_ir
        .iter_mut()
        .find(|node| node.kind == StructuralOpKind::Function)
    else {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            "structural_ir",
            "canonical AIR has no function structural region for entry operands",
        ));
        return;
    };
    for (value_id, type_ref) in entry_values {
        entry.block_arguments.push(SsaValue { value_id, type_ref });
    }
    entry
        .block_arguments
        .sort_by(|left, right| left.value_id.cmp(&right.value_id));
}

/// Resolve a program binding reference to its imported program reference string.
fn resolve_program_ref(binding_ref: &str, graph: &FrontendGraph) -> String {
    graph
        .imported_program_refs
        .iter()
        .find(|import| import.program_ref == binding_ref)
        .map(|import| import.program_ref.clone())
        .unwrap_or_else(|| binding_ref.to_string())
}

/// Resolve an ordered list of value ids into typed SSA operands, taking each
/// operand's slot from the matching data edge and its type from the value table.
fn resolve_operands(
    consumer: &str,
    operand_values: &[String],
    values: &HashMap<&str, &Value>,
    slot_of: &HashMap<(&str, &str), &str>,
    verdict: &mut Verdict,
) -> Vec<Operand> {
    let mut operands = Vec::with_capacity(operand_values.len());
    for value_id in operand_values {
        let Some(value) = values.get(value_id.as_str()) else {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                consumer.to_string(),
                format!("operand value '{value_id}' is not a declared value"),
            ));
            continue;
        };
        let Some(slot) = slot_of.get(&(consumer, value_id.as_str())) else {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                consumer.to_string(),
                format!("operand value '{value_id}' has no typed data edge naming its slot"),
            ));
            continue;
        };
        operands.push(Operand {
            slot: (*slot).to_string(),
            value_id: value_id.clone(),
            type_ref: value.type_ref.clone(),
        });
    }
    operands
}

/// Map a control intent to its structural AIS operation kind.
fn select_structural_op(kind: ControlKind) -> StructuralOpKind {
    match kind {
        ControlKind::Conditional => StructuralOpKind::Branch,
        ControlKind::Switch => StructuralOpKind::Switch,
        ControlKind::Loop => StructuralOpKind::Loop,
        ControlKind::TaskGroup => StructuralOpKind::ParallelJoin,
        ControlKind::TryCatch => StructuralOpKind::Try,
        ControlKind::Throw => StructuralOpKind::Throw,
        ControlKind::Yield => StructuralOpKind::Yield,
        ControlKind::Return => StructuralOpKind::Return,
    }
}

fn lower_predicate(predicate: &ControlPredicate) -> AirControlPredicate {
    AirControlPredicate {
        root_value_id: predicate.root_value_id.clone(),
        property_path: predicate.property_path.clone(),
        comparator: match predicate.comparator {
            PredicateComparator::Truthy => AirPredicateComparator::Truthy,
            PredicateComparator::Equals => AirPredicateComparator::Equals,
            PredicateComparator::NotEquals => AirPredicateComparator::NotEquals,
        },
        literal: predicate.literal.as_ref().map(|literal| match literal {
            PredicateLiteral::Boolean(value) => AirPredicateLiteral::Boolean(*value),
            PredicateLiteral::String(value) => AirPredicateLiteral::String(value.clone()),
            PredicateLiteral::Integer(value) => AirPredicateLiteral::Integer(*value),
            PredicateLiteral::Null => AirPredicateLiteral::Null,
        }),
    }
}

/// Build structural AIR: one enclosing function region, one node per lexical
/// region, and one node per control intent, each carrying typed block arguments
/// and operands. Hooks expand into ordered wrapper regions around their target.
fn lower_structural_ir(
    graph: &FrontendGraph,
    values: &HashMap<&str, &Value>,
    slot_of: &HashMap<(&str, &str), &str>,
    verdict: &mut Verdict,
) -> Vec<StructuralNode> {
    let mut nodes: Vec<StructuralNode> = Vec::new();

    // A loop's body region is the region the loop's operations parent to, so the
    // loop itself becomes the structural region enclosing them: the ais.loop node
    // adopts the body region id and the plain body region is not emitted twice.
    let loop_body_regions: HashMap<&str, &crate::frontend_graph::ControlIntent> = graph
        .control_intents
        .iter()
        .filter(|intent| intent.control_kind == ControlKind::Loop)
        .filter_map(|intent| {
            intent
                .body_region_ids
                .first()
                .map(|body| (body.as_str(), intent))
        })
        .collect();
    let owned_control_regions: HashMap<&str, (&crate::frontend_graph::ControlIntent, usize)> =
        graph
            .control_intents
            .iter()
            .filter(|intent| intent.control_kind != ControlKind::Loop)
            .flat_map(|intent| {
                intent
                    .body_region_ids
                    .iter()
                    .enumerate()
                    .map(move |(index, region_id)| (region_id.as_str(), (intent, index)))
            })
            .collect();

    // Lexical regions become structural region/function nodes preserving the
    // containment tree. The entrypoint body region is a function region; a loop
    // body region becomes the ais.loop node itself.
    let entry_body: Option<&str> = graph
        .functions
        .iter()
        .find(|f| f.is_entrypoint)
        .map(|f| f.body_region_id.as_str());

    for region in &graph.regions {
        let block_arguments = region_block_arguments(&region.region_id, graph, values);
        if let Some(intent) = loop_body_regions.get(region.region_id.as_str()) {
            // Emit the ais.loop node in place of the body region, parented where
            // the loop occurs and carrying the loop's typed operands.
            let operands = resolve_operands(
                &intent.node_id,
                &intent.operand_values,
                values,
                slot_of,
                verdict,
            );
            nodes.push(StructuralNode {
                region_id: region.region_id.clone(),
                kind: StructuralOpKind::Loop,
                parent_region_id: Some(intent.parent_region_id.clone()),
                execution_order: canonical_execution_order(
                    intent.execution_order,
                    STRUCTURAL_ORDER_OFFSET,
                ),
                block_arguments,
                operands,
                predicate: intent.predicate.as_ref().map(lower_predicate),
            });
            continue;
        }
        let kind = region_structural_kind(region, entry_body);
        let (parent_region_id, execution_order) = owned_control_regions
            .get(region.region_id.as_str())
            .map(|(intent, arm_index)| {
                (
                    Some(intent.node_id.clone()),
                    canonical_execution_order(*arm_index as u32, STRUCTURAL_ORDER_OFFSET),
                )
            })
            .unwrap_or_else(|| {
                (
                    region.parent_region_id.clone(),
                    canonical_execution_order(region.execution_order, STRUCTURAL_ORDER_OFFSET),
                )
            });
        nodes.push(StructuralNode {
            region_id: region.region_id.clone(),
            kind,
            parent_region_id,
            execution_order,
            block_arguments,
            operands: Vec::new(),
            predicate: None,
        });
    }

    // Non-loop control intents become structural nodes hanging off their parent
    // region. Loops are already emitted from their body region above.
    for intent in &graph.control_intents {
        if intent.control_kind == ControlKind::Loop {
            continue;
        }
        let kind = select_structural_op(intent.control_kind);
        let operands = resolve_operands(
            &intent.node_id,
            &intent.operand_values,
            values,
            slot_of,
            verdict,
        );
        let block_arguments = control_block_arguments(intent, values);
        nodes.push(StructuralNode {
            region_id: intent.node_id.clone(),
            kind,
            parent_region_id: Some(intent.parent_region_id.clone()),
            execution_order: canonical_execution_order(
                intent.execution_order,
                STRUCTURAL_ORDER_OFFSET,
            ),
            block_arguments,
            operands,
            predicate: intent.predicate.as_ref().map(lower_predicate),
        });
    }

    // Hook wrappers: one structural region per hook, ordered by declaration
    // order, hung off the hook's target selector. This adds no operation and no
    // dynamic registry.
    let mut hooks: Vec<&HookBinding> = graph.hook_bindings.iter().collect();
    hooks.sort_by(|a, b| {
        a.target_selector
            .cmp(&b.target_selector)
            .then(a.declaration_order.cmp(&b.declaration_order))
    });
    for hook in hooks {
        let (parent_region_id, target_execution_order) = hook_parent_region(graph, hook).expect(
            "lowering validation resolves every Hook target selector before structural expansion",
        );
        nodes.push(StructuralNode {
            region_id: hook.hook_id.clone(),
            kind: StructuralOpKind::Region,
            parent_region_id: Some(parent_region_id),
            execution_order: hook_execution_order(
                hook.phase,
                target_execution_order,
                hook.declaration_order,
            ),
            block_arguments: Vec::new(),
            operands: Vec::new(),
            predicate: None,
        });
    }

    // Canonical structural order: by containment then execution order, stable and
    // host-AST-independent.
    nodes.sort_by(|a, b| {
        a.parent_region_id
            .cmp(&b.parent_region_id)
            .then(a.execution_order.cmp(&b.execution_order))
            .then(a.region_id.cmp(&b.region_id))
    });
    nodes
}

/// Hook bindings retain their selected node or region in the typed Hook
/// metadata. Their structural wrapper is a sibling in that selected target's
/// lexical parent, because a Hook target can be a call node and only regions
/// may own structural children in AIR.
fn hook_parent_region(graph: &FrontendGraph, hook: &HookBinding) -> Option<(String, u32)> {
    graph
        .call_intents
        .iter()
        .find(|intent| intent.node_id == hook.target_selector)
        .map(|intent| (intent.parent_region_id.clone(), intent.execution_order))
        .or_else(|| {
            graph
                .control_intents
                .iter()
                .find(|intent| intent.node_id == hook.target_selector)
                .map(|intent| (intent.parent_region_id.clone(), intent.execution_order))
        })
        .or_else(|| {
            graph
                .regions
                .iter()
                .find(|region| region.region_id == hook.target_selector)
                .and_then(|region| {
                    region
                        .parent_region_id
                        .clone()
                        .map(|parent| (parent, region.execution_order))
                })
        })
}

/// The entrypoint body region lowers to a `function` structural node; all other
/// lexical regions lower to `region` nodes. Loop/try/catch semantics are carried
/// by the owning control intent, not by the passive lexical region.
fn region_structural_kind(region: &Region, entry_body: Option<&str>) -> StructuralOpKind {
    if entry_body == Some(region.region_id.as_str())
        && region.region_role == RegionRole::FunctionBody
    {
        StructuralOpKind::Function
    } else {
        StructuralOpKind::Region
    }
}

/// Collect the typed block arguments a region carries: any value whose origin is
/// a block argument and whose declaring block belongs to this region.
fn region_block_arguments(
    region_id: &str,
    graph: &FrontendGraph,
    values: &HashMap<&str, &Value>,
) -> Vec<SsaValue> {
    let mut args: Vec<SsaValue> = Vec::new();
    for block in graph.blocks.iter().filter(|b| b.region_id == region_id) {
        for value_id in &block.block_arguments {
            if let Some(value) = values.get(value_id.as_str()) {
                if value.origin == crate::frontend_graph::ValueOrigin::ResumeInput {
                    continue;
                }
                args.push(SsaValue {
                    value_id: value.value_id.clone(),
                    type_ref: value.type_ref.clone(),
                });
            }
        }
    }
    args.sort_by(|a, b| a.value_id.cmp(&b.value_id));
    args
}

/// Loops own carried values and yield nodes own their exact resume destination.
fn control_block_arguments(
    intent: &ControlIntent,
    values: &HashMap<&str, &Value>,
) -> Vec<SsaValue> {
    match intent.control_kind {
        ControlKind::Loop | ControlKind::Yield => intent
            .result_value
            .as_deref()
            .and_then(|value_id| values.get(value_id))
            .map(|value| {
                vec![SsaValue {
                    value_id: value.value_id.clone(),
                    type_ref: value.type_ref.clone(),
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Before-hooks wrap ahead of the target region, after-hooks behind it. The
/// declaration order breaks ties within a phase.
fn hook_execution_order(
    phase: HookPhase,
    target_execution_order: u32,
    declaration_order: u32,
) -> u32 {
    match phase {
        HookPhase::Before => canonical_execution_order(target_execution_order, declaration_order),
        HookPhase::After => canonical_execution_order(
            target_execution_order,
            STRUCTURAL_ORDER_OFFSET
                .saturating_add(1)
                .saturating_add(declaration_order),
        ),
    }
}

/// Reserve a deterministic ordering interval around every source-order slot:
/// before Hooks, the selected source node, then after Hooks. The source graph
/// remains the owner of the original lexical order; AIR uses the expanded order
/// so structural wrapper nodes never collide with effect nodes in one region.
fn canonical_execution_order(source_order: u32, offset: u32) -> u32 {
    source_order
        .saturating_mul(EXECUTION_ORDER_STRIDE)
        .saturating_add(offset)
}

/// Lower a FrontendGraph JSON document to canonical AIR JSON.
///
/// # Errors
///
/// Returns a diagnostic string when the graph does not decode, verify, or lower.
pub fn lower_frontend_graph_json(graph_json: &str) -> Result<String, String> {
    let graph: FrontendGraph = serde_json::from_str(graph_json).map_err(|e| e.to_string())?;
    let air = frontend_graph_to_air(&graph).map_err(format_verdict)?;
    serde_json::to_string(&air).map_err(|e| e.to_string())
}

fn format_verdict(verdict: Verdict) -> String {
    let rendered: Vec<String> = verdict
        .into_diagnostics()
        .into_iter()
        .map(|d| format!("{}:{}:{}", d.code.slug(), d.location, d.message))
        .collect();
    format!("frontend graph rejected: [{}]", rendered.join("; "))
}

/// Lowering-time cross-reference checks over the typed graph: context-flow and
/// Hook targets must reference real intents or regions.
fn validate_lowering(graph: &FrontendGraph, verdict: &mut Verdict) {
    let mut node_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for intent in &graph.call_intents {
        node_ids.insert(intent.node_id.as_str());
    }
    for intent in &graph.control_intents {
        node_ids.insert(intent.node_id.as_str());
    }
    let region_ids: std::collections::HashSet<&str> = graph
        .regions
        .iter()
        .map(|region| region.region_id.as_str())
        .collect();

    for edge in &graph.context_flow {
        if !node_ids.contains(edge.from_node.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                edge.from_node.clone(),
                "context flow from_node is not a recorded intent",
            ));
        }
        if !node_ids.contains(edge.to_node.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                edge.to_node.clone(),
                "context flow to_node is not a recorded intent",
            ));
        }
    }

    for hook in &graph.hook_bindings {
        let target = hook.target_selector.as_str();
        if !node_ids.contains(target) && !region_ids.contains(target) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                hook.hook_id.clone(),
                "hook target_selector does not reference an intent or region",
            ));
        }
    }
}
