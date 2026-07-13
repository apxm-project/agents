//! Additive, conservative optimization evidence derived from execution DAGs.

mod backend_legality;
mod dag_use;
mod effect_authority;
mod profile_cost;
mod prompt_contract;
mod store;
mod token_cost;

use apxm_core::types::compiler::OptimizationSummaryV1;
use apxm_core::types::execution::ExecutionDag;

pub(crate) use store::AnalysisStore;

/// Build the versioned summary embedded beside an executable artifact.
pub(crate) fn summarize(dags: &[ExecutionDag]) -> OptimizationSummaryV1 {
    AnalysisStore::new(dags.to_vec()).summary()
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::execution::{Edge, Node};
    use apxm_core::types::{AISOperationType, DependencyType};

    #[test]
    fn capability_invocation_fails_closed_for_reordering_and_memoization() {
        let mut dag = ExecutionDag::new();
        dag.nodes.push(Node::new(1, AISOperationType::InvCap));

        let summary = summarize(&[dag]);
        let legality = &summary.dags[0].nodes[0].legality;
        assert!(!legality.may_reorder);
        assert!(!legality.may_duplicate);
        assert!(!legality.may_memoize);
    }

    #[test]
    fn critical_path_uses_node_latency_and_typed_edges() {
        let mut first = Node::new(1, AISOperationType::Nop);
        first.metadata.estimated_latency = Some(2_000_000);
        let mut second = Node::new(2, AISOperationType::Nop);
        second.metadata.estimated_latency = Some(3_000_000);
        let mut dag = ExecutionDag::new();
        dag.nodes = vec![first, second];
        dag.edges.push(Edge::new(1, 2, 1, DependencyType::Effect));

        let summary = summarize(&[dag]);
        assert_eq!(summary.dags[0].effect_edges, 1);
        assert_eq!(summary.dags[0].weighted_critical_path_ms, 5);
    }
}
