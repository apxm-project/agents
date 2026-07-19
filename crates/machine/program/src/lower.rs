//! Deterministic FrontendGraph → AIR lowering.
//!
//! The five semantic operations and the structural control-flow records carry
//! straight through from the recorded graph to canonical AIR; the source map is
//! preserved. The lowering is a pure, order-preserving mapping with no provider,
//! runtime, store, or discovery dependency, so the same graph always lowers to
//! the same AIR.

use crate::air::{AirModule, AirVersion, SemanticOp, StructuralNode};
use crate::diagnostic::Verdict;
use crate::frontend_graph::FrontendGraph;

/// Lower a FrontendGraph to canonical AIR, failing closed with the graph's
/// verification diagnostics if it does not verify.
///
/// # Errors
///
/// Returns the rejecting [`Verdict`] when the FrontendGraph does not verify.
pub fn frontend_graph_to_air(graph: &FrontendGraph) -> Result<AirModule, Verdict> {
    let verdict = graph.verify();
    if !verdict.is_accepted() {
        return Err(verdict);
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

    let structural_ir = graph
        .structural_regions
        .iter()
        .map(|region| StructuralNode {
            region_id: region.region_id.clone(),
            kind: region.kind,
        })
        .collect();

    Ok(AirModule {
        schema_version: AirVersion::V1,
        semantic_operations,
        structural_ir,
        source_map: graph.source_map.clone(),
    })
}
