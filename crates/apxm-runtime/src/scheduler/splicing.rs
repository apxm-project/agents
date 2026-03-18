//! Dynamic DAG splicing for inner/outer plan unification
//!
//! This module implements the core capability for merging inner plan DAGs
//! into the live execution of outer plan DAGs, as described in the A-PXM paper.
//!
//! Key concepts:
//! - The outer plan is already executing
//! - An inner plan DAG is generated during execution (e.g., from PLAN/RSN)
//! - The inner DAG is spliced into the live execution
//! - The result is ONE unified DAG with merged dependencies

use apxm_core::{
    error::RuntimeError,
    log_debug, log_info, log_trace,
    types::{Node, NodeId, TokenId, Value, execution::ExecutionDag},
};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::internal_state::{OpState, TokenState};
use super::queue::Priority;
use super::state::SchedulerState;
use crate::aam::effects::operation_effects;
use crate::executor::dag_splicer::{DagSplicer, SpliceResult};

/// Configuration for splicing an inner DAG into a running execution
pub struct SpliceConfig {
    /// The inner DAG to splice in
    pub inner_dag: ExecutionDag,

    /// Mapping from inner DAG entry node inputs to outer DAG output tokens
    ///
    /// This connects the outer plan to the inner plan.
    /// Key: Inner DAG token ID that needs a value
    /// Value: Outer DAG token ID that provides the value
    pub token_connections: HashMap<TokenId, TokenId>,

    /// Offset to add to all inner DAG node IDs to avoid conflicts
    ///
    /// If None, will be auto-calculated as max(outer_dag_node_ids) + 1
    pub node_id_offset: Option<u64>,

    /// Offset to add to all inner DAG token IDs to avoid conflicts
    ///
    /// If None, will be auto-calculated as max(outer_dag_token_ids) + 1
    pub token_id_offset: Option<u64>,
}

impl SchedulerState {
    /// Splice an inner DAG into the live execution
    ///
    /// This is the core operation for inner/outer plan unification.
    /// The inner DAG nodes are added to the scheduler state and
    /// integrated with the currently executing outer DAG.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration specifying the inner DAG and how to connect it
    ///
    /// # Returns
    ///
    /// Mapping from original inner DAG token IDs to remapped token IDs
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Token connections reference non-existent tokens
    /// - Node/token ID conflicts occur
    /// - Inner DAG is malformed
    pub fn splice_dag(
        &self,
        config: SpliceConfig,
    ) -> Result<HashMap<TokenId, TokenId>, RuntimeError> {
        log_info!(
            "scheduler::splice",
            inner_nodes = config.inner_dag.nodes.len(),
            inner_edges = config.inner_dag.edges.len(),
            connections = config.token_connections.len(),
            "Splicing inner DAG into live execution"
        );

        // Calculate offsets to avoid ID conflicts
        let node_offset = config.node_id_offset.unwrap_or_else(|| {
            self.nodes
                .iter()
                .map(|entry| *entry.key())
                .max()
                .unwrap_or(0)
                + 1
        });

        let token_offset = config.token_id_offset.unwrap_or_else(|| {
            self.tokens
                .iter()
                .map(|entry| *entry.key())
                .max()
                .unwrap_or(0)
                + 1
        });

        log_debug!(
            "scheduler::splice",
            node_offset = node_offset,
            token_offset = token_offset,
            "Calculated ID offsets for splicing"
        );

        // Build token remapping
        let mut token_remap = HashMap::new();
        for (inner_token_id, outer_token_id) in &config.token_connections {
            token_remap.insert(*inner_token_id, *outer_token_id);
        }

        let mut remap_token = |token_id: TokenId| -> TokenId {
            *token_remap
                .entry(token_id)
                .or_insert_with(|| token_id + token_offset)
        };

        // Remap all inner DAG token IDs
        for node in &config.inner_dag.nodes {
            for &token_id in node.output_tokens.iter() {
                remap_token(token_id);
            }
            for &token_id in node.input_tokens.iter() {
                remap_token(token_id);
            }
        }

        // Remap nodes and add to scheduler state
        let mut ready_nodes = Vec::new();

        for node in &config.inner_dag.nodes {
            let original_inputs = node.input_tokens.clone();
            let original_outputs = node.output_tokens.clone();

            let mut node = node.clone();
            // Remap node ID
            let old_node_id = node.id;
            node.id += node_offset;

            // Remap input tokens
            node.input_tokens = node
                .input_tokens
                .iter()
                .map(|&tid| *token_remap.get(&tid).unwrap_or(&tid))
                .collect();

            // Remap output tokens
            node.output_tokens = node
                .output_tokens
                .iter()
                .map(|&tid| *token_remap.get(&tid).unwrap_or(&tid))
                .collect();

            // Add node to state
            let node_arc = Arc::new(node.clone());
            self.nodes.insert(node.id, Arc::clone(&node_arc));

            // Initialize operation state
            self.op_states.insert(
                node.id,
                OpState::new_with_effects(operation_effects(&node.op_type)),
            );

            // Set priority
            let priority = Priority::from_u8(node.metadata.priority as u8);
            self.priorities.insert(node.id, priority);

            // Initialize output tokens
            for (&original, &token_id) in original_outputs.iter().zip(node.output_tokens.iter()) {
                if config.token_connections.contains_key(&original) {
                    continue;
                }
                if !self.tokens.contains_key(&token_id) {
                    self.tokens.insert(token_id, TokenState::new());
                }
            }

            // Register as consumer for input tokens and check readiness
            let mut all_inputs_ready = true;
            let mut missing_count: usize = 0;
            for (original_token, &token_id) in original_inputs.iter().zip(node.input_tokens.iter())
            {
                if config.token_connections.contains_key(original_token) {
                    let Some(mut token_state) = self.tokens.get_mut(&token_id) else {
                        return Err(RuntimeError::Scheduler {
                            message: format!(
                                "Token connection references non-existent token: {}",
                                token_id
                            ),
                        });
                    };
                    token_state.consumers.push(node.id);
                    if !token_state.ready {
                        all_inputs_ready = false;
                        missing_count = missing_count.saturating_add(1);
                    }
                } else {
                    let mut entry = self.tokens.entry(token_id).or_insert_with(|| {
                        let mut ts = TokenState::new();
                        ts.ready = true;
                        ts.value = Some(Value::Null);
                        ts
                    });
                    entry.consumers.push(node.id);
                    if !entry.ready {
                        all_inputs_ready = false;
                        missing_count = missing_count.saturating_add(1);
                    }
                }
            }

            // If all inputs are ready, mark for scheduling; otherwise register pending count so
            // the ReadySet can decrement and enqueue this node when inputs arrive.
            if all_inputs_ready {
                ready_nodes.push((node.id, priority));
            } else {
                // Use ReadySet helper to register the number of missing inputs for this spliced node.
                // This ensures on_token_ready will find and decrement the pending count.
                self.ready_set.insert_pending(node.id, missing_count);
            }

            log_trace!(
                "scheduler::splice",
                old_node_id = old_node_id,
                new_node_id = node.id,
                op_type = ?node.op_type,
                ready = all_inputs_ready,
                "Spliced node into DAG"
            );
        }

        // Increment remaining counter for new nodes
        let new_node_count = config.inner_dag.nodes.len();
        let old_remaining = self.remaining.load(std::sync::atomic::Ordering::SeqCst);
        self.remaining
            .fetch_add(new_node_count, std::sync::atomic::Ordering::SeqCst);
        let new_remaining = self.remaining.load(std::sync::atomic::Ordering::SeqCst);
        tracing::info!(
            old_remaining = old_remaining,
            added = new_node_count,
            new_remaining = new_remaining,
            "Incremented remaining for spliced nodes"
        );

        // Enqueue ready nodes
        for (node_id, priority) in &ready_nodes {
            self.queue.push(*node_id, *priority);
            tracing::debug!(node_id = node_id, "Enqueued ready inner DAG node");
        }

        // Record progress to prevent deadlock detection
        self.record_progress();

        tracing::info!(
            spliced_nodes = new_node_count,
            ready_count = ready_nodes.len(),
            "Inner DAG spliced successfully"
        );

        Ok(token_remap)
    }

    /// Condense a set of nodes in the live DAG into a single replacement node.
    ///
    /// This is the reverse of `splice_dag`. It removes the specified nodes and
    /// reconnects external edges through the replacement node.
    ///
    /// External inputs (tokens consumed by the sub-DAG but produced outside it)
    /// become inputs to the replacement node.
    /// External outputs (tokens produced by the sub-DAG but consumed outside it)
    /// become outputs of the replacement node.
    pub fn condense_subdag(
        &self,
        node_ids: &[NodeId],
        replacement: Arc<Node>,
    ) -> Result<(), RuntimeError> {
        if node_ids.is_empty() {
            return Err(RuntimeError::Scheduler {
                message: "condense_subdag: node_ids must not be empty".to_string(),
            });
        }

        let subgraph: HashSet<NodeId> = node_ids.iter().copied().collect();

        // 1. Validate all node_ids exist
        for &nid in &subgraph {
            if !self.nodes.contains_key(&nid) {
                return Err(RuntimeError::Scheduler {
                    message: format!("condense_subdag: node {} does not exist in the DAG", nid),
                });
            }
        }

        log_info!(
            "scheduler::condense",
            subgraph_size = subgraph.len(),
            replacement_id = replacement.id,
            "Condensing sub-DAG into single node"
        );

        // 2. Collect all tokens produced and consumed by the subgraph
        let mut subgraph_output_tokens: HashSet<TokenId> = HashSet::new();
        let mut subgraph_input_tokens: HashSet<TokenId> = HashSet::new();

        for &nid in &subgraph {
            if let Some(node) = self.nodes.get(&nid) {
                for &tid in &node.output_tokens {
                    subgraph_output_tokens.insert(tid);
                }
                for &tid in &node.input_tokens {
                    subgraph_input_tokens.insert(tid);
                }
            }
        }

        // External inputs: tokens consumed by subgraph but produced outside it
        let external_inputs: Vec<TokenId> = subgraph_input_tokens
            .iter()
            .filter(|tid| !subgraph_output_tokens.contains(tid))
            .copied()
            .collect();

        // External outputs: tokens produced by subgraph and consumed by nodes outside it
        let mut external_outputs: Vec<TokenId> = Vec::new();
        for &tid in &subgraph_output_tokens {
            if let Some(token_state) = self.tokens.get(&tid) {
                let has_external_consumer = token_state
                    .consumers
                    .iter()
                    .any(|cid| !subgraph.contains(cid));
                if has_external_consumer {
                    external_outputs.push(tid);
                }
            }
        }

        // Internal-only tokens: produced and consumed entirely within the subgraph
        let internal_tokens: Vec<TokenId> = subgraph_output_tokens
            .iter()
            .filter(|tid| !external_outputs.contains(tid))
            .copied()
            .collect();

        log_debug!(
            "scheduler::condense",
            external_inputs = external_inputs.len(),
            external_outputs = external_outputs.len(),
            internal_tokens = internal_tokens.len(),
            "Identified subgraph boundary"
        );

        // 3. Remove subgraph nodes and their op/priority state
        for &nid in &subgraph {
            self.nodes.remove(&nid);
            self.op_states.remove(&nid);
            self.priorities.remove(&nid);
            // Also remove from ready_set pending tracking
            self.ready_set.remove_pending(nid);
        }

        // 4. Clean up internal-only tokens
        for &tid in &internal_tokens {
            self.tokens.remove(&tid);
        }

        // 5. For external output tokens, remove subgraph nodes from consumers
        //    and remove the subgraph node as producer (the replacement takes over)
        for &tid in &external_outputs {
            if let Some(mut token_state) = self.tokens.get_mut(&tid) {
                token_state.consumers.retain(|cid| !subgraph.contains(cid));
            }
        }

        // 6. For external input tokens, replace subgraph consumer references
        //    with the replacement node
        for &tid in &external_inputs {
            if let Some(mut token_state) = self.tokens.get_mut(&tid) {
                // Remove all subgraph consumers
                token_state.consumers.retain(|cid| !subgraph.contains(cid));
                // Add replacement as consumer
                token_state.consumers.push(replacement.id);
            }
        }

        // 7. Insert the replacement node
        self.nodes.insert(replacement.id, Arc::clone(&replacement));
        self.op_states.insert(
            replacement.id,
            OpState::new_with_effects(operation_effects(&replacement.op_type)),
        );
        let priority = Priority::from_u8(replacement.metadata.priority as u8);
        self.priorities.insert(replacement.id, priority);

        // 8. Ensure output tokens for the replacement exist
        for &tid in &replacement.output_tokens {
            if !self.tokens.contains_key(&tid) {
                self.tokens.insert(tid, TokenState::new());
            }
        }

        // 9. Adjust remaining counter: removed N nodes, added 1
        let removed = subgraph.len();
        if removed > 1 {
            self.remaining
                .fetch_sub(removed - 1, std::sync::atomic::Ordering::SeqCst);
        }

        // 10. Check readiness of the replacement node and enqueue if ready
        let mut all_inputs_ready = true;
        let mut missing_count: usize = 0;
        for &tid in &replacement.input_tokens {
            if let Some(token_state) = self.tokens.get(&tid) {
                if !token_state.ready {
                    all_inputs_ready = false;
                    missing_count = missing_count.saturating_add(1);
                }
            } else {
                // Token doesn't exist yet — not ready
                all_inputs_ready = false;
                missing_count = missing_count.saturating_add(1);
            }
        }

        if all_inputs_ready {
            self.queue.push(replacement.id, priority);
            log_debug!(
                "scheduler::condense",
                node_id = replacement.id,
                "Replacement node is ready, enqueued"
            );
        } else {
            self.ready_set
                .insert_pending(replacement.id, missing_count);
        }

        self.record_progress();

        log_info!(
            "scheduler::condense",
            removed_nodes = removed,
            replacement_id = replacement.id,
            "Sub-DAG condensed successfully"
        );

        Ok(())
    }
}

/// Concrete implementation of the DagSplicer trait backed by a SchedulerState.
pub struct SchedulerDagSplicer {
    state: Arc<SchedulerState>,
}

impl SchedulerDagSplicer {
    pub fn new(state: Arc<SchedulerState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl DagSplicer for SchedulerDagSplicer {
    async fn splice_dag(
        &self,
        inner_dag: ExecutionDag,
        token_connections: HashMap<TokenId, TokenId>,
    ) -> SpliceResult {
        let config = SpliceConfig {
            inner_dag,
            token_connections,
            node_id_offset: None,
            token_id_offset: None,
        };

        self.state.splice_dag(config)
    }

    fn mark_tokens_delegated(&self, delegator_node_id: u64, token_ids: &[TokenId]) {
        for &token_id in token_ids {
            // Insert into sparse delegation set - O(1) and zero overhead for non-switch DAGs
            self.state
                .delegated_tokens
                .insert((delegator_node_id, token_id));
            log_debug!(
                "scheduler::splice",
                token_id = token_id,
                delegator = delegator_node_id,
                "Marked token as delegated"
            );
        }
    }

    async fn condense_subdag(
        &self,
        node_ids: &[NodeId],
        replacement: Arc<Node>,
    ) -> Result<(), RuntimeError> {
        self.state.condense_subdag(node_ids, replacement)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::MetricsCollector;
    use crate::scheduler::config::SchedulerConfig;
    use crate::scheduler::state::SchedulerState;
    use apxm_core::types::{Node, Value, execution::DagMetadata, operations::AISOperationType};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Instant;

    #[test]
    fn test_splice_config_creation() {
        let inner_dag = ExecutionDag {
            nodes: vec![],
            edges: vec![],
            entry_nodes: vec![],
            exit_nodes: vec![],
            metadata: DagMetadata::default(),
        };

        let config = SpliceConfig {
            inner_dag,
            token_connections: HashMap::new(),
            node_id_offset: Some(100),
            token_id_offset: Some(200),
        };

        assert_eq!(config.node_id_offset, Some(100));
        assert_eq!(config.token_id_offset, Some(200));
    }

    #[test]
    fn test_splice_dag_connects_outer_token() {
        let mut outer_dag = ExecutionDag::new();

        let mut producer = Node::new(1, AISOperationType::ConstStr);
        producer.output_tokens = vec![1];
        producer
            .attributes
            .insert("value".into(), Value::String("seed".into()));

        let mut consumer = Node::new(2, AISOperationType::Return);
        consumer.input_tokens = vec![1];

        outer_dag.nodes.push(producer);
        outer_dag.nodes.push(consumer);
        outer_dag.entry_nodes = vec![1];
        outer_dag.exit_nodes = vec![2];

        let cfg = SchedulerConfig::default();
        let metrics = Arc::new(MetricsCollector::default());
        let (state, _) =
            SchedulerState::new(outer_dag, cfg, metrics, Instant::now(), vec![]).unwrap();
        let state = Arc::new(state);

        let mut inner_dag = ExecutionDag::new();
        let mut inner = Node::new(5, AISOperationType::Ask);
        inner.input_tokens = vec![10];
        inner.output_tokens = vec![11];
        inner_dag.nodes.push(inner);
        inner_dag.entry_nodes = vec![5];
        inner_dag.exit_nodes = vec![5];

        let config = SpliceConfig {
            inner_dag,
            token_connections: HashMap::from([(10u64, 1u64)]),
            node_id_offset: None,
            token_id_offset: None,
        };

        let remap = state.splice_dag(config).expect("splice succeeds");
        assert_eq!(remap.get(&10), Some(&1));

        let inserted_id = state
            .nodes
            .iter()
            .filter_map(|entry| {
                let id = *entry.key();
                let node = entry.value();
                (id > 2 && matches!(node.op_type, AISOperationType::Ask)).then_some(id)
            })
            .next()
            .expect("inserted node present");

        let token_state = state.tokens.get(&1).expect("outer token exists");
        assert!(token_state.consumers.contains(&inserted_id));
    }

    /// Helper: build a SchedulerState from an ExecutionDag.
    fn build_state(dag: ExecutionDag) -> Arc<SchedulerState> {
        let cfg = SchedulerConfig::default();
        let metrics = Arc::new(MetricsCollector::default());
        let (state, _) =
            SchedulerState::new(dag, cfg, metrics, Instant::now(), vec![]).unwrap();
        Arc::new(state)
    }

    // ---- condense_subdag tests ----

    #[test]
    fn test_condense_subdag_empty_ids_errors() {
        // Build a trivial DAG so SchedulerState is valid
        let mut dag = ExecutionDag::new();
        let n = Node::new(1, AISOperationType::ConstStr);
        dag.nodes.push(n);
        dag.entry_nodes = vec![1];
        dag.exit_nodes = vec![1];

        let state = build_state(dag);

        let replacement = Arc::new(Node::new(99, AISOperationType::Return));
        let result = state.condense_subdag(&[], replacement);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("must not be empty"));
    }

    #[test]
    fn test_condense_subdag_missing_node_errors() {
        let mut dag = ExecutionDag::new();
        let n = Node::new(1, AISOperationType::ConstStr);
        dag.nodes.push(n);
        dag.entry_nodes = vec![1];
        dag.exit_nodes = vec![1];

        let state = build_state(dag);

        let replacement = Arc::new(Node::new(99, AISOperationType::Return));
        // Node 42 doesn't exist
        let result = state.condense_subdag(&[42], replacement);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("does not exist"));
    }

    #[test]
    fn test_condense_subdag_linear_chain() {
        // DAG: A(1) --tok1--> B(2) --tok2--> C(3) --tok3--> D(4)
        // Condense B,C into R(99)
        // After: A(1) --tok1--> R(99) --tok3--> D(4)

        let mut dag = ExecutionDag::new();

        let mut a = Node::new(1, AISOperationType::ConstStr);
        a.output_tokens = vec![10];
        a.attributes
            .insert("value".into(), Value::String("v".into()));

        let mut b = Node::new(2, AISOperationType::Ask);
        b.input_tokens = vec![10];
        b.output_tokens = vec![20];

        let mut c = Node::new(3, AISOperationType::Ask);
        c.input_tokens = vec![20];
        c.output_tokens = vec![30];

        let mut d = Node::new(4, AISOperationType::Return);
        d.input_tokens = vec![30];

        dag.nodes = vec![a, b, c, d];
        dag.entry_nodes = vec![1];
        dag.exit_nodes = vec![4];

        let state = build_state(dag);

        // Verify initial state
        assert!(state.nodes.contains_key(&2));
        assert!(state.nodes.contains_key(&3));
        let initial_remaining = state.remaining.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(initial_remaining, 4);

        // Build replacement: R consumes tok10 (external input), produces tok30 (external output)
        let mut r = Node::new(99, AISOperationType::Ask);
        r.input_tokens = vec![10];
        r.output_tokens = vec![30];
        let replacement = Arc::new(r);

        state
            .condense_subdag(&[2, 3], replacement.clone())
            .expect("condense succeeds");

        // Verify nodes B and C are removed
        assert!(!state.nodes.contains_key(&2));
        assert!(!state.nodes.contains_key(&3));

        // Verify replacement is inserted
        assert!(state.nodes.contains_key(&99));
        let inserted = state.nodes.get(&99).unwrap();
        assert_eq!(inserted.input_tokens, vec![10]);
        assert_eq!(inserted.output_tokens, vec![30]);

        // Verify tok10 now has R(99) as consumer (not B(2))
        let tok10 = state.tokens.get(&10).expect("tok10 exists");
        assert!(tok10.consumers.contains(&99));
        assert!(!tok10.consumers.contains(&2));

        // Verify tok30 no longer has C(3) in consumers but keeps D(4)
        let tok30 = state.tokens.get(&30).expect("tok30 exists");
        assert!(!tok30.consumers.contains(&3));
        assert!(tok30.consumers.contains(&4));

        // Verify internal token tok20 is removed
        assert!(!state.tokens.contains_key(&20));

        // Verify remaining counter: was 4, removed 2, added 1 => 3
        let new_remaining = state.remaining.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(new_remaining, 3);

        // Verify op_states for removed nodes are gone
        assert!(!state.op_states.contains_key(&2));
        assert!(!state.op_states.contains_key(&3));

        // Verify op_states for replacement exists
        assert!(state.op_states.contains_key(&99));
    }

    #[test]
    fn test_condense_subdag_single_node() {
        // Condensing a single node into a replacement (essentially a node swap)
        // DAG: A(1) --tok10--> B(2) --tok20--> C(3)
        // Condense B(2) into R(99)

        let mut dag = ExecutionDag::new();

        let mut a = Node::new(1, AISOperationType::ConstStr);
        a.output_tokens = vec![10];
        a.attributes
            .insert("value".into(), Value::String("v".into()));

        let mut b = Node::new(2, AISOperationType::Ask);
        b.input_tokens = vec![10];
        b.output_tokens = vec![20];

        let mut c = Node::new(3, AISOperationType::Return);
        c.input_tokens = vec![20];

        dag.nodes = vec![a, b, c];
        dag.entry_nodes = vec![1];
        dag.exit_nodes = vec![3];

        let state = build_state(dag);

        let mut r = Node::new(99, AISOperationType::Ask);
        r.input_tokens = vec![10];
        r.output_tokens = vec![20];
        let replacement = Arc::new(r);

        state
            .condense_subdag(&[2], replacement)
            .expect("condense single node");

        // remaining should stay the same (removed 1, added 1)
        assert_eq!(
            state.remaining.load(std::sync::atomic::Ordering::SeqCst),
            3
        );

        assert!(!state.nodes.contains_key(&2));
        assert!(state.nodes.contains_key(&99));

        // tok10 consumer should be 99, not 2
        let tok10 = state.tokens.get(&10).unwrap();
        assert!(tok10.consumers.contains(&99));
        assert!(!tok10.consumers.contains(&2));
    }

    #[test]
    fn test_condense_subdag_preserves_other_nodes() {
        // Ensure nodes outside the subgraph are untouched
        // DAG: A(1) --> B(2) --> C(3) --> D(4)
        //              ^--- condense B,C ---^
        // Also E(5) is independent

        let mut dag = ExecutionDag::new();

        let mut a = Node::new(1, AISOperationType::ConstStr);
        a.output_tokens = vec![10];
        a.attributes
            .insert("value".into(), Value::String("v".into()));

        let mut b = Node::new(2, AISOperationType::Ask);
        b.input_tokens = vec![10];
        b.output_tokens = vec![20];

        let mut c = Node::new(3, AISOperationType::Ask);
        c.input_tokens = vec![20];
        c.output_tokens = vec![30];

        let mut d = Node::new(4, AISOperationType::Return);
        d.input_tokens = vec![30];

        let mut e = Node::new(5, AISOperationType::ConstStr);
        e.output_tokens = vec![50];
        e.attributes
            .insert("value".into(), Value::String("independent".into()));

        dag.nodes = vec![a, b, c, d, e];
        dag.entry_nodes = vec![1, 5];
        dag.exit_nodes = vec![4];

        let state = build_state(dag);

        let mut r = Node::new(99, AISOperationType::Ask);
        r.input_tokens = vec![10];
        r.output_tokens = vec![30];
        let replacement = Arc::new(r);

        state.condense_subdag(&[2, 3], replacement).unwrap();

        // A, D, E should all still exist
        assert!(state.nodes.contains_key(&1));
        assert!(state.nodes.contains_key(&4));
        assert!(state.nodes.contains_key(&5));

        // Token 50 should be untouched
        assert!(state.tokens.contains_key(&50));
    }

    #[tokio::test]
    async fn test_scheduler_dag_splicer_condense() {
        // Integration test through the SchedulerDagSplicer trait impl
        let mut dag = ExecutionDag::new();

        let mut a = Node::new(1, AISOperationType::ConstStr);
        a.output_tokens = vec![10];
        a.attributes
            .insert("value".into(), Value::String("v".into()));

        let mut b = Node::new(2, AISOperationType::Ask);
        b.input_tokens = vec![10];
        b.output_tokens = vec![20];

        dag.nodes = vec![a, b];
        dag.entry_nodes = vec![1];
        dag.exit_nodes = vec![2];

        let state = build_state(dag);
        let splicer = SchedulerDagSplicer::new(Arc::clone(&state));

        let mut r = Node::new(99, AISOperationType::Return);
        r.input_tokens = vec![10];
        r.output_tokens = vec![20];

        let result = splicer.condense_subdag(&[2], Arc::new(r)).await;
        assert!(result.is_ok());
        assert!(state.nodes.contains_key(&99));
        assert!(!state.nodes.contains_key(&2));
    }
}
