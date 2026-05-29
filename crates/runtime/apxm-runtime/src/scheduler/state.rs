//! Scheduler state management.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use apxm_core::types::{
    ExecutionDag, ExecutionStats, Node, NodeId, NodeStatus, OpStatus, TokenId, Value,
};
use crossbeam_deque::Worker;
use dashmap::{DashMap, DashSet};
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::executor::hooks::{
    ExecutionHookContext, NodeFinishedEvent, NodeReadyEvent, NodeStartedEvent,
};
use crate::observability::MetricsCollector;
use crate::scheduler::concurrency_control::{ConcurrencyControl, ConcurrencyControlHandle};
use crate::scheduler::config::SchedulerConfig;
use apxm_core::error::RuntimeError;

type RuntimeResult<T> = Result<T, RuntimeError>;
use crate::aam::effects::{OperationEffects, operation_effects};
use crate::scheduler::internal_state::{ExecutionFrame, OpState, PromiseState, TokenState};
use crate::scheduler::queue::{Priority, PriorityQueue};
use crate::scheduler::ready_set::ReadySet;
use crate::scheduler::work_stealing::WorkStealingScheduler;

/// The internal state of the scheduler.
pub struct SchedulerState {
    pub cfg: SchedulerConfig,
    pub metrics: Arc<MetricsCollector>,
    pub start: Instant,
    pub hooks: ExecutionHookContext,

    // Immutable node data
    pub dag: Arc<ExecutionDag>,
    pub nodes: Arc<DashMap<NodeId, Arc<Node>>>,
    pub priorities: Arc<DashMap<NodeId, Priority>>,

    // Readiness tracking (encapsulated)
    pub(crate) ready_set: ReadySet,
    pub(crate) tokens: Arc<DashMap<TokenId, TokenState>>,
    pub(crate) op_states: Arc<DashMap<NodeId, OpState>>,

    // Work-stealing scheduler (encapsulated)
    pub work_stealing: Arc<WorkStealingScheduler>,
    pub queue: Arc<PriorityQueue>,

    // Concurrency control (encapsulated)
    pub concurrency: ConcurrencyControl,

    /// Separate concurrency controller for LLM operations (Ask/Think/Reason).
    ///
    /// Decouples LLM fan-out (request concurrency) from compute fan-out (CPU
    /// cores) so a continuous-batching backend can be saturated without
    /// inflating compute parallelism.
    pub llm_concurrency: ConcurrencyControl,

    // Coordination
    pub executed: Arc<AtomicUsize>,
    pub failed: Arc<AtomicUsize>,
    pub remaining: Arc<AtomicUsize>,
    pub notify_done: Arc<Notify>,
    /// Edge-triggered wake for idle workers. Ready-node producers signal this
    /// so workers can sleep instead of polling the steal queue.
    pub work_notify: Arc<Notify>,
    /// Edge-triggered wake for the watchdog. Workers signal this on
    /// progress/completion so the watchdog blocks instead of polling.
    pub watchdog_notify: Arc<Notify>,
    pub first_error: Arc<Mutex<Option<RuntimeError>>>,
    pub last_progress_ms: Arc<AtomicU64>,
    pub exit_nodes: Vec<NodeId>,

    // Promise tracking for flow calls
    pub pending_promises: Arc<DashMap<TokenId, PromiseState>>,
    pub execution_stack: Arc<Mutex<Vec<ExecutionFrame>>>,
    pub next_promise_token_id: Arc<AtomicU64>,

    /// Sparse set tracking delegated tokens for switch/case sub-DAG splicing.
    /// Key: (delegator_node_id, token_id) - only the delegating node skips publish.
    /// Zero overhead for DAGs without switch operations.
    pub delegated_tokens: Arc<DashSet<(NodeId, TokenId)>>,
}

impl SchedulerState {
    /// Create scheduler state, optionally with initial input values for entry tokens.
    ///
    /// When `inputs` is non-empty, the values are assigned to flow parameter tokens
    /// (input tokens that have no producer) in order.
    pub fn new(
        dag: ExecutionDag,
        cfg: SchedulerConfig,
        metrics: Arc<MetricsCollector>,
        start: Instant,
        inputs: Vec<Value>,
    ) -> RuntimeResult<(Self, Vec<Worker<NodeId>>)> {
        Self::new_with_hooks(
            dag,
            cfg,
            metrics,
            start,
            inputs,
            ExecutionHookContext::default(),
        )
    }

    pub fn new_with_hooks(
        dag: ExecutionDag,
        cfg: SchedulerConfig,
        metrics: Arc<MetricsCollector>,
        start: Instant,
        inputs: Vec<Value>,
        hooks: ExecutionHookContext,
    ) -> RuntimeResult<(Self, Vec<Worker<NodeId>>)> {
        // Validate configuration
        cfg.validate().map_err(|msg| RuntimeError::Scheduler {
            message: format!("Invalid scheduler config: {}", msg),
        })?;

        let dag_snapshot = Arc::new(dag.clone());

        // Build parameter substitution maps BEFORE consuming inputs
        // Named map: {{PARAM_NAME}} -> value
        let param_map: HashMap<String, String> = dag
            .metadata
            .parameters
            .iter()
            .zip(&inputs)
            .map(|(param, value)| {
                let value_str = match value {
                    Value::String(s) => s.clone(),
                    v => format!("{}", v),
                };
                (param.name.clone(), value_str)
            })
            .collect();

        // Positional map: {0}, {1}, ... -> value
        let positional_map: Vec<String> = inputs
            .iter()
            .map(|value| match value {
                Value::String(s) => s.clone(),
                v => format!("{}", v),
            })
            .collect();

        // Only compute input_map if we have inputs (zero-cost when empty)
        let input_map: Option<HashMap<TokenId, Value>> = if inputs.is_empty() {
            None
        } else {
            // Find tokens with no producer (flow parameters)
            let output_tokens: std::collections::HashSet<TokenId> = dag
                .nodes
                .iter()
                .flat_map(|n| n.output_tokens.iter().copied())
                .collect();

            let mut seen = std::collections::HashSet::new();
            let entry_tokens: Vec<TokenId> = dag
                .nodes
                .iter()
                .flat_map(|n| n.input_tokens.iter().copied())
                .filter(|tid| !output_tokens.contains(tid))
                .filter(|tid| seen.insert(*tid))
                .collect();

            Some(entry_tokens.into_iter().zip(inputs).collect())
        };

        // Build node and priority maps with parameter substitution
        let nodes: Arc<DashMap<NodeId, Arc<Node>>> = Arc::new(
            dag.nodes
                .iter()
                .map(|n| {
                    let mut node = n.clone();
                    // Substitute both {{PARAM_NAME}} and {0}, {1}, ... in all string attributes
                    if !param_map.is_empty() || !positional_map.is_empty() {
                        for (_key, value) in node.attributes.iter_mut() {
                            if let Value::String(s) = value {
                                // Substitute named {{PARAM_NAME}} placeholders
                                for (param_name, param_value) in &param_map {
                                    let placeholder = format!("{{{{{}}}}}", param_name);
                                    *s = s.replace(&placeholder, param_value);
                                }
                                // Substitute positional {0}, {1}, ... placeholders
                                for (i, param_value) in positional_map.iter().enumerate() {
                                    let placeholder = format!("{{{}}}", i);
                                    *s = s.replace(&placeholder, param_value);
                                }
                            }
                        }
                    }
                    (n.id, Arc::new(node))
                })
                .collect(),
        );

        let priorities: Arc<DashMap<NodeId, Priority>> = Arc::new(
            dag.nodes
                .iter()
                .map(|n| (n.id, Priority::from_u8(n.metadata.priority as u8)))
                .collect(),
        );

        // Initialize token and operation state
        let tokens = Arc::new(DashMap::new());
        let op_states = Arc::new(DashMap::new());
        materialize_graph_state(&dag, &tokens, &op_states, input_map.as_ref())?;

        // Create priority queue and work-stealing scheduler
        let queue = Arc::new(PriorityQueue::new());
        let worker_count = cfg.max_concurrency.max(cfg.llm_inflight);
        let (work_stealing, workers) = WorkStealingScheduler::new(worker_count, Arc::clone(&queue));
        let work_stealing = Arc::new(work_stealing);

        // Create readiness tracker
        let ready_set = ReadySet::new();

        // Create concurrency controllers: a general semaphore for compute-bound
        // ops and a separate one for LLM ops so the two pools don't starve
        // each other under remote-batched serving.
        let concurrency = ConcurrencyControl::new(cfg.max_inflight);
        let llm_concurrency = ConcurrencyControl::new(cfg.llm_inflight);

        // Build state
        let state = Self {
            cfg: cfg.clone(),
            metrics,
            start,
            hooks,

            dag: dag_snapshot,
            nodes,
            priorities,

            ready_set,
            tokens,
            op_states,

            work_stealing,
            queue: Arc::clone(&queue),

            concurrency,
            llm_concurrency,

            executed: Arc::new(AtomicUsize::new(0)),
            failed: Arc::new(AtomicUsize::new(0)),
            remaining: Arc::new(AtomicUsize::new(dag.nodes.len())),
            notify_done: Arc::new(Notify::new()),
            work_notify: Arc::new(Notify::new()),
            watchdog_notify: Arc::new(Notify::new()),
            first_error: Arc::new(Mutex::new(None)),
            last_progress_ms: Arc::new(AtomicU64::new(0)),
            exit_nodes: dag.exit_nodes.clone(),

            pending_promises: Arc::new(DashMap::new()),
            execution_stack: Arc::new(Mutex::new(Vec::new())),
            next_promise_token_id: Arc::new(AtomicU64::new(1_000_000)),
            delegated_tokens: Arc::new(DashSet::new()),
        };

        // Initialize readiness tracking and seed ready nodes
        let ready_nodes = state.ready_set.initialize(
            &dag.nodes,
            &state.tokens,
            &state.priorities,
            &state.op_states,
            &state.queue,
        )?;
        state.emit_node_ready_batch(&ready_nodes);
        if !ready_nodes.is_empty() {
            state.work_notify.notify_waiters();
        }

        // Initialize last progress timestamp
        state
            .last_progress_ms
            .store(state.elapsed_ms() as u64, Ordering::Relaxed);

        Ok((state, workers))
    }

    #[inline]
    pub fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    #[inline]
    pub fn mark_done(&self) {
        tracing::debug!("mark_done called, setting remaining to 0");
        self.remaining.store(0, Ordering::SeqCst);
        self.concurrency.cancel();
        self.llm_concurrency.cancel();
        self.notify_done.notify_waiters();
        self.work_notify.notify_waiters();
        self.watchdog_notify.notify_one();
    }

    /// Get a cloneable handle to the concurrency controller.
    pub fn concurrency_handle(&self) -> ConcurrencyControlHandle {
        self.concurrency.handle()
    }

    /// Check if execution has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.concurrency.is_cancelled()
    }

    /// Retrieve the effect metadata for a node, if available.
    pub fn operation_effects(&self, node_id: NodeId) -> Option<OperationEffects> {
        self.op_states
            .get(&node_id)
            .map(|entry| entry.effects().clone())
    }

    /// Set the first error if none has been set yet.
    pub fn set_first_error(&self, error: RuntimeError) {
        let mut guard = self.first_error.lock();
        if guard.is_none() {
            *guard = Some(error);
        }
    }

    /// Record progress to prevent deadlock detection.
    pub fn record_progress(&self) {
        self.last_progress_ms
            .store(self.elapsed_ms() as u64, Ordering::Relaxed);
        self.watchdog_notify.notify_one();
    }

    /// Check if any operations are currently running.
    ///
    /// Used by the watchdog to distinguish true deadlocks (nothing running,
    /// nothing becoming ready) from long-running operations (e.g. LLM calls).
    pub fn has_running_ops(&self) -> bool {
        self.op_states
            .iter()
            .any(|entry| matches!(entry.value().status, OpStatus::Running))
    }

    pub fn collect_exit_values(&self) -> RuntimeResult<HashMap<TokenId, Value>> {
        let mut results = HashMap::new();

        for &node_id in &self.exit_nodes {
            if let Some(node) = self.nodes.get(&node_id) {
                for token_id in &node.output_tokens {
                    if let Some(value) = self
                        .tokens
                        .get(token_id)
                        .and_then(|state| state.value.clone())
                    {
                        results.insert(*token_id, value);
                    }
                }
            }
        }

        Ok(results)
    }

    pub fn collect_all_values(&self) -> RuntimeResult<HashMap<TokenId, Value>> {
        let mut results = HashMap::new();
        for entry in self.tokens.iter() {
            if let Some(ref value) = entry.value().value {
                results.insert(*entry.key(), value.clone());
            }
        }
        Ok(results)
    }

    pub fn node_output_map(&self) -> HashMap<NodeId, Vec<TokenId>> {
        self.nodes
            .iter()
            .map(|entry| (*entry.key(), entry.value().output_tokens.clone()))
            .collect()
    }

    /// Create a new promise token for a flow call.
    ///
    /// The token is registered as "not ready" and will be resolved
    /// when the sub-flow completes.
    pub fn create_promise_token(&self, target_agent: String, target_flow: String) -> TokenId {
        let token_id = self.next_promise_token_id.fetch_add(1, Ordering::Relaxed);

        // Register promise state
        self.pending_promises
            .insert(token_id, PromiseState::new(target_agent, target_flow));

        // Register token as not ready (consumers will wait)
        self.tokens.insert(token_id, TokenState::new());

        token_id
    }

    /// Resolve a promise token with a value.
    ///
    /// This makes the token ready and propagates readiness to consumers.
    pub fn resolve_promise(&self, token_id: TokenId, value: Value) -> RuntimeResult<()> {
        // Update promise state
        if let Some(mut promise) = self.pending_promises.get_mut(&token_id) {
            promise.resolved = true;
            promise.value = Some(value.clone());
        }

        // Update token state (makes downstream nodes ready)
        if let Some(mut token) = self.tokens.get_mut(&token_id) {
            token.ready = true;
            token.value = Some(value);
        }

        // Propagate readiness to consumers
        let ready_nodes = self.ready_set.on_token_ready(
            token_id,
            &self.tokens,
            &self.priorities,
            &self.op_states,
            &self.queue,
        )?;
        self.emit_node_ready_batch(&ready_nodes);
        if !ready_nodes.is_empty() {
            self.work_notify.notify_waiters();
        }

        self.record_progress();
        Ok(())
    }

    /// Push an execution frame onto the stack (for sub-flow execution).
    pub fn push_execution_frame(&self, frame: ExecutionFrame) {
        let mut stack = self.execution_stack.lock();
        stack.push(frame);
    }

    /// Pop an execution frame from the stack.
    pub fn pop_execution_frame(&self) -> Option<ExecutionFrame> {
        let mut stack = self.execution_stack.lock();
        stack.pop()
    }

    /// Get the current execution stack depth.
    pub fn execution_stack_depth(&self) -> usize {
        let stack = self.execution_stack.lock();
        stack.len()
    }

    /// Project AAM goal priorities onto scheduler node priorities.
    ///
    /// For each node that has a `goal_id` attribute, look up the matching goal
    /// in the AAM (by description) and boost the node's scheduler priority to
    /// `max(compile_time_priority, goal_priority)`.
    ///
    /// This closes the gap between the AAM goal system (runtime) and the
    /// scheduler priority system (compile-time), ensuring that nodes associated
    /// with high-priority goals are scheduled first.
    pub fn apply_goal_priorities(&self, aam: &crate::aam::Aam) {
        use apxm_core::constants::graph::attrs;

        let mut updated = 0usize;
        for entry in self.nodes.iter() {
            let node = entry.value();
            let goal_desc = node
                .attributes
                .get(attrs::GOAL_ID)
                .and_then(|v| v.as_string().map(|s| s.to_string()));

            let goal_priority = if let Some(desc) = goal_desc {
                aam.goal_priority_by_description(&desc)
            } else {
                // If node has no explicit goal_id, use the top active goal's priority
                // only when the node has zero compile-time priority (i.e., default).
                if node.metadata.priority == 0 {
                    aam.active_goal_priority()
                } else {
                    None
                }
            };

            if let Some(gp) = goal_priority {
                let compile_prio = node.metadata.priority;
                let effective = compile_prio.max(gp);
                let new_level = Priority::from_u8(effective.min(255) as u8);

                if let Some(mut current) = self.priorities.get_mut(entry.key()) {
                    if new_level > *current {
                        *current = new_level;
                        updated += 1;
                    }
                }
            }
        }

        if updated > 0 {
            tracing::info!(
                updated_nodes = updated,
                "Applied AAM goal priorities to scheduler node priorities"
            );
        }
    }

    pub fn build_stats(&self) -> ExecutionStats {
        let duration_ms = self.elapsed_ms();

        let node_statuses = self
            .op_states
            .iter()
            .map(|entry| {
                let v = entry.value();
                let dur = match (v.started_at, v.finished_at) {
                    (Some(s), Some(f)) => Some(f.saturating_duration_since(s).as_millis()),
                    _ => None,
                };
                let ready_at_ms = v
                    .ready_at
                    .map(|t| t.saturating_duration_since(self.start).as_millis());
                let started_at_ms = v
                    .started_at
                    .map(|t| t.saturating_duration_since(self.start).as_millis());
                let queue_wait_ms = match (v.ready_at, v.started_at) {
                    (Some(ready_at), Some(started_at)) => {
                        Some(started_at.saturating_duration_since(ready_at).as_millis())
                    }
                    _ => None,
                };
                let priority = self
                    .priorities
                    .get(entry.key())
                    .map(|priority| priority.as_str().to_string());
                NodeStatus {
                    node_id: *entry.key(),
                    status: v.status,
                    retries: v.retries,
                    last_error: v.last_error.clone(),
                    ready_at_ms,
                    started_at_ms,
                    finished_at_ms: v
                        .finished_at
                        .map(|t| t.saturating_duration_since(self.start).as_millis()),
                    duration_ms: dur,
                    queue_wait_ms,
                    priority,
                    input_tokens: None,
                    output_tokens: None,
                }
            })
            .collect();

        let mut stats = ExecutionStats {
            executed_nodes: self.executed.load(Ordering::Relaxed),
            failed_nodes: self.failed.load(Ordering::Relaxed),
            duration_ms,
            node_statuses,
            observed_graph: None,
        };
        stats.attach_observed_graph_metrics(&self.dag);
        stats
    }

    pub(crate) fn emit_node_ready_batch(&self, node_ids: &[NodeId]) {
        if self.hooks.is_empty() {
            return;
        }
        for node_id in node_ids {
            self.emit_node_ready(*node_id);
        }
    }

    pub(crate) fn emit_node_ready(&self, node_id: NodeId) {
        if self.hooks.is_empty() {
            return;
        }
        let Some(node) = self.nodes.get(&node_id).map(|node| node.value().clone()) else {
            return;
        };
        let priority = self
            .priorities
            .get(&node_id)
            .map(|priority| *priority)
            .unwrap_or(Priority::Normal);
        let ready_at_ms = self
            .op_states
            .get(&node_id)
            .and_then(|state| {
                state
                    .ready_at
                    .map(|ready_at| ready_at.saturating_duration_since(self.start).as_millis())
            })
            .unwrap_or_else(|| self.elapsed_ms());

        self.hooks.emit_node_ready(NodeReadyEvent {
            execution_id: self.hooks.execution_id().to_string(),
            graph_id: self.hooks.graph_id().to_string(),
            node_id,
            op_type: node.op_type,
            priority: priority.as_str().to_string(),
            ready_at_ms,
        });
    }

    pub(crate) fn emit_node_started(&self, node_id: NodeId, node: &Node, worker_id: usize) {
        if self.hooks.is_empty() {
            return;
        }
        let priority = self
            .priorities
            .get(&node_id)
            .map(|priority| *priority)
            .unwrap_or(Priority::Normal);
        let Some(op_state) = self.op_states.get(&node_id) else {
            return;
        };
        let ready_at_ms = op_state
            .ready_at
            .map(|ready_at| ready_at.saturating_duration_since(self.start).as_millis());
        let started_at_ms = op_state
            .started_at
            .map(|started_at| started_at.saturating_duration_since(self.start).as_millis())
            .unwrap_or_else(|| self.elapsed_ms());
        let queue_wait_ms = match (op_state.ready_at, op_state.started_at) {
            (Some(ready_at), Some(started_at)) => {
                Some(started_at.saturating_duration_since(ready_at).as_millis())
            }
            _ => None,
        };

        self.hooks.emit_node_started(NodeStartedEvent {
            execution_id: self.hooks.execution_id().to_string(),
            graph_id: self.hooks.graph_id().to_string(),
            node_id,
            op_type: node.op_type,
            priority: priority.as_str().to_string(),
            worker_id: Some(worker_id),
            ready_at_ms,
            started_at_ms,
            queue_wait_ms,
        });
    }

    pub(crate) fn emit_node_finished(&self, node_id: NodeId, node: &Node, attempts: u32) {
        if self.hooks.is_empty() {
            return;
        }
        let Some(op_state) = self.op_states.get(&node_id) else {
            return;
        };
        let started_at_ms = op_state
            .started_at
            .map(|started_at| started_at.saturating_duration_since(self.start).as_millis());
        let finished_at_ms = op_state
            .finished_at
            .map(|finished_at| {
                finished_at
                    .saturating_duration_since(self.start)
                    .as_millis()
            })
            .unwrap_or_else(|| self.elapsed_ms());
        let duration_ms = match (op_state.started_at, op_state.finished_at) {
            (Some(started_at), Some(finished_at)) => Some(
                finished_at
                    .saturating_duration_since(started_at)
                    .as_millis(),
            ),
            _ => None,
        };

        self.hooks.emit_node_finished(NodeFinishedEvent {
            execution_id: self.hooks.execution_id().to_string(),
            graph_id: self.hooks.graph_id().to_string(),
            node_id,
            op_type: node.op_type,
            status: op_state.status,
            attempts,
            started_at_ms,
            finished_at_ms,
            duration_ms,
            error: op_state.last_error.clone(),
        });
    }
}

/// Initialize token and operation state from the DAG.
///
/// This validates that each token has exactly one producer and registers
/// consumers for each token. If `input_values` is provided, those values
/// are used for tokens that have no producer (flow parameters).
fn materialize_graph_state(
    dag: &ExecutionDag,
    tokens: &DashMap<TokenId, TokenState>,
    op_states: &DashMap<NodeId, OpState>,
    input_values: Option<&HashMap<TokenId, Value>>,
) -> RuntimeResult<()> {
    // PASS 1: Register all output tokens (producers) first.
    // This ensures that when we process input tokens, the producer's entry
    // already exists and we won't create a spurious "pre-ready" state that
    // then conflicts with the actual producer.
    for node in dag.nodes.iter() {
        op_states.insert(
            node.id,
            OpState::new_with_effects(operation_effects(&node.op_type)),
        );

        for &token_id in &node.output_tokens {
            if tokens.contains_key(&token_id) {
                tracing::debug!(
                    node_id = node.id,
                    token_id = token_id,
                    node_outputs = ?node.output_tokens,
                    "Duplicate producer detected"
                );
                return Err(RuntimeError::SchedulerDuplicateProducer { token_id });
            }
            tokens.insert(token_id, TokenState::new());
        }
    }

    // PASS 2: Register all input tokens (consumers).
    // For tokens with a producer (from pass 1), we just add the consumer.
    // For tokens without a producer (flow parameters), we create a pre-ready state.
    for node in dag.nodes.iter() {
        for &token_id in &node.input_tokens {
            tokens
                .entry(token_id)
                .or_insert({
                    // No producer for this token — it's a flow parameter or external input.
                    let mut ts = TokenState::new();
                    ts.ready = true;
                    ts.value = Some(
                        input_values
                            .and_then(|m| m.get(&token_id).cloned())
                            .unwrap_or(Value::Null),
                    );
                    ts
                })
                .consumers
                .push(node.id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::execution::FlowParameter;
    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::{DagMetadata, DependencyType, Edge, ExecutionDag, Node, Value};
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    /// Helper: build a SchedulerConfig suitable for tests.
    fn test_config() -> SchedulerConfig {
        SchedulerConfig::new()
            .with_max_concurrency(2)
            .with_max_inflight(4)
    }

    /// Helper: create a simple node with explicit input/output tokens.
    fn make_node(id: NodeId, input_tokens: Vec<TokenId>, output_tokens: Vec<TokenId>) -> Node {
        Node {
            id,
            op_type: AISOperationType::Nop,
            attributes: HashMap::new(),
            input_tokens,
            output_tokens,
            metadata: NodeMetadata::default(),
        }
    }

    /// Helper: build a linear 2-node DAG.
    ///
    ///   [Node 1] --token 10--> [Node 2]
    ///
    /// Node 1 has no inputs (entry); Node 2 has no outgoing edges (exit).
    fn two_node_dag() -> ExecutionDag {
        let n1 = make_node(1, vec![], vec![10]);
        let n2 = make_node(2, vec![10], vec![20]);

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.add_node(n2).unwrap();
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag
    }

    /// Helper: build a fan-out 3-node DAG.
    ///
    ///   [Node 1] --token 10--> [Node 2]
    ///            \--token 11--> [Node 3]
    ///
    fn fan_out_dag() -> ExecutionDag {
        let n1 = make_node(1, vec![], vec![10, 11]);
        let n2 = make_node(2, vec![10], vec![20]);
        let n3 = make_node(3, vec![11], vec![30]);

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.add_node(n2).unwrap();
        dag.add_node(n3).unwrap();
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .unwrap();
        dag.add_edge(Edge::new(1, 3, 11, DependencyType::Data))
            .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag
    }

    // ── SchedulerState::new() tests ────────────────────────────────────

    #[test]
    fn test_new_two_node_dag_creates_tokens() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        // Worker pool size is `max(max_concurrency, llm_inflight)`; pin
        // llm_inflight to max_concurrency so the count is deterministic. The
        // default llm_inflight is 32, which this assertion predated (it asserted
        // a node-count-based 2 and silently failed once worker sizing changed).
        let cfg = test_config().with_llm_inflight(2);
        let (state, workers) =
            SchedulerState::new(dag, cfg, metrics, Instant::now(), vec![]).unwrap();

        // Token 10 (produced by node 1, consumed by node 2) should exist
        assert!(state.tokens.contains_key(&10));
        // Token 20 (produced by node 2, no consumer) should exist
        assert!(state.tokens.contains_key(&20));
        // Workers should be sized to the configured concurrency.
        assert_eq!(workers.len(), 2);
    }

    #[test]
    fn test_new_two_node_dag_nodes_stored() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        assert_eq!(state.nodes.len(), 2);
        assert!(state.nodes.contains_key(&1));
        assert!(state.nodes.contains_key(&2));
    }

    #[test]
    fn test_new_two_node_dag_exit_nodes() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        assert_eq!(state.exit_nodes, vec![2]);
    }

    #[test]
    fn test_new_two_node_dag_op_states() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // Both nodes should have OpState entries
        assert!(state.op_states.contains_key(&1));
        assert!(state.op_states.contains_key(&2));

        // Node 1 (entry, no inputs) should be Ready
        let n1_status = state.op_states.get(&1).unwrap().status;
        assert_eq!(n1_status, OpStatus::Ready);

        // Node 2 (waiting for token 10) should still be Pending
        let n2_status = state.op_states.get(&2).unwrap().status;
        assert_eq!(n2_status, OpStatus::Pending);
    }

    #[test]
    fn test_new_two_node_dag_remaining_count() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        assert_eq!(state.remaining.load(Ordering::SeqCst), 2);
        assert_eq!(state.executed.load(Ordering::Relaxed), 0);
        assert_eq!(state.failed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_new_fan_out_dag_tokens_and_consumers() {
        let dag = fan_out_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // Token 10 should have node 2 as consumer
        let t10 = state.tokens.get(&10).unwrap();
        assert!(t10.consumers.contains(&2));

        // Token 11 should have node 3 as consumer
        let t11 = state.tokens.get(&11).unwrap();
        assert!(t11.consumers.contains(&3));
    }

    #[test]
    fn test_new_fan_out_dag_entry_ready() {
        let dag = fan_out_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // Node 1 (entry) should be enqueued in the priority queue
        let n1_status = state.op_states.get(&1).unwrap().status;
        assert_eq!(n1_status, OpStatus::Ready);

        // Nodes 2 and 3 should be pending (waiting for tokens from node 1)
        let n2_status = state.op_states.get(&2).unwrap().status;
        assert_eq!(n2_status, OpStatus::Pending);
        let n3_status = state.op_states.get(&3).unwrap().status;
        assert_eq!(n3_status, OpStatus::Pending);
    }

    #[test]
    fn test_new_fan_out_dag_remaining() {
        let dag = fan_out_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        assert_eq!(state.remaining.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_new_with_inputs() {
        // Build a node that has an external input (token 50 not produced by any node)
        let n1 = make_node(1, vec![50], vec![10]);
        let n2 = make_node(2, vec![10], vec![20]);

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.add_node(n2).unwrap();
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let input_val = Value::String("hello".to_string());
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) = SchedulerState::new(
            dag,
            test_config(),
            metrics,
            Instant::now(),
            vec![input_val.clone()],
        )
        .unwrap();

        // Token 50 should be ready with the injected value
        let t50 = state.tokens.get(&50).unwrap();
        assert!(t50.ready);
        assert_eq!(t50.value, Some(input_val));
    }

    #[test]
    fn test_new_substitutes_named_runtime_parameters_in_node_attributes() {
        let mut node = make_node(1, vec![], vec![10]);
        node.op_type = AISOperationType::Communicate;
        node.attributes.insert(
            apxm_core::constants::graph::attrs::MESSAGE.to_string(),
            Value::String("Task: {{task}}".to_string()),
        );

        let mut dag = ExecutionDag::new();
        dag.add_node(node).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag.metadata = DagMetadata {
            name: Some("parameterized".to_string()),
            is_entry: true,
            parameters: vec![FlowParameter {
                name: "task".to_string(),
                type_name: "str".to_string(),
            }],
        };

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) = SchedulerState::new(
            dag,
            test_config(),
            metrics,
            Instant::now(),
            vec![Value::String("review the demo".to_string())],
        )
        .unwrap();

        let stored = state.nodes.get(&1).expect("node should be stored");
        let message = stored
            .attributes
            .get(apxm_core::constants::graph::attrs::MESSAGE)
            .and_then(Value::as_str)
            .expect("message attribute");
        assert_eq!(message, "Task: review the demo");
    }

    #[test]
    fn test_new_invalid_config_errors() {
        let dag = two_node_dag();
        let bad_cfg = SchedulerConfig {
            max_concurrency: 0,
            ..SchedulerConfig::default()
        };
        let metrics = Arc::new(MetricsCollector::new());
        let result = SchedulerState::new(dag, bad_cfg, metrics, Instant::now(), vec![]);
        assert!(result.is_err());
    }

    // ── materialize_graph_state tests ──────────────────────────────────

    #[test]
    fn test_materialize_duplicate_producer_errors() {
        // Two nodes both claiming to produce token 10
        let n1 = make_node(1, vec![], vec![10]);
        let n2 = make_node(2, vec![], vec![10]);

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.add_node(n2).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let tokens = DashMap::new();
        let op_states = DashMap::new();
        let result = materialize_graph_state(&dag, &tokens, &op_states, None);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::SchedulerDuplicateProducer { token_id: 10 }
        ));
    }

    #[test]
    fn test_materialize_unproduced_input_is_pre_ready_null() {
        // Node 1 consumes token 50 which nobody produces
        let n1 = make_node(1, vec![50], vec![10]);

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let tokens = DashMap::new();
        let op_states = DashMap::new();
        materialize_graph_state(&dag, &tokens, &op_states, None).unwrap();

        let t50 = tokens.get(&50).unwrap();
        assert!(t50.ready);
        assert_eq!(t50.value, Some(Value::Null));
        assert!(t50.consumers.contains(&1));
    }

    #[test]
    fn test_materialize_with_input_values() {
        let n1 = make_node(1, vec![50, 51], vec![10]);

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let tokens = DashMap::new();
        let op_states = DashMap::new();

        let mut input_map = HashMap::new();
        input_map.insert(50, Value::String("val_a".to_string()));
        // token 51 is not in the input map -- should get Null

        materialize_graph_state(&dag, &tokens, &op_states, Some(&input_map)).unwrap();

        let t50 = tokens.get(&50).unwrap();
        assert!(t50.ready);
        assert_eq!(t50.value, Some(Value::String("val_a".to_string())));

        let t51 = tokens.get(&51).unwrap();
        assert!(t51.ready);
        assert_eq!(t51.value, Some(Value::Null));
    }

    // ── Helper method tests ────────────────────────────────────────────

    #[test]
    fn test_mark_done() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        assert_eq!(state.remaining.load(Ordering::SeqCst), 2);
        assert!(!state.is_cancelled());

        state.mark_done();

        assert_eq!(state.remaining.load(Ordering::SeqCst), 0);
        assert!(state.is_cancelled());
    }

    #[test]
    fn test_set_first_error_only_once() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        let error1 = RuntimeError::Scheduler {
            message: "first".to_string(),
        };
        let error2 = RuntimeError::Scheduler {
            message: "second".to_string(),
        };

        state.set_first_error(error1);
        state.set_first_error(error2);

        let guard = state.first_error.lock();
        match guard.as_ref().unwrap() {
            RuntimeError::Scheduler { message } => assert_eq!(message, "first"),
            _ => panic!("Expected Scheduler error"),
        }
    }

    #[test]
    fn test_has_running_ops() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // Initially no running ops
        assert!(!state.has_running_ops());

        // Set node 1 to Running
        if let Some(mut op) = state.op_states.get_mut(&1) {
            op.status = OpStatus::Running;
        }
        assert!(state.has_running_ops());
    }

    #[test]
    fn test_collect_exit_values_empty_before_execution() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // Exit node is node 2, its output token is 20.
        // Token 20 has no value set yet.
        let results = state.collect_exit_values().unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_collect_exit_values_after_publish() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // Simulate publishing a value on exit token 20
        if let Some(mut tok) = state.tokens.get_mut(&20) {
            tok.ready = true;
            tok.value = Some(Value::String("output".to_string()));
        }

        let results = state.collect_exit_values().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results.get(&20), Some(&Value::String("output".to_string())));
    }

    #[test]
    fn test_execution_stack() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        assert_eq!(state.execution_stack_depth(), 0);

        state.push_execution_frame(ExecutionFrame {
            execution_id: "exec-1".to_string(),
            flow_name: "main".to_string(),
            parent_promise: None,
        });
        assert_eq!(state.execution_stack_depth(), 1);

        state.push_execution_frame(ExecutionFrame {
            execution_id: "exec-2".to_string(),
            flow_name: "sub".to_string(),
            parent_promise: Some(100),
        });
        assert_eq!(state.execution_stack_depth(), 2);

        let popped = state.pop_execution_frame().unwrap();
        assert_eq!(popped.flow_name, "sub");
        assert_eq!(state.execution_stack_depth(), 1);

        let popped = state.pop_execution_frame().unwrap();
        assert_eq!(popped.flow_name, "main");
        assert_eq!(state.execution_stack_depth(), 0);

        assert!(state.pop_execution_frame().is_none());
    }

    #[test]
    fn test_create_promise_token() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        let token_id = state.create_promise_token("agent_x".to_string(), "flow_y".to_string());

        // Token should exist in tokens map, not ready
        let tok = state.tokens.get(&token_id).unwrap();
        assert!(!tok.ready);
        assert!(tok.value.is_none());

        // Promise should exist in pending_promises
        let promise = state.pending_promises.get(&token_id).unwrap();
        assert_eq!(promise.target_agent, "agent_x");
        assert_eq!(promise.target_flow, "flow_y");
        assert!(!promise.resolved);
    }

    #[tokio::test]
    async fn test_resolve_promise_token() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        let token_id = state.create_promise_token("a".to_string(), "f".to_string());
        let val = Value::String("resolved_value".to_string());

        state.resolve_promise(token_id, val.clone()).unwrap();

        // Token should be ready with value
        let tok = state.tokens.get(&token_id).unwrap();
        assert!(tok.ready);
        assert_eq!(tok.value, Some(val.clone()));

        // Promise should be resolved
        let promise = state.pending_promises.get(&token_id).unwrap();
        assert!(promise.resolved);
        assert_eq!(promise.value, Some(val));
    }

    #[test]
    fn test_build_stats() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let start = Instant::now();
        let (state, _) = SchedulerState::new(dag, test_config(), metrics, start, vec![]).unwrap();

        state.executed.store(1, Ordering::Relaxed);
        state.failed.store(1, Ordering::Relaxed);
        if let Some(mut op) = state.op_states.get_mut(&1) {
            op.status = OpStatus::Completed;
            op.ready_at = Some(start + Duration::from_millis(5));
            op.started_at = Some(start + Duration::from_millis(15));
            op.finished_at = Some(start + Duration::from_millis(45));
        }
        if let Some(mut op) = state.op_states.get_mut(&2) {
            op.status = OpStatus::Completed;
            op.ready_at = Some(start + Duration::from_millis(50));
            op.started_at = Some(start + Duration::from_millis(70));
            op.finished_at = Some(start + Duration::from_millis(120));
        }

        let stats = state.build_stats();
        assert_eq!(stats.executed_nodes, 1);
        assert_eq!(stats.failed_nodes, 1);
        assert_eq!(stats.node_statuses.len(), 2);
        let node_one = stats
            .node_statuses
            .iter()
            .find(|status| status.node_id == 1)
            .unwrap();
        assert_eq!(node_one.ready_at_ms, Some(5));
        assert_eq!(node_one.started_at_ms, Some(15));
        assert_eq!(node_one.queue_wait_ms, Some(10));
        assert_eq!(node_one.priority.as_deref(), Some("low"));
        let observed = stats.observed_graph.as_ref().unwrap();
        assert_eq!(observed.critical_path.nodes, vec![1, 2]);
        assert_eq!(observed.critical_path.duration_ms, 80);
        assert_eq!(observed.queue_wait.total_ms, 30);
    }

    #[test]
    fn test_priorities_are_tracked() {
        // Build a node with non-default priority
        let mut n1 = make_node(1, vec![], vec![10]);
        n1.metadata.priority = 90; // Critical

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        use crate::scheduler::queue::Priority;
        let prio = state.priorities.get(&1).unwrap();
        assert_eq!(*prio, Priority::Critical);
    }

    // ── Goal-aware scheduling tests ──────────────────────────────────

    #[test]
    fn test_apply_goal_priorities_boosts_node_with_goal_id() {
        use crate::aam::{Aam, GoalStatus, TransitionLabel};
        use apxm_core::types::goal::{Goal, GoalId};

        // Create a node with goal_id attribute and low compile-time priority
        let mut n1 = make_node(1, vec![], vec![10]);
        n1.attributes.insert(
            "goal_id".to_string(),
            Value::String("high_priority_goal".to_string()),
        );
        n1.metadata.priority = 10; // Low compile-time priority

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // Before: priority should be Low (10 -> Low)
        let prio_before = *state.priorities.get(&1).unwrap();
        assert_eq!(prio_before, Priority::Low);

        // Create AAM with a high-priority active goal matching the description
        let aam = Aam::new();
        let goal = Goal {
            id: GoalId::new(),
            description: "high_priority_goal".into(),
            priority: 95, // Critical
            status: GoalStatus::Active,
            parent_id: None,
        };
        aam.add_goal(goal, TransitionLabel::custom("test"));

        // Apply goal priorities
        state.apply_goal_priorities(&aam);

        // After: priority should be boosted to Critical (max(10, 95) = 95 -> Critical)
        let prio_after = *state.priorities.get(&1).unwrap();
        assert_eq!(prio_after, Priority::Critical);
    }

    #[test]
    fn test_apply_goal_priorities_no_downgrade() {
        use crate::aam::{Aam, GoalStatus, TransitionLabel};
        use apxm_core::types::goal::{Goal, GoalId};

        // Node with high compile-time priority
        let mut n1 = make_node(1, vec![], vec![10]);
        n1.attributes
            .insert("goal_id".to_string(), Value::String("low_goal".to_string()));
        n1.metadata.priority = 95; // Critical compile-time priority

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // Before: Critical
        assert_eq!(*state.priorities.get(&1).unwrap(), Priority::Critical);

        // AAM goal with low priority
        let aam = Aam::new();
        let goal = Goal {
            id: GoalId::new(),
            description: "low_goal".into(),
            priority: 10, // Low
            status: GoalStatus::Active,
            parent_id: None,
        };
        aam.add_goal(goal, TransitionLabel::custom("test"));

        state.apply_goal_priorities(&aam);

        // Should still be Critical (not downgraded)
        assert_eq!(*state.priorities.get(&1).unwrap(), Priority::Critical);
    }

    #[test]
    fn test_apply_goal_priorities_default_nodes_get_active_goal_boost() {
        use crate::aam::{Aam, GoalStatus, TransitionLabel};
        use apxm_core::types::goal::{Goal, GoalId};

        // Node with no goal_id attribute and zero compile-time priority
        let n1 = make_node(1, vec![], vec![10]);
        // metadata.priority is 0 by default

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // Before: Low (priority 0)
        assert_eq!(*state.priorities.get(&1).unwrap(), Priority::Low);

        // AAM with high-priority active goal (no goal_id on node, but active goal exists)
        let aam = Aam::new();
        let goal = Goal {
            id: GoalId::new(),
            description: "important task".into(),
            priority: 70, // High
            status: GoalStatus::Active,
            parent_id: None,
        };
        aam.add_goal(goal, TransitionLabel::custom("test"));

        state.apply_goal_priorities(&aam);

        // Should be boosted to High (from active goal)
        assert_eq!(*state.priorities.get(&1).unwrap(), Priority::High);
    }

    #[test]
    fn test_apply_goal_priorities_inactive_goals_ignored() {
        use crate::aam::{Aam, GoalStatus, TransitionLabel};
        use apxm_core::types::goal::{Goal, GoalId};

        let n1 = make_node(1, vec![], vec![10]);

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        // AAM with a completed (not active) goal
        let aam = Aam::new();
        let goal = Goal {
            id: GoalId::new(),
            description: "completed task".into(),
            priority: 95,
            status: GoalStatus::Completed,
            parent_id: None,
        };
        aam.add_goal(goal, TransitionLabel::custom("test"));

        state.apply_goal_priorities(&aam);

        // Should remain Low (completed goals are not active)
        assert_eq!(*state.priorities.get(&1).unwrap(), Priority::Low);
    }

    #[test]
    fn test_apply_goal_priorities_no_goals_no_change() {
        let n1 = make_node(1, vec![], vec![10]);

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        let aam = crate::aam::Aam::new(); // empty AAM

        state.apply_goal_priorities(&aam);

        // Should remain Low
        assert_eq!(*state.priorities.get(&1).unwrap(), Priority::Low);
    }

    #[test]
    fn test_apply_goal_priorities_nonzero_compile_priority_not_boosted_without_goal_id() {
        use crate::aam::{Aam, GoalStatus, TransitionLabel};
        use apxm_core::types::goal::{Goal, GoalId};

        // Node with explicit compile-time priority but no goal_id
        let mut n1 = make_node(1, vec![], vec![10]);
        n1.metadata.priority = 35; // Normal

        let mut dag = ExecutionDag::new();
        dag.add_node(n1).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        assert_eq!(*state.priorities.get(&1).unwrap(), Priority::Normal);

        // AAM with high-priority active goal
        let aam = Aam::new();
        let goal = Goal {
            id: GoalId::new(),
            description: "important".into(),
            priority: 95,
            status: GoalStatus::Active,
            parent_id: None,
        };
        aam.add_goal(goal, TransitionLabel::custom("test"));

        state.apply_goal_priorities(&aam);

        // Should NOT be boosted because node has nonzero compile-time priority
        // and no goal_id attribute -- the active goal boost only applies to
        // zero-priority nodes without an explicit goal_id.
        assert_eq!(*state.priorities.get(&1).unwrap(), Priority::Normal);
    }

    #[tokio::test]
    async fn test_llm_concurrency_independent_of_compute_concurrency() {
        // Exhaust the LLM semaphore; compute permits must remain available.
        // This is the core invariant behind Step 5: LLM fan-out can saturate
        // without throttling compute-bound work, and vice versa.
        let cfg = SchedulerConfig::new()
            .with_max_concurrency(2)
            .with_max_inflight(2)
            .with_llm_inflight(1);
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) = SchedulerState::new(dag, cfg, metrics, Instant::now(), vec![]).unwrap();

        // Take the only LLM permit.
        let llm_permit = state.llm_concurrency.acquire().await.expect("llm acquire");
        assert_eq!(state.llm_concurrency.available_permits(), 0);

        // Compute permits are unaffected.
        assert_eq!(state.concurrency.available_permits(), 2);
        let compute_permit = state
            .concurrency
            .try_acquire()
            .expect("compute permit must still be available");
        drop(compute_permit);
        drop(llm_permit);
    }

    #[test]
    fn test_record_progress_updates_timestamp() {
        let dag = two_node_dag();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![]).unwrap();

        let before = state.last_progress_ms.load(Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(2));
        state.record_progress();
        let after = state.last_progress_ms.load(Ordering::Relaxed);

        assert!(after >= before);
    }
}
