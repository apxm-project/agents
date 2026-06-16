//! Partial replay seeding for `rerun-from-node`.
//!
//! Genuine partial replay re-executes only a chosen node and its descendants
//! while reusing the prior run's outputs for everything upstream. This module
//! turns a prior run's captured token values into a [`ReplaySeed`] the scheduler
//! applies when it builds state: the upstream nodes are pre-completed (never
//! enqueued, so their handlers are never re-invoked) and the token values they
//! produced are seeded ready so the sub-DAG rooted at `from_node` sees the same
//! inputs it saw originally.
//!
//! The seed is computed against the *recompiled* DAG, so it is only valid when
//! the recompiled graph carries the same node/token ids as the prior run (the
//! deterministic skill-compilation case). When `from_node` is not in the graph,
//! seed construction returns `None` and the caller falls back to a full re-run.

use std::collections::{HashMap, HashSet};

use apxm_core::types::{ExecutionDag, NodeId, TokenId, Value};

/// A computed plan for partially replaying a prior run from a chosen node.
///
/// Built by [`ReplaySeed::compute`]. Applying it to a fresh [`super::state::SchedulerState`]
/// pre-completes the upstream nodes and seeds the boundary token values, so only
/// `from_node` and its descendants execute.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplaySeed {
    /// The node the replay restarts from.
    pub from_node: NodeId,
    /// Nodes that are re-executed: `from_node` and everything reachable from it.
    pub replayed_nodes: HashSet<NodeId>,
    /// Nodes that are NOT re-executed; they are marked completed up front.
    pub completed_nodes: HashSet<NodeId>,
    /// Token values to seed ready before execution: the outputs of completed
    /// nodes that feed into the replayed sub-DAG (the replay boundary).
    pub seed_tokens: HashMap<TokenId, Value>,
}

impl ReplaySeed {
    /// Compute a replay seed for `from_node` against `dag`, drawing prior token
    /// values from `prior_token_values` (keyed by token id, as captured on the
    /// original run).
    ///
    /// Returns `None` when `from_node` is not a node in `dag` (nothing to replay
    /// from — the caller should fall back to a full re-run).
    ///
    /// A seed is still returned when some boundary token values are missing from
    /// `prior_token_values`; those tokens are simply not seeded (they fall back
    /// to the scheduler's default `Null` for unproduced inputs). Callers that
    /// require complete boundary coverage can check [`ReplaySeed::is_complete`].
    pub fn compute(
        dag: &ExecutionDag,
        from_node: NodeId,
        prior_token_values: &HashMap<TokenId, Value>,
    ) -> Option<Self> {
        if !dag.nodes.iter().any(|node| node.id == from_node) {
            return None;
        }

        let replayed_nodes = descendants_inclusive(dag, from_node);
        let completed_nodes: HashSet<NodeId> = dag
            .nodes
            .iter()
            .map(|node| node.id)
            .filter(|id| !replayed_nodes.contains(id))
            .collect();

        // The replay boundary is every token that a completed node produces and a
        // replayed node consumes. Seed those tokens with their prior values so the
        // sub-DAG observes identical inputs.
        let completed_outputs: HashSet<TokenId> = dag
            .nodes
            .iter()
            .filter(|node| completed_nodes.contains(&node.id))
            .flat_map(|node| node.output_tokens.iter().copied())
            .collect();

        let replayed_inputs: HashSet<TokenId> = dag
            .nodes
            .iter()
            .filter(|node| replayed_nodes.contains(&node.id))
            .flat_map(|node| node.input_tokens.iter().copied())
            .collect();

        let seed_tokens = completed_outputs
            .intersection(&replayed_inputs)
            .filter_map(|token_id| {
                prior_token_values
                    .get(token_id)
                    .map(|value| (*token_id, value.clone()))
            })
            .collect();

        Some(Self {
            from_node,
            replayed_nodes,
            completed_nodes,
            seed_tokens,
        })
    }

    /// Whether every replay-boundary token has a seeded value. When `false`, some
    /// upstream value was not captured and the sub-DAG would observe `Null` for
    /// it; the caller may prefer a full re-run in that case.
    pub fn is_complete(&self, dag: &ExecutionDag) -> bool {
        let completed_outputs: HashSet<TokenId> = dag
            .nodes
            .iter()
            .filter(|node| self.completed_nodes.contains(&node.id))
            .flat_map(|node| node.output_tokens.iter().copied())
            .collect();
        dag.nodes
            .iter()
            .filter(|node| self.replayed_nodes.contains(&node.id))
            .flat_map(|node| node.input_tokens.iter().copied())
            .filter(|token_id| completed_outputs.contains(token_id))
            .all(|token_id| self.seed_tokens.contains_key(&token_id))
    }

    /// Number of nodes that will actually execute on the replay.
    pub fn replayed_count(&self) -> usize {
        self.replayed_nodes.len()
    }

    /// Decode a replay seed from execution metadata for `dag`.
    ///
    /// The host stamps [`REPLAY_FROM_NODE`](crate::metadata_keys::REPLAY_FROM_NODE)
    /// (the node id) and
    /// [`REPLAY_TOKEN_VALUES`](crate::metadata_keys::REPLAY_TOKEN_VALUES)
    /// (a JSON object `{token_id: Value}` of the prior run's captured token
    /// values) into the execution metadata. This decodes those keys and computes
    /// the seed against the recompiled `dag`.
    ///
    /// Returns `None` (i.e. a full re-run) when the keys are absent, malformed,
    /// or `from_node` is not a node in `dag`. Both scheduler entry paths — the
    /// server path in [`Runtime::execute_artifact_inner`](crate::runtime::Runtime)
    /// and the [`ExecutorEngine`](crate::executor::ExecutorEngine) fallback path —
    /// route through this so they honor partial replay identically.
    pub fn from_metadata(
        metadata: &HashMap<String, String>,
        dag: &ExecutionDag,
    ) -> Option<Self> {
        let from_node: NodeId = metadata
            .get(crate::metadata_keys::REPLAY_FROM_NODE)?
            .parse()
            .ok()?;
        let values_json = metadata.get(crate::metadata_keys::REPLAY_TOKEN_VALUES)?;
        let raw: HashMap<String, Value> = serde_json::from_str(values_json).ok()?;
        let prior_values: HashMap<TokenId, Value> = raw
            .into_iter()
            .filter_map(|(key, value)| key.parse::<TokenId>().ok().map(|id| (id, value)))
            .collect();
        Self::compute(dag, from_node, &prior_values)
    }
}

/// Collect `root` plus every node reachable from it by following data edges
/// (token producer -> consumer). Used to decide which nodes a partial replay
/// re-executes.
fn descendants_inclusive(dag: &ExecutionDag, root: NodeId) -> HashSet<NodeId> {
    // token_id -> consumer node ids
    let mut consumers: HashMap<TokenId, Vec<NodeId>> = HashMap::new();
    for node in &dag.nodes {
        for &token_id in &node.input_tokens {
            consumers.entry(token_id).or_default().push(node.id);
        }
    }
    let node_outputs: HashMap<NodeId, &Vec<TokenId>> =
        dag.nodes.iter().map(|n| (n.id, &n.output_tokens)).collect();

    let mut reachable = HashSet::new();
    let mut stack = vec![root];
    while let Some(node_id) = stack.pop() {
        if !reachable.insert(node_id) {
            continue;
        }
        if let Some(outputs) = node_outputs.get(&node_id) {
            for token_id in *outputs {
                if let Some(downstream) = consumers.get(token_id) {
                    for &consumer in downstream {
                        if !reachable.contains(&consumer) {
                            stack.push(consumer);
                        }
                    }
                }
            }
        }
    }
    reachable
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::{DependencyType, Edge, ExecutionDag, Node, NodeMetadata};
    use std::collections::HashMap as Map;

    fn node(id: NodeId, inputs: Vec<TokenId>, outputs: Vec<TokenId>) -> Node {
        Node {
            id,
            op_type: AISOperationType::Nop,
            attributes: Map::new(),
            input_tokens: inputs,
            output_tokens: outputs,
            metadata: NodeMetadata::default(),
        }
    }

    /// Linear chain: 1 --t10--> 2 --t20--> 3
    fn chain_dag() -> ExecutionDag {
        let mut dag = ExecutionDag::new();
        dag.add_node(node(1, vec![], vec![10])).unwrap();
        dag.add_node(node(2, vec![10], vec![20])).unwrap();
        dag.add_node(node(3, vec![20], vec![30])).unwrap();
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .unwrap();
        dag.add_edge(Edge::new(2, 3, 20, DependencyType::Data))
            .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag
    }

    #[test]
    fn compute_returns_none_for_unknown_node() {
        let dag = chain_dag();
        assert!(ReplaySeed::compute(&dag, 99, &Map::new()).is_none());
    }

    #[test]
    fn replay_from_middle_keeps_node_and_descendants() {
        let dag = chain_dag();
        let mut prior = Map::new();
        prior.insert(10, Value::String("from-node-1".into()));
        let seed = ReplaySeed::compute(&dag, 2, &prior).unwrap();

        // Nodes 2 and 3 replay; node 1 is completed (not re-run).
        assert_eq!(seed.replayed_nodes, HashSet::from([2, 3]));
        assert_eq!(seed.completed_nodes, HashSet::from([1]));
        // The boundary token (10, node 1 -> node 2) is seeded.
        assert_eq!(
            seed.seed_tokens.get(&10),
            Some(&Value::String("from-node-1".into()))
        );
        assert!(seed.is_complete(&dag));
        assert_eq!(seed.replayed_count(), 2);
    }

    #[test]
    fn replay_from_entry_keeps_whole_graph() {
        let dag = chain_dag();
        let seed = ReplaySeed::compute(&dag, 1, &Map::new()).unwrap();
        assert_eq!(seed.replayed_nodes, HashSet::from([1, 2, 3]));
        assert!(seed.completed_nodes.is_empty());
        assert!(seed.seed_tokens.is_empty());
    }

    #[test]
    fn missing_boundary_value_marks_incomplete() {
        let dag = chain_dag();
        // No prior value for the boundary token 10.
        let seed = ReplaySeed::compute(&dag, 2, &Map::new()).unwrap();
        assert!(seed.seed_tokens.is_empty());
        assert!(!seed.is_complete(&dag));
    }

    #[test]
    fn diamond_replay_collects_all_descendants() {
        // 1 fans to 2 and 3, both feed 4.
        //   1 --t10--> 2 --t20--> 4
        //   1 --t11--> 3 --t30--> 4
        let mut dag = ExecutionDag::new();
        dag.add_node(node(1, vec![], vec![10, 11])).unwrap();
        dag.add_node(node(2, vec![10], vec![20])).unwrap();
        dag.add_node(node(3, vec![11], vec![30])).unwrap();
        dag.add_node(node(4, vec![20, 30], vec![40])).unwrap();
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .unwrap();
        dag.add_edge(Edge::new(1, 3, 11, DependencyType::Data))
            .unwrap();
        dag.add_edge(Edge::new(2, 4, 20, DependencyType::Data))
            .unwrap();
        dag.add_edge(Edge::new(3, 4, 30, DependencyType::Data))
            .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let mut prior = Map::new();
        prior.insert(11, Value::String("branch-b".into()));
        // Replay from node 3: node 4 is a descendant; nodes 1 and 2 are completed.
        let seed = ReplaySeed::compute(&dag, 3, &prior).unwrap();
        assert_eq!(seed.replayed_nodes, HashSet::from([3, 4]));
        assert_eq!(seed.completed_nodes, HashSet::from([1, 2]));
        // Boundary tokens into the replayed set: 11 (1->3) and 20 (2->4).
        assert_eq!(
            seed.seed_tokens.get(&11),
            Some(&Value::String("branch-b".into()))
        );
        // Token 20 has no prior value -> not seeded, seed incomplete.
        assert!(!seed.seed_tokens.contains_key(&20));
        assert!(!seed.is_complete(&dag));
    }
}
