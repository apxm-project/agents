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
use dashmap::DashMap;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, Weak};

use super::internal_state::{OpState, TokenState};
use super::queue::Priority;
use super::state::SchedulerState;
use crate::aam::effects::operation_effects;
use crate::executor::dag_splicer::{DagSplicer, SpliceResult};

type NodeMap = DashMap<NodeId, Arc<Node>>;
type TokenMap = DashMap<TokenId, TokenState>;

#[derive(Default)]
struct ReservedSpliceIds {
    nodes: HashSet<NodeId>,
    tokens: HashSet<TokenId>,
}

struct SpliceIdReservations {
    nodes: Weak<NodeMap>,
    tokens: Weak<TokenMap>,
    reserved: Mutex<ReservedSpliceIds>,
}

impl SpliceIdReservations {
    fn matches(&self, nodes: &Arc<NodeMap>, tokens: &Arc<TokenMap>) -> bool {
        self.nodes
            .upgrade()
            .is_some_and(|current| Arc::ptr_eq(&current, nodes))
            && self
                .tokens
                .upgrade()
                .is_some_and(|current| Arc::ptr_eq(&current, tokens))
    }
}

struct SpliceIdReservation {
    owner: Arc<SpliceIdReservations>,
    node_ids: Vec<NodeId>,
    token_ids: Vec<TokenId>,
}

impl Drop for SpliceIdReservation {
    fn drop(&mut self) {
        let mut reserved = self.owner.reserved.lock();
        for node_id in &self.node_ids {
            reserved.nodes.remove(node_id);
        }
        for token_id in &self.token_ids {
            reserved.tokens.remove(token_id);
        }
    }
}

fn splice_id_registry() -> &'static Mutex<HashMap<usize, Weak<SpliceIdReservations>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<usize, Weak<SpliceIdReservations>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn checked_next_id<I>(ids: I, kind: &str) -> Result<u64, RuntimeError>
where
    I: Iterator<Item = u64>,
{
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| RuntimeError::Scheduler {
            message: format!("Cannot reserve {kind} IDs after u64::MAX"),
        })
}

fn remapped_ids<I>(ids: I, offset: u64, kind: &str) -> Result<Vec<u64>, RuntimeError>
where
    I: Iterator<Item = u64>,
{
    ids.map(|id| {
        id.checked_add(offset)
            .ok_or_else(|| RuntimeError::Scheduler {
                message: format!("{kind} ID {id} overflows with splice offset {offset}"),
            })
    })
    .collect()
}

#[cfg(test)]
thread_local! {
    static SPLICE_RESERVATION_BARRIER: std::cell::RefCell<Option<Arc<std::sync::Barrier>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_splice_reservation_barrier<T>(
    barrier: Arc<std::sync::Barrier>,
    splice: impl FnOnce() -> T,
) -> T {
    SPLICE_RESERVATION_BARRIER.with(|slot| {
        let previous = slot.replace(Some(barrier));
        let result = splice();
        slot.replace(previous);
        result
    })
}

#[cfg(test)]
fn wait_at_splice_reservation_barrier() {
    SPLICE_RESERVATION_BARRIER.with(|slot| {
        if let Some(barrier) = slot.borrow().as_ref() {
            barrier.wait();
        }
    });
}

#[cfg(not(test))]
fn wait_at_splice_reservation_barrier() {}

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
    fn splice_id_reservations(&self) -> Arc<SpliceIdReservations> {
        let key = Arc::as_ptr(&self.nodes) as usize;
        let mut registry = splice_id_registry().lock();
        registry.retain(|_, entry| entry.strong_count() > 0);

        if let Some(existing) = registry.get(&key).and_then(Weak::upgrade)
            && existing.matches(&self.nodes, &self.tokens)
        {
            return existing;
        }

        let reservations = Arc::new(SpliceIdReservations {
            nodes: Arc::downgrade(&self.nodes),
            tokens: Arc::downgrade(&self.tokens),
            reserved: Mutex::new(ReservedSpliceIds::default()),
        });
        registry.insert(key, Arc::downgrade(&reservations));
        reservations
    }

    fn reserve_splice_ids(
        &self,
        config: &SpliceConfig,
        observed_node_next: NodeId,
        observed_token_next: TokenId,
    ) -> Result<(NodeId, TokenId, SpliceIdReservation), RuntimeError> {
        let reservations = self.splice_id_reservations();
        let mut reserved = reservations.reserved.lock();

        let node_next = checked_next_id(
            self.nodes
                .iter()
                .map(|entry| *entry.key())
                .chain(reserved.nodes.iter().copied()),
            "node",
        )?
        .max(observed_node_next);
        let token_next = checked_next_id(
            self.tokens
                .iter()
                .map(|entry| *entry.key())
                .chain(reserved.tokens.iter().copied()),
            "token",
        )?
        .max(observed_token_next);

        let node_offset = config.node_id_offset.unwrap_or(node_next);
        let token_offset = config.token_id_offset.unwrap_or(token_next);

        let node_ids = remapped_ids(
            config.inner_dag.nodes.iter().map(|node| node.id),
            node_offset,
            "Node",
        )?;
        let connected_tokens: HashSet<TokenId> = config.token_connections.keys().copied().collect();
        let inner_tokens: HashSet<TokenId> = config
            .inner_dag
            .nodes
            .iter()
            .flat_map(|node| node.input_tokens.iter().chain(&node.output_tokens).copied())
            .filter(|token_id| !connected_tokens.contains(token_id))
            .collect();
        let token_ids = remapped_ids(inner_tokens.into_iter(), token_offset, "Token")?;

        let unique_nodes: HashSet<NodeId> = node_ids.iter().copied().collect();
        if unique_nodes.len() != node_ids.len() {
            return Err(RuntimeError::Scheduler {
                message: "Inner DAG contains duplicate node IDs".to_string(),
            });
        }

        if let Some(node_id) = node_ids
            .iter()
            .find(|node_id| self.nodes.contains_key(node_id) || reserved.nodes.contains(node_id))
        {
            return Err(RuntimeError::Scheduler {
                message: format!("Spliced node ID conflicts with live scheduler state: {node_id}"),
            });
        }
        if let Some(token_id) = token_ids.iter().find(|token_id| {
            self.tokens.contains_key(token_id) || reserved.tokens.contains(token_id)
        }) {
            return Err(RuntimeError::Scheduler {
                message: format!(
                    "Spliced token ID conflicts with live scheduler state: {token_id}"
                ),
            });
        }

        reserved.nodes.extend(node_ids.iter().copied());
        reserved.tokens.extend(token_ids.iter().copied());
        drop(reserved);

        Ok((
            node_offset,
            token_offset,
            SpliceIdReservation {
                owner: reservations,
                node_ids,
                token_ids,
            },
        ))
    }

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

        // Observe the current high-water marks without blocking scheduler work.
        // The reservation below rechecks them while atomically claiming both ID
        // sets, so concurrent splices that observe the same maxima cannot overlap.
        let observed_node_next =
            checked_next_id(self.nodes.iter().map(|entry| *entry.key()), "node")?;
        let observed_token_next =
            checked_next_id(self.tokens.iter().map(|entry| *entry.key()), "token")?;
        wait_at_splice_reservation_barrier();
        let (node_offset, token_offset, _id_reservation) =
            self.reserve_splice_ids(&config, observed_node_next, observed_token_next)?;

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
            if let Some(mut op_state) = self.op_states.get_mut(node_id) {
                op_state.status = apxm_core::types::OpStatus::Ready;
                op_state.ready_at = Some(std::time::Instant::now());
            }
            self.queue.push(*node_id, *priority);
            self.emit_node_ready(*node_id);
            tracing::debug!(node_id = node_id, "Enqueued ready inner DAG node");
        }
        if !ready_nodes.is_empty() {
            self.work_notify.notify_waiters();
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

    /// Re-arm the in-graph conversation loop after a `recv` node woke with a
    /// user message.
    ///
    /// Splices a fresh turn sub-DAG whose `turn_input_token` is connected to the
    /// woken `message_token`, plus a `fresh_recv` node (no inputs → becomes ready
    /// and re-parks for the next message), into the live execution in ONE splice.
    /// Consequences:
    /// - each user turn runs its OWN spliced sub-DAG; prior turns are never
    ///   re-executed;
    /// - the loop continues because the fresh recv re-arms;
    /// - it is pure composition over [`Self::splice_dag`], so the fire-once and
    ///   `remaining`-count invariants (the park/wake suite) are preserved — no new
    ///   scheduler primitive is required.
    ///
    /// `fresh_recv` MUST carry a node id unique within `turn_dag`; both are
    /// offset away from live IDs by the splice.
    ///
    /// `carry_connections` carries per-turn session state across re-arms:
    /// each entry maps an inner turn-body input token → a live token holding
    /// prior state (e.g. the running history/summary token produced by the
    /// previous turn). This threads cross-turn memory through spliced token
    /// connections without re-running prior turns.
    pub fn splice_turn_and_rearm(
        &self,
        message_token: TokenId,
        turn_input_token: TokenId,
        carry_connections: HashMap<TokenId, TokenId>,
        mut turn_dag: ExecutionDag,
        fresh_recv: Node,
    ) -> Result<HashMap<TokenId, TokenId>, RuntimeError> {
        // Fold the fresh recv into the same inner DAG so the turn body and the
        // re-arm graft atomically in one splice.
        turn_dag.add_node(fresh_recv)?;
        let mut connections = carry_connections;
        connections.insert(turn_input_token, message_token);
        self.splice_dag(SpliceConfig {
            inner_dag: turn_dag,
            token_connections: connections,
            node_id_offset: None,
            token_id_offset: None,
        })
    }

    /// Re-arm one explicit continuation from a woken AWAIT_INPUT node.
    ///
    /// Builds, in one splice: (1) a FLOW_CALL to the declared continuation with
    /// the woken input bound to its declared parameter, and (2) a fresh cloned
    /// AWAIT_INPUT node. Prior continuations are never re-run.
    pub fn rearm_input_continuation(
        &self,
        input_token: TokenId,
        await_node: &Node,
        agent_name: &str,
        flow_name: &str,
        input_name: &str,
    ) -> Result<(), RuntimeError> {
        use apxm_core::constants::graph::attrs as ga;
        use apxm_core::types::execution::NodeMetadata;
        use apxm_core::types::operations::AISOperationType;

        // FLOW_CALL node: dispatch the explicit continuation, binding the input
        // token to its declared parameter through args + input_names.
        let mut fc_attrs = std::collections::HashMap::new();
        fc_attrs.insert(
            ga::AGENT_NAME.to_string(),
            Value::String(agent_name.to_string()),
        );
        fc_attrs.insert(
            ga::FLOW_NAME.to_string(),
            Value::String(flow_name.to_string()),
        );
        let mut args = std::collections::HashMap::new();
        args.insert(
            input_name.to_string(),
            Value::String(format!("{{{input_name}}}")),
        );
        fc_attrs.insert(ga::ARGS.to_string(), Value::Object(args));
        fc_attrs.insert(
            ga::INPUT_NAMES.to_string(),
            Value::Array(vec![Value::String(input_name.to_string())]),
        );
        let flow_call = Node {
            id: 1,
            op_type: AISOperationType::FlowCall,
            attributes: fc_attrs,
            input_tokens: vec![1],
            output_tokens: vec![2],
            metadata: NodeMetadata::default(),
        };
        let mut continuation_dag = ExecutionDag::new();
        continuation_dag.add_node(flow_call)?;

        // Fresh wait: same attrs as the woken AWAIT_INPUT (so it re-arms again
        // on the next wake), no inputs, and a fresh output token.
        let mut fresh_await = await_node.clone();
        fresh_await.id = 2;
        fresh_await.input_tokens = vec![];
        fresh_await.output_tokens = vec![3];

        self.splice_turn_and_rearm(
            input_token,
            1, // the FLOW_CALL inner input token connected to input_token
            std::collections::HashMap::new(),
            continuation_dag,
            fresh_await,
        )?;
        Ok(())
    }

    /// Condense a set of nodes in the live DAG into a single replacement node.
    ///
    /// This is the reverse of `splice_dag`. It removes the specified nodes and
    /// reconnects external edges through the replacement node.
    ///
    /// External inputs: tokens required by the sub-DAG but produced outside it.
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

        // 2. Collect all tokens produced and required by the subgraph.
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

        // External inputs: tokens required by subgraph but produced outside it.
        let external_inputs: Vec<TokenId> = subgraph_input_tokens
            .iter()
            .filter(|tid| !subgraph_output_tokens.contains(tid))
            .copied()
            .collect();

        // External outputs: tokens produced by subgraph for nodes outside it.
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

        // 9. Adjust remaining counter: deleted N nodes, added 1.
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
            if let Some(mut op_state) = self.op_states.get_mut(&replacement.id) {
                op_state.status = apxm_core::types::OpStatus::Ready;
                op_state.ready_at = Some(std::time::Instant::now());
            }
            self.queue.push(replacement.id, priority);
            self.emit_node_ready(replacement.id);
            self.work_notify.notify_waiters();
            log_debug!(
                "scheduler::condense",
                node_id = replacement.id,
                "Replacement node is ready, enqueued"
            );
        } else {
            self.ready_set.insert_pending(replacement.id, missing_count);
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
