//! Token-cost facts derived from canonical artifact attributes.

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::Value;
use apxm_core::types::compiler::{CostProvenance, CostSummary};
use apxm_core::types::execution::Node;

/// Derive token and latency estimates without claiming runtime observations.
pub(super) fn summarize(node: &Node) -> CostSummary {
    CostSummary {
        static_prompt_tokens: number(node, graph_attrs::EST_TEMPLATE_TOKENS),
        estimated_dynamic_tokens: number(node, graph_attrs::ESTIMATED_DYNAMIC_TOKENS),
        max_output_tokens: node
            .get_attribute(graph_attrs::TOKEN_BUDGET)
            .and_then(Value::as_u64),
        estimated_latency_ms: node
            .metadata
            .estimated_latency
            .map(nanoseconds_to_millis)
            .unwrap_or(1),
        latency_provenance: if node.metadata.estimated_latency.is_some() {
            CostProvenance::Observed
        } else {
            CostProvenance::OperationCount
        },
        sample_count: 0,
    }
}

/// Return the scheduler weight used when a profile did not provide latency.
pub(super) fn latency_weight_ms(node: &Node) -> u64 {
    summarize(node).estimated_latency_ms
}

fn number(node: &Node, name: &str) -> u64 {
    node.get_attribute(name)
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn nanoseconds_to_millis(value: u64) -> u64 {
    value.saturating_add(999_999) / 1_000_000
}
