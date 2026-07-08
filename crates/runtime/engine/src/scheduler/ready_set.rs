//! O(1) readiness tracking for dataflow operations.
//!
//! This module provides efficient tracking of operation readiness using pending input counts.
//! When an operation's pending count reaches 0, it becomes ready to execute.

use std::sync::Arc;

use apxm_core::types::{Node, NodeId, OpStatus, TokenId};
use dashmap::DashMap;

use crate::scheduler::internal_state::{OpState, TokenState};
use crate::scheduler::queue::{Priority, PriorityQueue};
use apxm_core::error::RuntimeError;

/// Tracks operation readiness using pending input counts.
///
/// Each operation starts with a count of how many inputs it's waiting for.
/// As inputs become ready, the count decrements. When it reaches 0, the
/// operation is marked ready and enqueued.
type RuntimeResult<T> = std::result::Result<T, RuntimeError>;

pub(crate) struct ReadySet {
    /// Number of pending inputs per operation.
    ///
    /// Operations with count = 0 are ready to execute.
    /// Missing entries are treated as ready (0 pending).
    pending_inputs: Arc<DashMap<NodeId, usize>>,
}

impl ReadySet {
    /// Create a new empty ready set.
    pub fn new() -> Self {
        Self {
            pending_inputs: Arc::new(DashMap::new()),
        }
    }

    /// Insert or update a pending input count for a node.
    ///
    /// This helper is intended for dynamic graph updates (e.g. DAG splicing)
    /// so callers can register a node with its initial missing-inputs count.
    /// If `count` is zero, the node is not inserted (it should be enqueued instead).
    pub(crate) fn insert_pending(&self, node_id: NodeId, count: usize) {
        if count == 0 {
            return;
        }
        self.pending_inputs.insert(node_id, count);
    }

    /// Remove a node from pending tracking.
    ///
    /// Cleans up nodes during DAG condensation deletion.
    pub(crate) fn remove_pending(&self, node_id: NodeId) {
        self.pending_inputs.remove(&node_id);
    }

    /// Initialize readiness tracking for all nodes in the graph.
    ///
    /// Returns the set of immediately ready nodes (those with no pending inputs).
    /// Initialize readiness tracking for all nodes; nodes in `skip` are neither
    /// enqueued nor tracked for readiness. Returns the set of immediately ready
    /// nodes. Partial replay (`rerun-from-node`) skips pre-completed upstream
    /// nodes so their handlers are never
    /// re-invoked, while their seeded output tokens still satisfy the readiness
    /// of the replayed sub-DAG. Pass `None` for an ordinary full run.
    pub(crate) fn initialize_with_skip(
        &self,
        nodes: &[Node],
        tokens: &DashMap<TokenId, TokenState>,
        priorities: &DashMap<NodeId, Priority>,
        op_states: &DashMap<NodeId, OpState>,
        queue: &PriorityQueue,
        skip: Option<&std::collections::HashSet<NodeId>>,
    ) -> RuntimeResult<Vec<NodeId>> {
        let mut ready_nodes = Vec::new();

        for node in nodes {
            if skip.is_some_and(|s| s.contains(&node.id)) {
                continue;
            }
            let needed = self.count_missing_inputs(node, tokens)?;

            if needed == 0 {
                // Node is immediately ready
                self.mark_ready(node.id, priorities, op_states, queue);
                ready_nodes.push(node.id);
            } else {
                // Track pending inputs
                self.pending_inputs.insert(node.id, needed);
            }
        }

        Ok(ready_nodes)
    }

    /// Count how many inputs this node is waiting for.
    ///
    /// Returns the number of input tokens that are not yet ready.
    fn count_missing_inputs(
        &self,
        node: &Node,
        tokens: &DashMap<TokenId, TokenState>,
    ) -> RuntimeResult<usize> {
        let mut needed = 0;

        for &token_id in &node.input_tokens {
            let Some(state) = tokens.get(&token_id) else {
                return Err(RuntimeError::SchedulerMissingToken {
                    node_id: node.id,
                    token_id,
                });
            };

            if !state.ready {
                needed += 1;
            }
        }

        Ok(needed)
    }

    /// Mark a node as ready and enqueue it at the appropriate priority.
    fn mark_ready(
        &self,
        node_id: NodeId,
        priorities: &DashMap<NodeId, Priority>,
        op_states: &DashMap<NodeId, OpState>,
        queue: &PriorityQueue,
    ) {
        // Update operation status
        if let Some(mut state) = op_states.get_mut(&node_id) {
            state.status = OpStatus::Ready;
            state.ready_at = Some(std::time::Instant::now());
        }

        // Enqueue at appropriate priority level
        let priority = priorities
            .get(&node_id)
            .map(|entry| *entry.value())
            .unwrap_or(Priority::Normal);
        queue.push(node_id, priority);
    }

    /// Handle a token becoming ready.
    ///
    /// Decrements pending counts for all consumers. If any consumer reaches 0,
    /// it's marked ready and enqueued.
    ///
    /// Returns the list of newly ready node IDs.
    pub(crate) fn on_token_ready(
        &self,
        token_id: TokenId,
        tokens: &DashMap<TokenId, TokenState>,
        priorities: &DashMap<NodeId, Priority>,
        op_states: &DashMap<NodeId, OpState>,
        queue: &PriorityQueue,
    ) -> RuntimeResult<Vec<NodeId>> {
        let Some(token_state) = tokens.get(&token_id) else {
            // Token doesn't exist - this shouldn't happen but handle gracefully
            return Ok(Vec::new());
        };

        let mut newly_ready = Vec::new();

        // Process each consumer
        for &consumer_id in &token_state.consumers {
            // Decrement pending count
            if let Some(mut entry) = self.pending_inputs.get_mut(&consumer_id) {
                *entry.value_mut() = entry.value().saturating_sub(1);
                let new_count = *entry.value();

                if new_count == 0 {
                    // Consumer is now ready
                    drop(entry); // Release lock before marking ready
                    self.pending_inputs.remove(&consumer_id);
                    self.mark_ready(consumer_id, priorities, op_states, queue);
                    newly_ready.push(consumer_id);
                }
            }
        }

        Ok(newly_ready)
    }

    /// Get a snapshot of all pending counts for diagnostics.
    pub(crate) fn snapshot(&self) -> Vec<(NodeId, usize)> {
        let mut snapshot: Vec<_> = self
            .pending_inputs
            .iter()
            .map(|entry| (*entry.key(), *entry.value()))
            .collect();
        snapshot.sort_unstable_by_key(|(node_id, _)| *node_id);
        snapshot
    }
}

impl Default for ReadySet {
    fn default() -> Self {
        Self::new()
    }
}
