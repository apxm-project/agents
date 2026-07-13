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
use std::fmt;

use apxm_core::types::operations::AISOperationType;
use apxm_core::types::{ExecutionDag, NodeId, TokenId, Value};

/// A partial replay request cannot preserve the original execution boundary.
///
/// Token values prove dataflow parity only. They are not evidence that a
/// skipped operation's authority, approval, or durable effect remains valid in
/// the new execution. Until the runtime persists and verifies that evidence,
/// partial replay is limited to data-only completed subgraphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayRejection {
    /// The replay metadata was incomplete or could not be decoded.
    InvalidMetadata { reason: &'static str },
    /// The metadata named a node that is absent from the recompiled DAG.
    UnknownRestartNode { node_id: NodeId },
    /// A token crossing from the completed subgraph into the replayed subgraph
    /// was not captured by the prior run.
    IncompleteBoundary { missing_tokens: Vec<TokenId> },
    /// Skipping this operation could bypass an approval, authority check, or
    /// durable effect whose prior evidence is not available to the replayer.
    UnsafeCompletedOperation {
        node_id: NodeId,
        operation: AISOperationType,
    },
}

impl fmt::Display for ReplayRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetadata { reason } => {
                write!(formatter, "invalid partial replay metadata: {reason}")
            }
            Self::UnknownRestartNode { node_id } => {
                write!(
                    formatter,
                    "partial replay restart node {node_id} is absent from the recompiled DAG"
                )
            }
            Self::IncompleteBoundary { missing_tokens } => write!(
                formatter,
                "partial replay is missing captured values for boundary tokens {missing_tokens:?}"
            ),
            Self::UnsafeCompletedOperation { node_id, operation } => write!(
                formatter,
                "partial replay cannot skip completed node {node_id} ({operation:?}) without persisted effect and approval evidence"
            ),
        }
    }
}

impl std::error::Error for ReplayRejection {}

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
        self.missing_boundary_tokens(dag).is_empty()
    }

    /// Return every token whose prior value is required at the replay boundary
    /// but was not captured by the source execution.
    pub fn missing_boundary_tokens(&self, dag: &ExecutionDag) -> Vec<TokenId> {
        let completed_outputs: HashSet<TokenId> = dag
            .nodes
            .iter()
            .filter(|node| self.completed_nodes.contains(&node.id))
            .flat_map(|node| node.output_tokens.iter().copied())
            .collect();
        let mut missing = dag
            .nodes
            .iter()
            .filter(|node| self.replayed_nodes.contains(&node.id))
            .flat_map(|node| node.input_tokens.iter().copied())
            .filter(|token_id| completed_outputs.contains(token_id))
            .filter(|token_id| !self.seed_tokens.contains_key(token_id))
            .collect::<Vec<_>>();
        missing.sort_unstable();
        missing.dedup();
        missing
    }

    /// Validate that a seed can be used for partial replay without silently
    /// bypassing an unrecorded authority or effect boundary.
    ///
    /// This is deliberately conservative. The runtime has no durable,
    /// replay-verifiable record for completed approvals, effects, or
    /// nondeterministic results yet, so only a strict allow-list of inert
    /// dataflow operations may be skipped. A caller may choose a full run
    /// instead.
    pub fn validate_partial_replay(&self, dag: &ExecutionDag) -> Result<(), ReplayRejection> {
        let missing_tokens = self.missing_boundary_tokens(dag);
        if !missing_tokens.is_empty() {
            return Err(ReplayRejection::IncompleteBoundary { missing_tokens });
        }

        if let Some(node) = dag.nodes.iter().find(|node| {
            self.completed_nodes.contains(&node.id) && !operation_is_safe_to_skip(node.op_type)
        }) {
            return Err(ReplayRejection::UnsafeCompletedOperation {
                node_id: node.id,
                operation: node.op_type,
            });
        }

        Ok(())
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
    /// This compatibility helper returns `None` when the request cannot be
    /// decoded or validated. Production scheduler entry points use
    /// [`Self::from_metadata_checked`] so an explicit invalid or unsafe request
    /// is returned to the caller rather than being downgraded silently.
    pub fn from_metadata(metadata: &HashMap<String, String>, dag: &ExecutionDag) -> Option<Self> {
        Self::from_metadata_checked(metadata, dag).ok().flatten()
    }

    /// Decode and validate a partial replay request from execution metadata.
    ///
    /// Missing replay metadata means a normal full run. A malformed request,
    /// incomplete boundary, or skipped authority/effect operation is an explicit
    /// rejection instead of silently turning a requested partial replay into an
    /// unsafe execution.
    pub fn from_metadata_checked(
        metadata: &HashMap<String, String>,
        dag: &ExecutionDag,
    ) -> Result<Option<Self>, ReplayRejection> {
        let Some(from_node_raw) = metadata.get(crate::metadata_keys::REPLAY_FROM_NODE) else {
            return Ok(None);
        };
        let from_node: NodeId =
            from_node_raw
                .parse()
                .map_err(|_| ReplayRejection::InvalidMetadata {
                    reason: "replay_from_node must be a node id",
                })?;
        let values_json = metadata
            .get(crate::metadata_keys::REPLAY_TOKEN_VALUES)
            .ok_or(ReplayRejection::InvalidMetadata {
                reason: "replay_token_values is required with replay_from_node",
            })?;
        let raw: HashMap<String, Value> =
            serde_json::from_str(values_json).map_err(|_| ReplayRejection::InvalidMetadata {
                reason: "replay_token_values must be a JSON object",
            })?;
        let prior_values: HashMap<TokenId, Value> = raw
            .into_iter()
            .filter_map(|(key, value)| key.parse::<TokenId>().ok().map(|id| (id, value)))
            .collect();
        let seed = Self::compute(dag, from_node, &prior_values)
            .ok_or(ReplayRejection::UnknownRestartNode { node_id: from_node })?;
        seed.validate_partial_replay(dag)?;
        Ok(Some(seed))
    }
}

/// Operations whose skipped execution is data-only and deterministic under the
/// scheduler's existing token model. Every other operation requires a
/// persisted, replay-verifiable receipt before a partial replay may skip it.
fn operation_is_safe_to_skip(operation: AISOperationType) -> bool {
    matches!(
        operation,
        AISOperationType::ConstStr
            | AISOperationType::Nop
            | AISOperationType::Identity
            | AISOperationType::Merge
            | AISOperationType::WaitAll
            | AISOperationType::Fence
    )
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

    #[test]
    fn checked_replay_rejects_an_incomplete_data_boundary() {
        let dag = chain_dag();
        let metadata = HashMap::from([
            (
                crate::metadata_keys::REPLAY_FROM_NODE.to_string(),
                "2".to_string(),
            ),
            (
                crate::metadata_keys::REPLAY_TOKEN_VALUES.to_string(),
                "{}".to_string(),
            ),
        ]);

        assert_eq!(
            ReplaySeed::from_metadata_checked(&metadata, &dag),
            Err(ReplayRejection::IncompleteBoundary {
                missing_tokens: vec![10]
            })
        );
    }

    #[test]
    fn checked_replay_rejects_skipping_capability_effects_without_a_receipt() {
        let mut dag = chain_dag();
        dag.nodes[0].op_type = AISOperationType::InvCap;
        let metadata = HashMap::from([
            (
                crate::metadata_keys::REPLAY_FROM_NODE.to_string(),
                "2".to_string(),
            ),
            (
                crate::metadata_keys::REPLAY_TOKEN_VALUES.to_string(),
                serde_json::json!({"10": "prior-capability-output"}).to_string(),
            ),
        ]);

        assert_eq!(
            ReplaySeed::from_metadata_checked(&metadata, &dag),
            Err(ReplayRejection::UnsafeCompletedOperation {
                node_id: 1,
                operation: AISOperationType::InvCap,
            })
        );
    }

    #[test]
    fn checked_replay_keeps_data_only_boundary_and_expected_node_set() {
        let dag = chain_dag();
        let metadata = HashMap::from([
            (
                crate::metadata_keys::REPLAY_FROM_NODE.to_string(),
                "2".to_string(),
            ),
            (
                crate::metadata_keys::REPLAY_TOKEN_VALUES.to_string(),
                serde_json::json!({"10": "prior-data-output"}).to_string(),
            ),
        ]);

        let seed = ReplaySeed::from_metadata_checked(&metadata, &dag)
            .expect("safe data-only replay")
            .expect("replay metadata is present");
        assert_eq!(seed.completed_nodes, HashSet::from([1]));
        assert_eq!(seed.replayed_nodes, HashSet::from([2, 3]));
        assert_eq!(
            seed.seed_tokens.get(&10),
            Some(&Value::String("prior-data-output".to_string()))
        );
    }
}
