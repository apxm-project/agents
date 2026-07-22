//! Deterministic FrontendGraph → AIR lowering.
//!
//! Typed containment and sibling execution order lower without flattening or
//! reconstructing control from names or source order.
//!
//! The native bridges and `dekk agents canonical-air` submit FrontendGraph to
//! this owner path. No alternative graph-to-AIR builder is reachable.

use std::collections::HashSet;

use crate::air::{AirModule, AirVersion, ContextEdge as AirContextEdge, SemanticOp, StructuralNode};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict};
use crate::frontend_graph::FrontendGraph;

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
            parent_region_id: op.parent_region_id.clone(),
            execution_order: op.execution_order,
            operands: op.operands.clone(),
        })
        .collect();

    let structural_ir = lower_structural_ir(graph);

    Ok(AirModule {
        schema_version: AirVersion::V1,
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
    graph
        .structural_regions
        .iter()
        .map(|region| StructuralNode {
            region_id: region.region_id.clone(),
            kind: region.kind,
            parent_region_id: region.parent_region_id.clone(),
            execution_order: region.execution_order,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend_graph::verify_frontend_graph_json;
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
                "op": "model.call",
                "parent_region_id": "region.body",
                "execution_order": 0
            }],
            "structural_regions": [{
                "region_id": "region.body",
                "kind": "region",
                "execution_order": 0
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
    fn lowers_typed_containment_without_synthetic_nodes() {
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
                {
                    "node_id": "node.model.1",
                    "op": "model.call",
                    "parent_region_id": "region.loop.1",
                    "execution_order": 0,
                    "operands": { "model_target_ref": "model.default" }
                },
                {
                    "node_id": "node.cap.1",
                    "op": "capability.invoke",
                    "parent_region_id": "region.loop.1",
                    "execution_order": 1,
                    "operands": { "capability_ref": "cap.search" }
                }
            ],
            "structural_regions": [
                {
                    "region_id": "region.loop.1",
                    "kind": "ais.loop",
                    "execution_order": 0
                }
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
                "region_annotations": [
                    {"region_id": "region.loop.1", "annotation": "structural_loop"}
                ]
            }
        }));
        let air = frontend_graph_to_air(&graph).expect("lowering succeeds");
        assert_eq!(air.structural_ir.len(), 1);
        assert_eq!(air.structural_ir[0].region_id, "region.loop.1");
        assert_eq!(air.semantic_operations[0].parent_region_id, "region.loop.1");
        assert_eq!(air.semantic_operations[1].execution_order, 1);
        assert_eq!(air.semantic_operations.len(), 2);
    }
}
