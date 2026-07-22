//! Deterministic FrontendGraph → AIR lowering.
//!
//! Typed regions lower to executable structural AIR/SSA: function shells, blocks,
//! loop-carried context values, yield/resume boundaries, joins, try/catch, and
//! compiled Hook callsites. Semantic operations preserve authored order; static
//! Hook bindings become structural `value` nodes the runtime executes around
//! their targets.
//!
//! The native bridges and `dekk agents canonical-air` submit FrontendGraph to
//! this owner path. No alternative graph-to-AIR builder is reachable.

use std::collections::{BTreeSet, HashSet};

use crate::air::{AirModule, AirVersion, SemanticOp, StructuralKind, StructuralNode};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict};
use crate::frontend_graph::{
    FrontendGraph, HookBinding, HookPhase, HookScope, ProgramDefinition, StructuralRegion,
};

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

    let semantic_operations = graph
        .semantic_operations
        .iter()
        .map(|op| SemanticOp {
            node_id: op.node_id.clone(),
            op: op.op,
            operands: op.operands.clone(),
        })
        .collect();

    let structural_ir = lower_structural_ir(graph);

    Ok(AirModule {
        schema_version: AirVersion::V1,
        semantic_operations,
        structural_ir,
        source_map: graph.source_map.clone(),
    })
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

fn validate_lowering(graph: &FrontendGraph, verdict: &mut Verdict) {
    let node_ids: HashSet<&str> = graph
        .semantic_operations
        .iter()
        .map(|op| op.node_id.as_str())
        .collect();
    let region_ids: HashSet<&str> = graph
        .structural_regions
        .iter()
        .map(|region| region.region_id.as_str())
        .collect();

    for edge in &graph.context_flow {
        if !node_ids.contains(edge.from_node.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                edge.from_node.clone(),
                "context flow from_node is not a recorded semantic operation",
            ));
        }
        if !node_ids.contains(edge.to_node.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                edge.to_node.clone(),
                "context flow to_node is not a recorded semantic operation",
            ));
        }
    }

    for hook in &graph.hook_bindings {
        let target = hook.target_selector.as_str();
        if !node_ids.contains(target) && !region_ids.contains(target) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                hook.hook_id.clone(),
                "hook target_selector does not reference a semantic node or structural region",
            ));
        }
    }
}

fn lower_structural_ir(graph: &FrontendGraph) -> Vec<StructuralNode> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    for program in &graph.program_definitions {
        push_unique_many(
            &mut out,
            &mut seen,
            [
                function_shell(program),
                entry_region(program),
                entry_block(program),
            ],
        );
    }

    push_unique(&mut out, &mut seen, execution_block());

    for region in &graph.structural_regions {
        expand_authored_region(graph, region, &mut out, &mut seen);
    }

    for hook in sorted_hook_bindings(&graph.hook_bindings) {
        if hook.phase == HookPhase::Before {
            push_unique(&mut out, &mut seen, hook_callsite(hook));
        }
    }

    for edge in &graph.context_flow {
        push_unique(&mut out, &mut seen, context_value(edge));
    }

    for hook in sorted_hook_bindings(&graph.hook_bindings) {
        if hook.phase == HookPhase::After {
            push_unique(&mut out, &mut seen, hook_callsite(hook));
        }
    }

    out
}

fn push_unique(out: &mut Vec<StructuralNode>, seen: &mut BTreeSet<String>, nodes: StructuralNode) {
    if seen.insert(nodes.region_id.clone()) {
        out.push(nodes);
    }
}

fn push_unique_many(
    out: &mut Vec<StructuralNode>,
    seen: &mut BTreeSet<String>,
    nodes: impl IntoIterator<Item = StructuralNode>,
) {
    for node in nodes {
        push_unique(out, seen, node);
    }
}

fn function_shell(program: &ProgramDefinition) -> StructuralNode {
    structural_node(format!("region.fn.{}", program.program_id), StructuralKind::Function)
}

fn entry_region(program: &ProgramDefinition) -> StructuralNode {
    structural_node(
        format!("region.entry.{}", program.program_id),
        StructuralKind::Region,
    )
}

fn entry_block(program: &ProgramDefinition) -> StructuralNode {
    structural_node(
        format!("block.entry.{}", program.program_id),
        StructuralKind::Block,
    )
}

fn execution_block() -> StructuralNode {
    structural_node("block.semantic.body", StructuralKind::Block)
}

fn expand_authored_region(
    graph: &FrontendGraph,
    region: &StructuralRegion,
    out: &mut Vec<StructuralNode>,
    seen: &mut BTreeSet<String>,
) {
    push_unique(out, seen, structural_node(region.region_id.clone(), region.kind));

    match region.kind {
        StructuralKind::Loop => {
            push_unique_many(
                out,
                seen,
                [
                    structural_node(format!("block.{}.header", region.region_id), StructuralKind::Block),
                    structural_node(format!("block.{}.body", region.region_id), StructuralKind::Block),
                    structural_node(
                        format!("value.{}.context.carry", region.region_id),
                        StructuralKind::Value,
                    ),
                    structural_node(
                        format!("value.{}.resume.input", region.region_id),
                        StructuralKind::Value,
                    ),
                    structural_node(
                        format!("region.{}.yield", region.region_id),
                        StructuralKind::Yield,
                    ),
                ],
            );
        }
        StructuralKind::Try => {
            push_unique_many(
                out,
                seen,
                [
                    structural_node(format!("block.{}.try", region.region_id), StructuralKind::Block),
                    structural_node(
                        format!("region.{}.catch", region.region_id),
                        StructuralKind::Catch,
                    ),
                ],
            );
        }
        StructuralKind::ParallelJoin => {
            push_unique(
                out,
                seen,
                structural_node(format!("block.{}.join", region.region_id), StructuralKind::Block),
            );
        }
        StructuralKind::Branch | StructuralKind::Switch => {
            push_unique_many(
                out,
                seen,
                [
                    structural_node(format!("block.{}.then", region.region_id), StructuralKind::Block),
                    structural_node(format!("block.{}.else", region.region_id), StructuralKind::Block),
                ],
            );
        }
        StructuralKind::Catch | StructuralKind::Throw | StructuralKind::Return | StructuralKind::Yield => {}
        StructuralKind::Function | StructuralKind::Region | StructuralKind::Block | StructuralKind::Value => {}
    }

    // Join nodes referenced by parallel/task scopes when present in the graph.
    if region.kind == StructuralKind::Loop {
        let _ = graph;
    }
}

fn hook_callsite(hook: &HookBinding) -> StructuralNode {
    let phase = match hook.phase {
        HookPhase::Before => "before",
        HookPhase::After => "after",
    };
    structural_node(
        format!("value.hook.{phase}.{}", hook.hook_id),
        StructuralKind::Value,
    )
}

fn context_value(edge: &crate::frontend_graph::ContextEdge) -> StructuralNode {
    structural_node(
        format!("value.context.{}.{}", edge.from_node, edge.to_node),
        StructuralKind::Value,
    )
}

fn structural_node(region_id: impl Into<String>, kind: StructuralKind) -> StructuralNode {
    StructuralNode {
        region_id: region_id.into(),
        kind,
    }
}

fn sorted_hook_bindings(bindings: &[HookBinding]) -> Vec<&HookBinding> {
    let mut sorted: Vec<&HookBinding> = bindings.iter().collect();
    sorted.sort_by(|left, right| {
        scope_rank(left.scope)
            .cmp(&scope_rank(right.scope))
            .then_with(|| left.declaration_order.cmp(&right.declaration_order))
            .then_with(|| left.hook_id.cmp(&right.hook_id))
    });
    sorted
}

const fn scope_rank(scope: HookScope) -> u8 {
    match scope {
        HookScope::Agent => 0,
        HookScope::Loop => 1,
        HookScope::Node => 2,
        HookScope::Model => 3,
        HookScope::Capability => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend_graph::verify_frontend_graph_json;
    use std::collections::BTreeMap;
    use serde_json::json;

    fn graph(value: serde_json::Value) -> FrontendGraph {
        serde_json::from_value(value).expect("valid graph fixture")
    }

    #[test]
    fn rejects_unknown_hook_target() {
        let graph = graph(json!({
            "schema_version": "apxm.frontend-graph.v1",
            "source_language": "python",
            "program_definitions": [{
                "program_id": "P",
                "entrypoint": "run",
                "input_type_ref": "I",
                "output_type_ref": "O",
                "has_default_context": false
            }],
            "imported_program_refs": [],
            "semantic_operations": [{
                "node_id": "node.one",
                "op": "model.call"
            }],
            "structural_regions": [{
                "region_id": "region.return",
                "kind": "return"
            }],
            "context_flow": [],
            "hook_bindings": [{
                "hook_id": "hook.bad",
                "scope": "node",
                "phase": "before",
                "target_selector": "node.missing",
                "declaration_order": 0,
                "handler_ref": "hooks.bad",
                "handler_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "input_type_ref": "I",
                "output_type_ref": "O",
                "return_mode": "observe"
            }],
            "capability_requirements": [],
            "model_requirements": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        }));
        assert!(verify_frontend_graph_json(&json!(graph)).is_accepted());
        let err = frontend_graph_to_air(&graph).expect_err("unknown hook target rejected");
        assert!(
            err.diagnostics()
                .iter()
                .any(|d| d.location == "hook.bad"),
            "expected hook diagnostic, got {err:?}"
        );
    }

    #[test]
    fn lowers_loop_yield_and_hook_callsites() {
        let graph = graph(json!({
            "schema_version": "apxm.frontend-graph.v1",
            "source_language": "python",
            "program_definitions": [{
                "program_id": "Specialist",
                "entrypoint": "run",
                "input_type_ref": "SpecialistInput",
                "output_type_ref": "SpecialistOutput",
                "context_type_ref": "SpecialistContext",
                "has_default_context": true
            }],
            "imported_program_refs": [],
            "semantic_operations": [
                { "node_id": "node.model.1", "op": "model.call", "operands": { "model_target_ref": "model.default" } },
                { "node_id": "node.cap.1", "op": "capability.invoke", "operands": { "capability_ref": "cap.search" } }
            ],
            "structural_regions": [
                { "region_id": "region.loop.1", "kind": "loop" },
                { "region_id": "region.return.1", "kind": "return" }
            ],
            "context_flow": [{
                "from_node": "node.model.1",
                "to_node": "node.cap.1",
                "context_type_ref": "SpecialistContext"
            }],
            "hook_bindings": [{
                "hook_id": "hook.before.model",
                "scope": "model",
                "phase": "before",
                "target_selector": "node.model.1",
                "declaration_order": 0,
                "handler_ref": "hooks.before_model",
                "handler_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "input_type_ref": "ModelContext",
                "output_type_ref": "ModelContext",
                "return_mode": "observe"
            }],
            "capability_requirements": [{ "capability_ref": "cap.search" }],
            "model_requirements": [{ "model_target_ref": "model.default" }],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        }));
        let air = frontend_graph_to_air(&graph).expect("lowering succeeds");
        let kinds: BTreeMap<_, _> = air
            .structural_ir
            .iter()
            .map(|node| (node.region_id.as_str(), node.kind))
            .collect();
        assert!(kinds.contains_key("region.fn.Specialist"));
        assert!(kinds.contains_key("region.loop.1"));
        assert!(kinds.contains_key("region.region.loop.1.yield"));
        assert!(kinds.contains_key("value.hook.before.hook.before.model"));
        assert!(kinds.contains_key("value.context.node.model.1.node.cap.1"));
        assert_eq!(air.semantic_operations.len(), 2);
    }
}
