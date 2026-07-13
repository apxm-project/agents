//! Backend contract facts derived from authored operation requirements.

use std::collections::{HashMap, HashSet};

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::compiler::{
    BackendLegalityRequirements, EffectAuthoritySummary, TransformationLegality,
};
use apxm_core::types::execution::{ExecutionDag, Node};
use apxm_core::types::{AISOperationType, Value};

use super::evidence::{AnalysisNodeKey, CompilerAnalysisInputs};

/// Derive required backend features without selecting a provider.
pub(super) fn requirements(node: &Node) -> BackendLegalityRequirements {
    BackendLegalityRequirements {
        requires_tools: may_invoke_tools(node),
        requires_structured_output: node.attributes.contains_key(graph_attrs::OUTPUT_SCHEMA),
        requires_prefix_reuse: node
            .get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
            .and_then(Value::as_u64)
            .is_some_and(|value| value > 0),
        requires_thinking: matches!(
            node.op_type,
            AISOperationType::Think | AISOperationType::Reason
        ) || node
            .get_attribute(graph_attrs::EFFORT)
            .and_then(Value::as_str)
            .is_some_and(|effort| effort != "off"),
    }
}

/// Whether the authored node can enter the runtime tool loop.
///
/// Invalid or incomplete tool configuration remains ineligible for caching.
/// The compiler cannot prove that a malformed value disables the runtime path,
/// so the safe result is to require the no-tools contract explicitly.
pub(super) fn may_invoke_tools(node: &Node) -> bool {
    node.attributes.contains_key(graph_attrs::TOOLS_CONFIG)
        || non_empty_array_attribute(node, graph_attrs::TOOLS)
        || non_empty_array_attribute(node, graph_attrs::CAPABILITY_GROUPS)
        || enabled_boolean_attribute(node, graph_attrs::TOOLS_ENABLED)
        || enabled_boolean_attribute(node, graph_attrs::ENABLE_DELEGATE)
        || positive_integer_attribute(node, graph_attrs::MAX_TOOL_ITERATIONS)
}

/// Derive per-node transformation legality for one execution DAG.
///
/// Memoization additionally requires an effect-safe transitive input chain.
/// The runtime cache keys the fully rendered request, but it must never turn a
/// capability, approval, or replay-bound producer into an implicit cache hit.
pub(super) fn transformation_legalities(
    dag: &ExecutionDag,
    effects: &[EffectAuthoritySummary],
    requirements: &[BackendLegalityRequirements],
    inputs: &CompilerAnalysisInputs,
    dag_index: usize,
) -> Vec<TransformationLegality> {
    let node_indexes: HashMap<_, _> = dag
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect();

    dag.nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let mut legality = transformation_legality(&effects[index]);
            let key = AnalysisNodeKey {
                dag_index,
                node_id: node.id,
            };
            let execution_ready = inputs.execution_ready(key, dag, node, &effects[index]);
            let confinement_satisfied = inputs.confinement_satisfied(key, node);
            legality.may_reorder &= execution_ready;
            legality.may_duplicate &= execution_ready && confinement_satisfied;
            legality.may_speculate &= execution_ready && confinement_satisfied;
            legality.may_memoize = legality.may_memoize
                && !requirements[index].requires_tools
                && inputs.supports_backend_contract(node, &requirements[index])
                && execution_ready
                && confinement_satisfied
                && inputs_are_memoization_safe(dag, node, effects, &node_indexes);
            legality.may_batch =
                batch_candidate(dag, index, effects, requirements, inputs, dag_index);
            legality
        })
        .collect()
}

/// Whether one model call has an independent same-route peer for legal batching.
fn batch_candidate(
    dag: &ExecutionDag,
    node_index: usize,
    effects: &[EffectAuthoritySummary],
    requirements: &[BackendLegalityRequirements],
    inputs: &CompilerAnalysisInputs,
    dag_index: usize,
) -> bool {
    let node = &dag.nodes[node_index];
    let key = AnalysisNodeKey {
        dag_index,
        node_id: node.id,
    };
    if !batch_safe_effect(&effects[node_index])
        || requirements[node_index].requires_tools
        || !inputs.supports_backend_contract(node, &requirements[node_index])
        || !inputs.supports_batching(node)
        || !inputs.execution_ready(key, dag, node, &effects[node_index])
        || !inputs.confinement_satisfied(key, node)
    {
        return false;
    }

    dag.nodes.iter().enumerate().any(|(peer_index, peer)| {
        if peer_index == node_index
            || peer.op_type != node.op_type
            || !requirements_compatible(&requirements[node_index], &requirements[peer_index])
            || !inputs.same_configured_backend(node, peer)
            || !batch_safe_effect(&effects[peer_index])
            || !inputs.supports_backend_contract(peer, &requirements[peer_index])
            || !inputs.supports_batching(peer)
        {
            return false;
        }
        let peer_key = AnalysisNodeKey {
            dag_index,
            node_id: peer.id,
        };
        inputs.execution_ready(peer_key, dag, peer, &effects[peer_index])
            && inputs.confinement_satisfied(peer_key, peer)
            && dependency_independent(dag, node.id, peer.id)
    })
}

/// Whether an operation can share transport without moving an observable effect.
fn batch_safe_effect(effect: &EffectAuthoritySummary) -> bool {
    effect.writes.is_empty()
        && effect.permission_operations.is_empty()
        && !effect.approval_required
        && effect.determinism == apxm_core::types::compiler::Determinism::Proven
        && effect.idempotent
        && effect.replay_safety == apxm_core::types::compiler::ReplaySafety::Safe
}

/// Whether two requests require the same backend request-shape capabilities.
fn requirements_compatible(
    left: &BackendLegalityRequirements,
    right: &BackendLegalityRequirements,
) -> bool {
    left.requires_tools == right.requires_tools
        && left.requires_structured_output == right.requires_structured_output
        && left.requires_prefix_reuse == right.requires_prefix_reuse
        && left.requires_thinking == right.requires_thinking
}

/// Whether neither node is upstream of the other through any typed dependency.
pub(super) fn dependency_independent(dag: &ExecutionDag, left: u64, right: u64) -> bool {
    !has_path(dag, left, right) && !has_path(dag, right, left)
}

/// Whether the emitted DAG contains a typed path from one node to another.
fn has_path(dag: &ExecutionDag, from: u64, to: u64) -> bool {
    let mut pending = vec![from];
    let mut visited = HashSet::from([from]);
    while let Some(current) = pending.pop() {
        for edge in dag.edges.iter().filter(|edge| edge.from == current) {
            if edge.to == to {
                return true;
            }
            if visited.insert(edge.to) {
                pending.push(edge.to);
            }
        }
    }
    false
}

/// Transform legality is enabled only for operations proven side-effect-free.
pub(super) fn transformation_legality(effect: &EffectAuthoritySummary) -> TransformationLegality {
    let pure = effect.reads.is_empty()
        && effect.writes.is_empty()
        && effect.permission_operations.is_empty()
        && !effect.approval_required
        && effect.idempotent;
    let memoization_safe = effect.writes.is_empty()
        && effect.permission_operations.is_empty()
        && !effect.approval_required
        && effect.determinism == apxm_core::types::compiler::Determinism::Proven
        && effect.idempotent
        && effect.replay_safety == apxm_core::types::compiler::ReplaySafety::Safe;
    TransformationLegality {
        may_reorder: pure,
        may_duplicate: pure,
        may_memoize: memoization_safe,
        may_batch: false,
        may_speculate: pure,
    }
}

/// Whether every producer that can influence this request is safe to reuse.
fn inputs_are_memoization_safe(
    dag: &ExecutionDag,
    node: &Node,
    effects: &[EffectAuthoritySummary],
    node_indexes: &HashMap<u64, usize>,
) -> bool {
    let mut pending = vec![node.id];
    let mut visited = HashSet::from([node.id]);

    while let Some(target) = pending.pop() {
        for edge in dag.edges.iter().filter(|edge| edge.to == target) {
            let Some(&source_index) = node_indexes.get(&edge.from) else {
                return false;
            };
            if !memoization_safe_producer(&effects[source_index]) {
                return false;
            }
            if visited.insert(edge.from) {
                pending.push(edge.from);
            }
        }
    }

    true
}

/// Whether a producer can contribute to an exact memoized model request.
fn memoization_safe_producer(effect: &EffectAuthoritySummary) -> bool {
    effect.writes.is_empty()
        && effect.permission_operations.is_empty()
        && !effect.approval_required
        && effect.determinism != apxm_core::types::compiler::Determinism::NonDeterministic
        && effect.replay_safety == apxm_core::types::compiler::ReplaySafety::Safe
}

/// Whether an array-backed tool selector can resolve one or more tools.
fn non_empty_array_attribute(node: &Node, attribute: &str) -> bool {
    node.get_attribute(attribute)
        .is_some_and(|value| value.as_array().map_or(true, |values| !values.is_empty()))
}

/// Whether a boolean tool switch is enabled or cannot be proven disabled.
fn enabled_boolean_attribute(node: &Node, attribute: &str) -> bool {
    !matches!(
        node.get_attribute(attribute),
        None | Some(Value::Bool(false))
    )
}

/// Whether a tool-loop limit is positive or malformed.
fn positive_integer_attribute(node: &Node, attribute: &str) -> bool {
    node.get_attribute(attribute)
        .is_some_and(|value| value.as_u64().map_or(true, |limit| limit > 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::compiler::{Determinism, ReplaySafety};

    #[test]
    fn batching_requires_deterministic_idempotent_replay_safe_effects() {
        let safe = EffectAuthoritySummary {
            determinism: Determinism::Proven,
            idempotent: true,
            replay_safety: ReplaySafety::Safe,
            ..Default::default()
        };
        let mut nondeterministic = safe.clone();
        nondeterministic.determinism = Determinism::NonDeterministic;
        let mut replay_bound = safe.clone();
        replay_bound.replay_safety = ReplaySafety::RequiresCheckpoint;

        assert!(batch_safe_effect(&safe));
        assert!(!batch_safe_effect(&nondeterministic));
        assert!(!batch_safe_effect(&replay_bound));
    }
}
