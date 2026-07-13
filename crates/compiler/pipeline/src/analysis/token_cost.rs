//! Token-cost facts derived from canonical artifact attributes.

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::Value;
use apxm_core::types::compiler::{CostProvenance, CostSummary};
use apxm_core::types::execution::Node;

use super::evidence::ProfileCostEvidence;

/// Derive token and latency estimates without claiming runtime observations.
pub(super) fn summarize(node: &Node, profile: Option<ProfileCostEvidence>) -> CostSummary {
    let (estimated_latency_ms, latency_provenance, sample_count) = match profile {
        Some(profile) => (
            profile.latency_ms,
            CostProvenance::Observed,
            profile.sample_count,
        ),
        None => match node.metadata.estimated_latency {
            Some(latency) => (
                nanoseconds_to_millis(latency),
                CostProvenance::BackendTier,
                0,
            ),
            None => (1, CostProvenance::OperationCount, 0),
        },
    };
    CostSummary {
        static_prompt_tokens: number(node, graph_attrs::EST_TEMPLATE_TOKENS),
        estimated_dynamic_tokens: profile
            .and_then(|profile| profile.dynamic_tokens)
            .unwrap_or_else(|| number(node, graph_attrs::ESTIMATED_DYNAMIC_TOKENS)),
        max_output_tokens: node
            .get_attribute(graph_attrs::TOKEN_BUDGET)
            .and_then(Value::as_u64),
        estimated_latency_ms,
        latency_provenance,
        sample_count,
    }
}

fn number(node: &Node, name: &str) -> u64 {
    node.get_attribute(name)
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn nanoseconds_to_millis(value: u64) -> u64 {
    value.saturating_add(999_999) / 1_000_000
}
