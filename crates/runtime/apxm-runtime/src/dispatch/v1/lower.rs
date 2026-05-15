//! Lowering between APXM seed types and `DispatchIrV1`.
//!
//! `lower_graph` builds a `DispatchIrV1` from compiled `GraphMetadata` plus the
//! per-node `ApxmGraphHints` already produced by the runtime. `derive_apxm_hints`
//! goes the other way for one node, so the vLLM adapter can keep emitting
//! today's `extra_body.vllm_xargs.apxm` payload from the new IR without any
//! field loss.
//!
//! This is the round-trip path the design note requires before Dispatch IR can
//! claim to "preserve the existing fields". See
//! `.apxm/docs/design/dispatch-ir.md` §"Working Goal".

use std::collections::HashMap;

use apxm_core::types::graph_hints::{ApxmGraphHints, GraphMetadata, PinPolicy};
use apxm_core::types::graph_metrics::NodeGraphMetrics;

use super::plan::{
    BackendCapabilityRequirements, DispatchIrV1, GraphDispatchPlan, NodeDispatchPlan,
    TelemetryContract,
};

/// Lower a compiled graph (metadata + per-node hints) into `DispatchIrV1`.
///
/// `node_hints` is keyed by the same `node_id` field that `GraphMetadata::nodes`
/// uses; nodes without a matching hint still appear in the IR with default
/// pin policy and no graph-metrics fields.
pub(crate) fn lower_graph(
    metadata: &GraphMetadata,
    node_hints: &HashMap<u32, ApxmGraphHints>,
    requirements: BackendCapabilityRequirements,
    telemetry: TelemetryContract,
) -> DispatchIrV1 {
    let graph = GraphDispatchPlan {
        graph_id: metadata.graph_id.clone(),
        execution_id: metadata.execution_id.clone(),
        node_count: metadata.node_count,
        critical_path_length: metadata.critical_path_length,
        max_parallelism: metadata.max_parallelism,
        default_pin_ttl_ms: metadata.default_pin_ttl_ms,
    };

    let nodes = metadata
        .nodes
        .iter()
        .map(|spec| {
            let hints = node_hints.get(&spec.node_id);
            let metrics = hints
                .map(|h| h.graph_metrics.clone())
                .unwrap_or_else(|| spec.graph_metrics.clone());
            let pin_policy = hints
                .map(|h| h.pin_policy.clone())
                .unwrap_or_else(PinPolicy::none);
            let priority_class = hints.and_then(|h| h.priority_class).or(spec.priority_class);
            let downstream_nodes = if let Some(h) = hints {
                if h.downstream_nodes.is_empty() {
                    spec.downstream_nodes.clone()
                } else {
                    h.downstream_nodes.clone()
                }
            } else {
                spec.downstream_nodes.clone()
            };
            let reuse_group = hints
                .and_then(|h| h.reuse_group.clone())
                .or_else(|| spec.reuse_group.clone());
            let node_name = hints
                .and_then(|h| h.node_name.clone())
                .or_else(|| spec.node_name.clone());

            NodeDispatchPlan {
                node_id: spec.node_id,
                node_name,
                backend: None,
                model: None,
                priority_class,
                latency_class: metrics.latency_class,
                downstream_nodes,
                prefix_cohort: reuse_group.clone(),
                pin_policy,
                batch_group: metrics.batch_group.clone(),
                fanout_count: metrics.fanout_count,
                remaining_path_len: metrics.remaining_path_len,
                stage_index: metrics.stage_index,
                estimated_dynamic_tokens: metrics.estimated_dynamic_tokens,
                reuse_group,
                compiler_hints: hints.map(|h| h.compiler_hints.clone()).unwrap_or_default(),
                registration_node_name: spec.node_name.clone(),
                registration_estimated_prompt_tokens: spec.estimated_prompt_tokens,
                registration_is_critical_path: Some(spec.is_critical_path),
            }
        })
        .collect();

    DispatchIrV1::new(graph, requirements, nodes, telemetry)
}

/// Derive an `ApxmGraphHints` for one node, suitable for the vLLM
/// `extra_body.vllm_xargs.apxm` payload. Round-trips the fields that already
/// exist in the runtime's wire shape.
pub(crate) fn derive_apxm_hints(ir: &DispatchIrV1, node: &NodeDispatchPlan) -> ApxmGraphHints {
    ApxmGraphHints {
        schema_version: 1,
        graph_id: Some(ir.graph.graph_id.clone()),
        execution_id: ir.graph.execution_id.clone(),
        node_id: Some(node.node_id),
        node_name: node.node_name.clone(),
        priority_class: node.priority_class,
        downstream_nodes: node.downstream_nodes.clone(),
        reuse_group: node.reuse_group.clone(),
        graph_metrics: NodeGraphMetrics {
            fanout_count: node.fanout_count,
            remaining_path_len: node.remaining_path_len,
            latency_class: node.latency_class,
            batch_group: node.batch_group.clone(),
            stage_index: node.stage_index,
            estimated_dynamic_tokens: node.estimated_dynamic_tokens,
        },
        pin_policy: node.pin_policy.clone(),
        compiler_hints: node.compiler_hints.clone(),
    }
}
