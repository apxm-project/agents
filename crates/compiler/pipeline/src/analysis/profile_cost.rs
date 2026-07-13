//! Critical-path calculation from emitted per-node latency evidence.

use std::collections::{BTreeSet, HashMap, VecDeque};

use apxm_core::types::compiler::CostSummary;
use apxm_core::types::execution::ExecutionDag;

/// Compute a deterministic weighted critical path across all typed edges.
pub(super) fn weighted_critical_path_ms(dag: &ExecutionDag, costs: &[CostSummary]) -> u64 {
    let mut remaining: HashMap<u64, usize> = dag.nodes.iter().map(|node| (node.id, 0)).collect();
    let mut successors: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut predecessors: HashMap<u64, Vec<u64>> = HashMap::new();
    for edge in &dag.edges {
        if remaining.contains_key(&edge.from) && remaining.contains_key(&edge.to) {
            *remaining.entry(edge.to).or_default() += 1;
            successors.entry(edge.from).or_default().push(edge.to);
            predecessors.entry(edge.to).or_default().push(edge.from);
        }
    }

    let mut ready: BTreeSet<u64> = remaining
        .iter()
        .filter_map(|(node_id, degree)| (*degree == 0).then_some(*node_id))
        .collect();
    let weights: HashMap<u64, u64> = dag
        .nodes
        .iter()
        .zip(costs)
        .map(|(node, cost)| (node.id, cost.estimated_latency_ms))
        .collect();
    let mut distance = HashMap::new();
    let mut queue = VecDeque::new();
    while let Some(node_id) = ready.pop_first() {
        queue.push_back(node_id);
    }

    while let Some(node_id) = queue.pop_front() {
        let predecessor_max = predecessors
            .get(&node_id)
            .into_iter()
            .flatten()
            .filter_map(|predecessor| distance.get(predecessor))
            .copied()
            .max()
            .unwrap_or(0_u64);
        distance.insert(
            node_id,
            predecessor_max.saturating_add(*weights.get(&node_id).unwrap_or(&1)),
        );
        if let Some(next_nodes) = successors.get(&node_id) {
            let mut ordered = next_nodes.clone();
            ordered.sort_unstable();
            for next in ordered {
                let degree = remaining.get_mut(&next).expect("known successor");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(next);
                }
            }
        }
    }

    distance.values().copied().max().unwrap_or(0)
}
