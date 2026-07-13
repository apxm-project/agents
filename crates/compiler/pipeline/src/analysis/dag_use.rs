//! Execution-DAG dependency facts used by summary construction.

use apxm_core::types::DependencyType;
use apxm_core::types::execution::{ExecutionDag, Node};

/// Return sorted incoming producer identifiers grouped by dependency kind.
pub(super) fn inputs(dag: &ExecutionDag, node: &Node) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
    let mut data = Vec::new();
    let mut effect = Vec::new();
    let mut control = Vec::new();
    for edge in dag.edges.iter().filter(|edge| edge.to == node.id) {
        match edge.dependency_type {
            DependencyType::Data => data.push(edge.from),
            DependencyType::Effect => effect.push(edge.from),
            DependencyType::Control => control.push(edge.from),
        }
    }
    data.sort_unstable();
    effect.sort_unstable();
    control.sort_unstable();
    (data, effect, control)
}

/// Count edges by their typed dependency boundary.
pub(super) fn edge_counts(dag: &ExecutionDag) -> (usize, usize, usize) {
    dag.edges
        .iter()
        .fold((0, 0, 0), |(data, effect, control), edge| {
            match edge.dependency_type {
                DependencyType::Data => (data + 1, effect, control),
                DependencyType::Effect => (data, effect + 1, control),
                DependencyType::Control => (data, effect, control + 1),
            }
        })
}
