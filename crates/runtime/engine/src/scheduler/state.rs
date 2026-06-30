//! Scheduler state management.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use apxm_core::types::{
    ExecutionDag, ExecutionStats, Node, NodeId, NodeStatus, OpStatus, TokenId, Value,
};
use apxm_core::utils::template::{parse_placeholder_names, placeholder_root};
use crossbeam_deque::Worker;
use dashmap::{DashMap, DashSet};
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::executor::CancellationToken;
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

fn render_runtime_parameter_placeholders(
    template: &str,
    named: &HashMap<String, Value>,
    positional: &[String],
) -> RuntimeResult<String> {
    let mut out = template.to_string();

    // Preserve the explicit named form first.
    for (param_name, param_value) in named {
        let placeholder = format!("{{{{{param_name}}}}}");
        out = out.replace(&placeholder, &value_to_template_string(param_value));
    }

    // Dotted selectors use the same authored template grammar as node inputs:
    // `{data.event.subject}` resolves root `data` from flow parameters and then
    // navigates JSON fields. Non-parameter placeholders such as `{respond}` are
    // left untouched for the operation handler to resolve from `input_names`.
    for placeholder in parse_placeholder_names(template) {
        let root = placeholder_root(placeholder);
        let Some(root_value) = named.get(root) else {
            continue;
        };
        let replacement = if placeholder == root {
            value_to_template_string(root_value)
        } else {
            let value = resolve_parameter_placeholder(root_value, placeholder).ok_or_else(
                || RuntimeError::Scheduler {
                    message: format!(
                        "runtime parameter placeholder '{{{placeholder}}}' could not be resolved"
                    ),
                },
            )?;
            value_to_template_string(&value)
        };
        out = out.replace(&format!("{{{placeholder}}}"), &replacement);
    }

    // Preserve positional `{0}`, `{1}`, ... substitution.
    for (i, param_value) in positional.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), param_value);
    }

    Ok(out)
}

fn render_runtime_parameter_params_json(
    params_json: &str,
    named: &HashMap<String, Value>,
    positional: &[String],
) -> RuntimeResult<String> {
    let mut value = serde_json::from_str::<serde_json::Value>(params_json).map_err(|err| {
        RuntimeError::Scheduler {
            message: format!("invalid params_json before parameter substitution: {err}"),
        }
    })?;
    render_runtime_parameter_json_value(&mut value, named, positional)?;
    serde_json::to_string(&value).map_err(|err| RuntimeError::Scheduler {
        message: format!("could not serialize params_json after parameter substitution: {err}"),
    })
}

fn render_runtime_parameter_json_value(
    value: &mut serde_json::Value,
    named: &HashMap<String, Value>,
    positional: &[String],
) -> RuntimeResult<()> {
    match value {
        serde_json::Value::String(s) => {
            *s = render_runtime_parameter_placeholders(s, named, positional)?;
        }
        serde_json::Value::Array(items) => {
            for item in items {
                render_runtime_parameter_json_value(item, named, positional)?;
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                render_runtime_parameter_json_value(item, named, positional)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn resolve_parameter_placeholder(root_value: &Value, placeholder: &str) -> Option<Value> {
    let (_, path) = placeholder.split_once('.')?;
    let mut cur = root_value.to_json().ok()?;
    for seg in path.split('.') {
        cur = match cur {
            serde_json::Value::Object(map) => map.get(seg)?.clone(),
            serde_json::Value::Array(arr) => arr.get(seg.parse::<usize>().ok()?)?.clone(),
            _ => return None,
        };
    }
    Value::try_from(cur).ok()
}

fn value_to_template_string(value: &Value) -> String {
    value
        .as_string()
        .map_or_else(|| format!("{value}"), |s| s.to_string())
}

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

    /// Per-session re-arm turn counter for the in-graph conversation loop, keyed
    /// by session id. The park-based recv re-arm increments this each turn and
    /// stops re-arming once the recv node's `max_iterations` cap is reached, so a
    /// long-lived session loop is bounded rather than splicing nodes forever
    /// (CONV-1: bounds the previously-unbounded re-arm growth).
    pub(crate) rearm_turns: Arc<DashMap<String, u64>>,

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

    /// Concurrency controller for long-WAITING ops (PAUSE/RESUME/recv) that block
    /// on an external event. Kept separate and generous so a burst of
    /// human-in-the-loop pauses cannot exhaust the compute or LLM pools and stall
    /// real work.
    pub blocking_concurrency: ConcurrencyControl,
    pub cancellation_token: CancellationToken,

    // Coordination
    pub executed: Arc<AtomicUsize>,
    pub failed: Arc<AtomicUsize>,
    pub remaining: Arc<AtomicUsize>,
    /// Nodes currently PARKED on an external event (yielded their worker + permit
    /// via [`crate::scheduler::park_registry`]). A node here has NOT finished, so
    /// it counts in `remaining`; the watchdog excludes parked nodes so a
    /// legitimately-waiting DAG is never mistaken for a deadlock.
    pub parked: Arc<AtomicUsize>,
    /// Host admission key (stamped by the server into `metadata`). While this
    /// execution has any parked node, its cross-execution admission slot is
    /// released via [`crate::scheduler::admission_registry`]; it is reacquired
    /// (best-effort) when no nodes remain parked. `None` = not admission-managed
    /// (tests / non-server runs) → no-op.
    pub admission_id: Option<String>,
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
        Self::new_with_replay(dag, cfg, metrics, start, inputs, hooks, None)
    }

    /// Create scheduler state for a partial replay (`rerun-from-node`).
    ///
    /// When `replay_seed` is `Some`, the upstream nodes it names are pre-completed
    /// (never enqueued, so their handlers are never re-invoked) and their boundary
    /// output tokens are seeded ready with the prior run's values. Only the seed's
    /// `from_node` and its descendants are scheduled to execute. When `None`, this
    /// is an ordinary full run.
    pub fn new_with_replay(
        dag: ExecutionDag,
        cfg: SchedulerConfig,
        metrics: Arc<MetricsCollector>,
        start: Instant,
        inputs: Vec<Value>,
        hooks: ExecutionHookContext,
        replay_seed: Option<&crate::scheduler::replay::ReplaySeed>,
    ) -> RuntimeResult<(Self, Vec<Worker<NodeId>>)> {
        // Validate configuration
        cfg.validate().map_err(|msg| RuntimeError::Scheduler {
            message: format!("Invalid scheduler config: {}", msg),
        })?;

        let dag_snapshot = Arc::new(dag.clone());

        // Build parameter substitution maps BEFORE consuming inputs.
        let param_map: HashMap<String, Value> = dag
            .metadata
            .parameters
            .iter()
            .zip(&inputs)
            .map(|(param, value)| (param.name.clone(), value.clone()))
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

        // Build node and priority maps with parameter substitution.
        let node_map: DashMap<NodeId, Arc<Node>> = DashMap::new();
        for n in &dag.nodes {
            let mut node = n.clone();
            // Substitute flow parameters in all string attributes. This includes
            // `{{PARAM_NAME}}`, positional `{0}`, and named JSON selectors
            // such as `{data.event.subject}`.
            if !param_map.is_empty() || !positional_map.is_empty() {
                for (key, value) in node.attributes.iter_mut() {
                    if let Value::String(s) = value {
                        *s = if key == apxm_core::constants::graph::attrs::PARAMS_JSON {
                            render_runtime_parameter_params_json(s, &param_map, &positional_map)?
                        } else {
                            render_runtime_parameter_placeholders(s, &param_map, &positional_map)?
                        };
                    }
                }
            }
            node_map.insert(n.id, Arc::new(node));
        }
        let nodes: Arc<DashMap<NodeId, Arc<Node>>> = Arc::new(node_map);

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

        // Partial-replay seeding (rerun-from-node): pre-fill the prior run's
        // boundary token values and mark the upstream nodes Completed so they are
        // never enqueued (their handlers are never re-invoked). `remaining` then
        // starts at the count of nodes that actually replay, preserving the
        // finish-count invariant.
        let mut precompleted = 0usize;
        if let Some(seed) = replay_seed {
            for (&token_id, value) in &seed.seed_tokens {
                let mut entry = tokens.entry(token_id).or_insert_with(TokenState::new);
                entry.ready = true;
                entry.value = Some(value.clone());
            }
            for &node_id in &seed.completed_nodes {
                if let Some(mut op) = op_states.get_mut(&node_id) {
                    op.status = OpStatus::Completed;
                }
                precompleted += 1;
            }
        }
        let initial_remaining = dag.nodes.len().saturating_sub(precompleted);

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
        // Generous separate pool for long-waiting ops (PAUSE/RESUME/recv) so they
        // never compete with compute/LLM permits.
        let blocking_concurrency = ConcurrencyControl::new(cfg.blocking_inflight);

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
            rearm_turns: Arc::new(DashMap::new()),

            blocking_concurrency,
            cancellation_token: CancellationToken::new(),
            work_stealing,
            queue: Arc::clone(&queue),

            concurrency,
            llm_concurrency,

            executed: Arc::new(AtomicUsize::new(0)),
            failed: Arc::new(AtomicUsize::new(0)),
            remaining: Arc::new(AtomicUsize::new(initial_remaining)),
            parked: Arc::new(AtomicUsize::new(0)),
            admission_id: None,
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

        // Initialize readiness tracking and seed ready nodes. On a partial replay
        // the pre-completed upstream nodes are skipped so they are never enqueued.
        let ready_nodes = state.ready_set.initialize_with_skip(
            &dag.nodes,
            &state.tokens,
            &state.priorities,
            &state.op_states,
            &state.queue,
            replay_seed.map(|seed| &seed.completed_nodes),
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

    /// Record one more delivered turn for an in-graph session loop and return the
    /// running count. The park re-arm uses this to stop re-arming once the recv
    /// node's `max_iterations` cap is reached (CONV-1: bound the loop).
    pub(crate) fn next_rearm_turn(&self, session_id: &str) -> u64 {
        let mut entry = self.rearm_turns.entry(session_id.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    #[inline]
    pub fn mark_done(&self) {
        tracing::debug!("mark_done called, setting remaining to 0");
        self.remaining.store(0, Ordering::SeqCst);
        self.concurrency.cancel();
        self.llm_concurrency.cancel();
        self.blocking_concurrency.cancel();
        self.cancellation_token.cancel();
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
        self.concurrency.is_cancelled() || self.cancellation_token.is_cancelled()
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
    /// Helps the watchdog distinguish true deadlocks (nothing running, nothing
    /// becoming ready) from long-running operations such as LLM calls.
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

    /// Resume a PARKED node: make its output token(s) ready with `value`, propagate
    /// readiness to consumers, and perform the SINGLE compensating completion the
    /// node skipped when it parked (it did neither `publish_outputs` nor
    /// `finish_one`). This is the cross-frame twin of [`Self::resolve_promise`] +
    /// `finish_one`; together a park→wake performs exactly one completion, so the
    /// `remaining` count is invariant vs a normal node finishing.
    pub fn wake_parked_node(&self, outputs: &[TokenId], value: Value) {
        if self.is_cancelled() {
            self.set_first_error(RuntimeError::SchedulerCancelled);
            self.mark_done();
            self.exit_parked();
            self.record_progress();
            return;
        }
        for &token_id in outputs {
            match self.tokens.get_mut(&token_id) {
                Some(token) if token.ready => continue, // already produced; idempotent
                Some(mut token) => {
                    token.ready = true;
                    token.value = Some(value.clone());
                }
                None => {
                    let mut ts = TokenState::new();
                    ts.ready = true;
                    ts.value = Some(value.clone());
                    self.tokens.insert(token_id, ts);
                }
            }
            if let Ok(ready_nodes) = self.ready_set.on_token_ready(
                token_id,
                &self.tokens,
                &self.priorities,
                &self.op_states,
                &self.queue,
            ) {
                self.emit_node_ready_batch(&ready_nodes);
                if !ready_nodes.is_empty() {
                    self.work_notify.notify_waiters();
                }
            }
        }
        // The parked node completes now — the one compensating decrement.
        let prev = self.remaining.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            self.notify_done.notify_waiters();
            self.work_notify.notify_waiters();
        }
        // Clear the parked count (and reacquire admission on the 1->0 edge).
        self.exit_parked();
        self.record_progress();
    }

    /// Number of nodes currently parked on an external event.
    pub fn parked_count(&self) -> usize {
        self.parked.load(Ordering::SeqCst)
    }

    /// Record that a node has parked. On the 0->1 transition (the execution
    /// enters a waiting state) release its cross-execution admission slot so the
    /// capacity it isn't using can admit other work.
    pub fn enter_parked(&self) {
        if self.parked.fetch_add(1, Ordering::SeqCst) == 0
            && let Some(id) = &self.admission_id
        {
            crate::scheduler::admission_registry::on_park(id);
        }
    }

    /// Record that a parked node has resumed. On the 1->0 transition (no nodes
    /// remain parked) best-effort reacquire the admission slot.
    fn exit_parked(&self) {
        if self.parked.fetch_sub(1, Ordering::SeqCst) == 1
            && let Some(id) = &self.admission_id
        {
            crate::scheduler::admission_registry::on_unpark(id);
        }
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

        // Token 10 (node 1 -> node 2) should exist.
        assert!(state.tokens.contains_key(&10));
        // Token 20 (produced by node 2, no consumer) should exist
        assert!(state.tokens.contains_key(&20));
        // Workers should be sized to the configured concurrency.
        assert_eq!(workers.len(), 2);
    }

    // ── Partial replay (rerun-from-node) seeding ──────────────────────────

    /// Linear 3-node chain: 1 --t10--> 2 --t20--> 3 (--t30 exit).
    fn three_node_chain() -> ExecutionDag {
        let mut dag = ExecutionDag::new();
        dag.add_node(make_node(1, vec![], vec![10])).unwrap();
        dag.add_node(make_node(2, vec![10], vec![20])).unwrap();
        dag.add_node(make_node(3, vec![20], vec![30])).unwrap();
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .unwrap();
        dag.add_edge(Edge::new(2, 3, 20, DependencyType::Data))
            .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag
    }

    /// Drain every queued node id (across all priority levels) for assertions.
    fn drain_queue(state: &SchedulerState) -> std::collections::HashSet<NodeId> {
        let mut queued = std::collections::HashSet::new();
        for injector in state.queue.injectors() {
            loop {
                match injector.steal() {
                    crossbeam_deque::Steal::Success(node_id) => {
                        queued.insert(node_id);
                    }
                    crossbeam_deque::Steal::Empty => break,
                    crossbeam_deque::Steal::Retry => {}
                }
            }
        }
        queued
    }

    /// THE load-bearing partial-replay invariant: when state is built with a
    /// replay seed rooted at the middle node, the upstream node is pre-completed
    /// and NEVER enqueued (so a worker can never re-invoke it), its boundary
    /// output token is seeded with the prior value, `remaining` counts only the
    /// replayed nodes, and only `from_node` is initially ready/enqueued.
    #[test]
    fn replay_seed_precompletes_upstream_and_only_enqueues_from_node() {
        use crate::scheduler::replay::ReplaySeed;

        let dag = three_node_chain();
        let mut prior = HashMap::new();
        // Node 1's prior output (token 10) feeds the replay boundary.
        prior.insert(10u64, Value::String("prior-output-of-node-1".into()));
        let seed = ReplaySeed::compute(&dag, 2, &prior).expect("seed for known node");

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _workers) = SchedulerState::new_with_replay(
            dag,
            test_config().with_llm_inflight(2),
            metrics,
            Instant::now(),
            vec![],
            ExecutionHookContext::default(),
            Some(&seed),
        )
        .unwrap();

        // Upstream node 1 is pre-completed, NOT ready, and never enqueued.
        assert_eq!(
            state.op_states.get(&1).unwrap().status,
            OpStatus::Completed,
            "upstream node must be marked completed"
        );
        let queued = drain_queue(&state);
        assert!(
            !queued.contains(&1),
            "upstream node must NOT be enqueued (so its handler is never re-invoked)"
        );

        // from_node (2) is ready/enqueued; node 3 waits on node 2's output.
        assert!(queued.contains(&2), "from_node must be enqueued to replay");
        assert!(!queued.contains(&3), "descendant waits on from_node output");
        assert_eq!(state.op_states.get(&2).unwrap().status, OpStatus::Ready);

        // The boundary token is seeded ready with the prior value.
        let t10 = state.tokens.get(&10).unwrap();
        assert!(t10.ready, "boundary token must be pre-seeded ready");
        assert_eq!(
            t10.value,
            Some(Value::String("prior-output-of-node-1".into()))
        );

        // remaining counts only the replayed nodes (2 and 3), not the completed one.
        assert_eq!(
            state.remaining.load(Ordering::SeqCst),
            2,
            "remaining must exclude pre-completed upstream nodes"
        );
    }

    /// Replaying from the entry node is equivalent to a full run: nothing is
    /// pre-completed, all nodes count toward `remaining`.
    #[test]
    fn replay_seed_from_entry_node_is_full_run() {
        use crate::scheduler::replay::ReplaySeed;

        let dag = three_node_chain();
        let seed = ReplaySeed::compute(&dag, 1, &HashMap::new()).unwrap();
        assert!(seed.completed_nodes.is_empty());

        let metrics = Arc::new(MetricsCollector::new());
        let (state, _workers) = SchedulerState::new_with_replay(
            dag,
            test_config().with_llm_inflight(2),
            metrics,
            Instant::now(),
            vec![],
            ExecutionHookContext::default(),
            Some(&seed),
        )
        .unwrap();

        assert_eq!(state.remaining.load(Ordering::SeqCst), 3);
        // Only the entry node is initially ready.
        let queued = drain_queue(&state);
        assert_eq!(queued, std::collections::HashSet::from([1]));
    }

    // ── Park/wake (event-driven continuation) ─────────────────────────────

    fn new_state(dag: ExecutionDag) -> SchedulerState {
        let metrics = Arc::new(MetricsCollector::new());
        SchedulerState::new(
            dag,
            test_config().with_llm_inflight(2),
            metrics,
            Instant::now(),
            vec![],
        )
        .unwrap()
        .0
    }

    /// THE mandatory remaining-count invariance gate (the #1 hazard): a node that
    /// PARKS (no finish_one) and is later WOKEN decrements `remaining` exactly
    /// once — identical to a normal completion. A miscount here hangs or
    /// prematurely finishes a DAG.
    #[test]
    fn wake_parked_node_preserves_remaining_invariant() {
        let state = new_state(two_node_dag());
        assert_eq!(state.remaining.load(Ordering::SeqCst), 2);

        // Park node 1: the worker increments `parked` and SKIPS finish_one.
        state.parked.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            state.remaining.load(Ordering::SeqCst),
            2,
            "park must not decrement remaining"
        );
        assert_eq!(state.parked_count(), 1);

        // Wake delivers node 1's output token (10) — the ONE compensating completion.
        state.wake_parked_node(&[10], Value::String("resumed".into()));

        assert_eq!(
            state.remaining.load(Ordering::SeqCst),
            1,
            "park->wake decrements remaining EXACTLY once (invariant)"
        );
        assert_eq!(state.parked_count(), 0, "parked counter cleared on wake");
        let t = state.tokens.get(&10).expect("token 10 exists");
        assert!(t.ready, "woken node's output token is ready");
        assert_eq!(t.value.clone(), Some(Value::String("resumed".into())));
    }

    /// A parked recv, on wake, splices a fresh turn sub-DAG
    /// (consuming the user message) AND re-arms a fresh recv — proving the native
    /// conversation loop composes from `splice_turn_and_rearm` + `wake_parked_node`
    /// with the park/wake invariants intact (no node re-execution; remaining and
    /// parked counts exact). No new scheduler primitive is required.
    #[test]
    fn recv_wake_splice_rearm_keystone() {
        // Anchor: a single recv node (id 1) whose output token (10) carries the
        // delivered user message.
        let mut dag = ExecutionDag::new();
        dag.add_node(make_node(1, vec![], vec![10])).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        let state = new_state(dag);
        assert_eq!(state.remaining.load(Ordering::SeqCst), 1);

        // The worker picks up the entry recv; it PARKS (dequeued, parked, no
        // finish_one) awaiting the next user message.
        let initial = drain_queue(&state);
        assert!(initial.contains(&1), "recv anchor is initially ready");
        state.parked.fetch_add(1, Ordering::SeqCst);
        assert_eq!(state.parked_count(), 1);

        // Build the fresh turn sub-DAG (turn body consumes the message via inner
        // token 1, produces reply token 2) and the fresh recv (re-arm; no inputs).
        let mut turn_dag = ExecutionDag::new();
        turn_dag.add_node(make_node(1, vec![1], vec![2])).unwrap();
        let fresh_recv = make_node(2, vec![], vec![3]);

        // Re-arm on wake: splice the turn body (input connected to message token
        // 10) + the fresh recv into the live execution.
        state
            .splice_turn_and_rearm(
                10,
                1,
                std::collections::HashMap::new(),
                turn_dag,
                fresh_recv,
            )
            .expect("re-arm splice succeeds");
        assert_eq!(
            state.remaining.load(Ordering::SeqCst),
            3,
            "splice adds turn body + fresh recv to remaining"
        );

        // Deliver the user message: wake the parked recv anchor.
        state.wake_parked_node(&[10], Value::String("hello turn".into()));

        // Invariants: the anchor's ONE compensating completion fires (3->2), the
        // parked counter clears, and the message token is delivered.
        assert_eq!(
            state.remaining.load(Ordering::SeqCst),
            2,
            "wake decrements exactly once; turn + fresh recv remain"
        );
        assert_eq!(state.parked_count(), 0, "parked cleared on wake");
        let msg_token = state.tokens.get(&10).expect("message token exists");
        assert!(msg_token.ready);
        assert_eq!(
            msg_token.value.clone(),
            Some(Value::String("hello turn".into()))
        );

        // The turn body (consuming the message) and the fresh recv (re-arm) are
        // both scheduled; the completed anchor (node 1) is NOT re-enqueued
        // (no prior-turn recompute). Splice offsets ids by max+1 (=2):
        // turn 1->3, fresh recv 2->4.
        let queued = drain_queue(&state);
        assert!(
            !queued.contains(&1),
            "completed recv anchor must not re-run"
        );
        assert!(
            queued.contains(&3),
            "turn body enqueued after message delivered"
        );
        assert!(queued.contains(&4), "fresh recv re-armed (ready)");
    }

    /// Per-turn session state (history/summary) carries across re-arms via
    /// spliced token connections — the new turn body consumes a live state token
    /// produced before the re-arm, without re-running anything.
    #[test]
    fn recv_rearm_carries_session_state() {
        let mut dag = ExecutionDag::new();
        dag.add_node(make_node(1, vec![], vec![10])).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        let state = new_state(dag);
        let _ = drain_queue(&state);
        state.parked.fetch_add(1, Ordering::SeqCst);

        // A live "running summary" token from the prior turn, already ready.
        {
            let mut ts = TokenState::new();
            ts.ready = true;
            ts.value = Some(Value::String("prior summary".into()));
            state.tokens.insert(11, ts);
        }

        // Turn body consumes BOTH the message (inner token 1 → live 10) and the
        // carried summary (inner token 4 → live 11).
        let mut turn_dag = ExecutionDag::new();
        turn_dag
            .add_node(make_node(1, vec![1, 4], vec![2]))
            .unwrap();
        let fresh_recv = make_node(2, vec![], vec![3]);

        let mut carry = std::collections::HashMap::new();
        carry.insert(4u64, 11u64); // inner summary input ← live summary token

        state
            .splice_turn_and_rearm(10, 1, carry, turn_dag, fresh_recv)
            .expect("re-arm with carried state splices");

        // The carried summary token (11) is ready, so the turn body waits only on
        // the message; wake delivers it and the turn becomes schedulable.
        state.wake_parked_node(&[10], Value::String("turn 2 message".into()));

        let queued = drain_queue(&state);
        assert!(
            queued.contains(&3),
            "turn body schedulable with carried state + message"
        );
        // The turn node now consumes the live summary token 11 (carried, not re-run).
        let turn = state.nodes.get(&3).expect("spliced turn node");
        assert!(
            turn.input_tokens.contains(&11),
            "carried session-state token wired into the new turn body"
        );
        assert!(
            state.tokens.get(&11).unwrap().ready,
            "carried state stays ready"
        );
    }

    /// End-to-end-ish: a session-loop recv, woken via the PRODUCTION
    /// re-arming `ParkWaker` (not a direct primitive call), splices a turn
    /// flow-call (binding the message) + a fresh recv — proving the emitted
    /// entry flow drives `splice_turn_and_rearm` through the real wake path.
    #[test]
    fn recv_wake_drives_rearm_via_production_waker() {
        use crate::scheduler::park_registry::{self, ParkWaker, RearmSpec};
        use apxm_core::types::operations::AISOperationType;

        // The recv anchor (id 1, output token 10) carries the loop attrs the
        // ConversationalAgent builder stamps.
        let mut recv = make_node(1, vec![], vec![10]);
        recv.op_type = AISOperationType::Autonomous;
        for (k, v) in [
            ("mode", "recv"),
            ("recv_once", "false"),
            ("turn_agent", "conversation"),
            ("turn_flow", "turn"),
            ("turn_param", "user_message"),
        ] {
            recv.attributes
                .insert(k.to_string(), Value::String(v.to_string()));
        }
        let mut dag = ExecutionDag::new();
        dag.add_node(recv.clone()).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        let state = Arc::new(new_state(dag));
        let _ = drain_queue(&state);
        state.parked.fetch_add(1, Ordering::SeqCst);

        // Register the re-arming waker exactly as the worker park path does.
        let spec = RearmSpec {
            recv_node: Arc::new(recv),
            turn_agent: "conversation".to_string(),
            turn_flow: "turn".to_string(),
            turn_param: "user_message".to_string(),
            session_id: "test".to_string(),
            max_turns: 100,
        };
        let key = "session_recv:rearm-prod-test-1";
        park_registry::register(
            key.to_string(),
            ParkWaker::new_rearming(Arc::clone(&state), vec![10], spec),
        );

        // Wake with the user message (the turn-input endpoint's action).
        let woken = park_registry::wake(key, Value::String("hello turn".into()));
        assert_eq!(woken, 1);

        // Message delivered to the recv output token.
        assert_eq!(
            state.tokens.get(&10).unwrap().value.clone(),
            Some(Value::String("hello turn".into()))
        );

        // A FLOW_CALL (turn dispatch) + a fresh recv (re-arm) were spliced live.
        let mut has_flow_call = false;
        let mut has_fresh_recv = false;
        for entry in state.nodes.iter() {
            let n = entry.value();
            if n.op_type == AISOperationType::FlowCall {
                has_flow_call = true;
            }
            if n.op_type == AISOperationType::Autonomous && n.id != 1 {
                has_fresh_recv = true;
            }
        }
        assert!(has_flow_call, "turn flow-call spliced on wake");
        assert!(has_fresh_recv, "fresh recv re-armed on wake");
    }

    /// Robustness lock-in (Audit minor 1): the production re-arming waker must
    /// splice BEFORE waking, so a SOLE loop recv (remaining == 1) never drives
    /// `remaining` to 0 — which would fire `notify_done` and signal completion.
    /// This test registers a `notify_done` waiter and asserts it is NOT fired by
    /// the wake; it FAILS under the prior wake-then-splice ordering.
    #[tokio::test]
    async fn rearm_splices_before_wake_no_zero_remaining_window() {
        use crate::scheduler::park_registry::{self, ParkWaker, RearmSpec};
        use apxm_core::types::operations::AISOperationType;
        use futures::poll;

        let mut recv = make_node(1, vec![], vec![10]);
        recv.op_type = AISOperationType::Autonomous;
        for (k, v) in [
            ("mode", "recv"),
            ("recv_once", "false"),
            ("turn_agent", "conversation"),
            ("turn_flow", "turn"),
            ("turn_param", "user_message"),
        ] {
            recv.attributes
                .insert(k.to_string(), Value::String(v.to_string()));
        }
        let mut dag = ExecutionDag::new();
        dag.add_node(recv.clone()).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        let state = Arc::new(new_state(dag));
        let _ = drain_queue(&state);
        state.parked.fetch_add(1, Ordering::SeqCst);
        assert_eq!(state.remaining.load(Ordering::SeqCst), 1);

        // Register a waiter on notify_done (fires only on the 1->0 remaining edge).
        let notified = state.notify_done.notified();
        futures::pin_mut!(notified);
        assert!(
            poll!(&mut notified).is_pending(),
            "waiter registers pending"
        );

        let spec = RearmSpec {
            recv_node: Arc::new(recv),
            turn_agent: "conversation".to_string(),
            turn_flow: "turn".to_string(),
            turn_param: "user_message".to_string(),
            session_id: "test".to_string(),
            max_turns: 100,
        };
        let key = "session_recv:zero-window-test-1";
        park_registry::register(
            key.to_string(),
            ParkWaker::new_rearming(Arc::clone(&state), vec![10], spec),
        );

        park_registry::wake(key, Value::String("hi".into()));

        // Splice-then-wake never reaches remaining == 0, so notify_done is NOT
        // fired (this assertion fails under wake-then-splice), and the loop lives.
        assert!(
            poll!(&mut notified).is_pending(),
            "splice-then-wake must not signal done (no zero-remaining window)"
        );
        assert_eq!(
            state.remaining.load(Ordering::SeqCst),
            2,
            "flow_call + fresh recv remain after the recv completes"
        );
    }

    #[test]
    fn park_registry_wake_resumes_parked_node() {
        use crate::scheduler::park_registry;
        let state = Arc::new(new_state(two_node_dag()));
        state.parked.fetch_add(1, Ordering::SeqCst);
        let key = "cp-park-resume-unique-1";
        park_registry::register(
            key.to_string(),
            park_registry::ParkWaker::new(Arc::clone(&state), vec![10]),
        );
        let woken = park_registry::wake(key, Value::String("hi".into()));
        assert_eq!(woken, 1, "one parked node woken");
        assert_eq!(state.remaining.load(Ordering::SeqCst), 1);
        assert!(state.tokens.get(&10).unwrap().ready);
    }

    #[test]
    fn park_registry_wake_before_register_is_not_lost() {
        use crate::scheduler::park_registry;
        let key = "cp-pre-resolved-unique-2";
        // Wake arrives BEFORE any waiter registers (the race).
        park_registry::wake(key, Value::String("early".into()));
        let state = Arc::new(new_state(two_node_dag()));
        state.parked.fetch_add(1, Ordering::SeqCst);
        // Registering now must fire immediately from the stored Resolved value.
        park_registry::register(
            key.to_string(),
            park_registry::ParkWaker::new(Arc::clone(&state), vec![10]),
        );
        assert!(
            state.tokens.get(&10).unwrap().ready,
            "pre-resolved wake delivered on register (no lost wakeup)"
        );
        assert_eq!(state.remaining.load(Ordering::SeqCst), 1);
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
    fn test_new_substitutes_dotted_json_runtime_parameters_in_tool_params() {
        let mut node = make_node(1, vec![20], vec![10]);
        node.op_type = AISOperationType::InvTool;
        node.attributes.insert(
            apxm_core::constants::graph::attrs::PARAMS_JSON.to_string(),
            Value::String(r#"{"chat_id":"{data.event.subject}","text":"{respond}"}"#.to_string()),
        );

        let mut dag = ExecutionDag::new();
        dag.add_node(node).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag.metadata = DagMetadata {
            name: Some("parameterized-tool".to_string()),
            is_entry: true,
            parameters: vec![FlowParameter {
                name: "data".to_string(),
                type_name: "json".to_string(),
            }],
        };

        let metrics = Arc::new(MetricsCollector::new());
        let data = Value::try_from(serde_json::json!({
            "event": { "subject": "chat-\"42\"\nnext", "payload": { "text": "hi" } }
        }))
        .unwrap();
        let (state, _) =
            SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![data]).unwrap();

        let stored = state.nodes.get(&1).expect("node should be stored");
        let params_json = stored
            .attributes
            .get(apxm_core::constants::graph::attrs::PARAMS_JSON)
            .and_then(Value::as_str)
            .expect("params_json attribute");
        let parsed: serde_json::Value =
            serde_json::from_str(params_json).expect("substituted params_json stays valid JSON");
        assert_eq!(parsed["chat_id"], serde_json::json!("chat-\"42\"\nnext"));
        assert_eq!(parsed["text"], serde_json::json!("{respond}"));
    }

    #[test]
    fn test_new_rejects_unresolved_dotted_json_runtime_parameter() {
        let mut node = make_node(1, vec![], vec![10]);
        node.op_type = AISOperationType::InvTool;
        node.attributes.insert(
            apxm_core::constants::graph::attrs::PARAMS_JSON.to_string(),
            Value::String(r#"{"chat_id":"{data.event.missing}"}"#.to_string()),
        );

        let mut dag = ExecutionDag::new();
        dag.add_node(node).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag.metadata = DagMetadata {
            name: Some("bad-parameterized-tool".to_string()),
            is_entry: true,
            parameters: vec![FlowParameter {
                name: "data".to_string(),
                type_name: "json".to_string(),
            }],
        };

        let metrics = Arc::new(MetricsCollector::new());
        let data = Value::try_from(serde_json::json!({
            "event": { "subject": "chat-42" }
        }))
        .unwrap();
        let err = match SchedulerState::new(dag, test_config(), metrics, Instant::now(), vec![data])
        {
            Ok(_) => panic!("missing dotted parameter path should fail closed"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("{data.event.missing}"));
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
