//! Backend contract facts derived from authored operation requirements.

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::compiler::{
    BackendLegalityRequirements, EffectAuthoritySummary, TransformationLegality,
};
use apxm_core::types::execution::Node;
use apxm_core::types::{AISOperationType, Value};

/// Derive required backend features without selecting a provider.
pub(super) fn requirements(node: &Node) -> BackendLegalityRequirements {
    BackendLegalityRequirements {
        requires_tools: node
            .get_attribute(graph_attrs::MAX_TOOL_ITERATIONS)
            .and_then(Value::as_u64)
            .is_some_and(|value| value > 0)
            || node.attributes.contains_key(graph_attrs::TOOLS_CONFIG),
        requires_structured_output: node.attributes.contains_key(graph_attrs::OUTPUT_SCHEMA),
        requires_prefix_reuse: node
            .get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
            .and_then(Value::as_u64)
            .is_some_and(|value| value > 0),
        requires_thinking: matches!(node.op_type, AISOperationType::Reason),
    }
}

/// Transform legality is enabled only for operations proven side-effect-free.
pub(super) fn transformation_legality(effect: &EffectAuthoritySummary) -> TransformationLegality {
    let pure = effect.reads.is_empty()
        && effect.writes.is_empty()
        && effect.permission_operations.is_empty()
        && effect.idempotent;
    TransformationLegality {
        may_reorder: pure,
        may_duplicate: pure,
        may_memoize: pure,
        may_batch: false,
        may_speculate: pure,
    }
}
