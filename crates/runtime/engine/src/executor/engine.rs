//! Executor engine - Main coordinator for DAG execution

use super::{ExecutionContext, ExecutionHookContext, Result, dispatcher::OperationDispatcher};
use crate::graph_lifecycle::{graph_dispatch_ir_from_dag, graph_metadata_from_dispatch_ir};
use crate::scheduler::DataflowScheduler;
use apxm_core::error::RuntimeError;
use apxm_core::types::{
    GraphStatusSnapshot,
    execution::{ExecutionDag, ExecutionStats, Node, NodeStatus, OpStatus},
    operations::AISOperationType,
    values::Value,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

/// Executor engine coordinates DAG execution.
pub struct ExecutorEngine {
    context: ExecutionContext,
}

impl ExecutorEngine {
    /// Create a new executor engine with the given context
    pub fn new(context: ExecutionContext) -> Self {
        Self { context }
    }

    /// Execute an operation using the provided context
    pub async fn execute_with_context(
        &self,
        node: &Node,
        inputs: Vec<Value>,
        ctx: &ExecutionContext,
    ) -> Result<OperationOutcome> {
        let value = OperationDispatcher::dispatch(ctx, node, inputs).await?;
        Ok(OperationOutcome { value })
    }

    /// Execute a complete DAG using the dataflow scheduler for parallel execution.
    ///
    /// Delegates to [`DataflowScheduler`] which provides automatic parallelism
    /// via token-based dataflow execution, work stealing, and backpressure.
    /// Falls back to sequential execution only when the scheduler is unavailable.
    pub async fn execute_dag(&self, dag: ExecutionDag) -> Result<ExecutionResult> {
        // Propagate scope_id to the event emitter for session isolation.
        if let Some(emitter) = &self.context.event_emitter {
            emitter.set_current_scope_id(self.context.current_scope_id.clone());
        }

        // Layer 2 — outermost executor entry: bracket the run with
        // turn_started / turn_complete (or turn_aborted on the error
        // path). This is the boundary above any SPAWN_AGENT scope.
        let turn_start = std::time::Instant::now();
        if let Some(emitter) = &self.context.event_emitter {
            emitter.emit_turn_started(&self.context.execution_id, None, None);
        }

        tracing::info!(
            execution_id = %self.context.execution_id,
            nodes = dag.nodes.len(),
            "Starting DAG execution"
        );

        // Register graph metadata with graph-aware backends.
        let graph_id = self.context.graph_id.clone();
        let dispatch_ir = graph_dispatch_ir_from_dag(&graph_id, &self.context.execution_id, &dag);
        self.context.set_dispatch_ir_v1(dispatch_ir.clone());
        self.register_graph_metadata(&dispatch_ir).await;

        // Detailed metrics: spawn pin-peak polling for this graph_id.
        // `start_pin_polling` returns `None` when no graph-aware backends are
        // registered, so we don't wake a no-op poll loop.
        let pin_poll_handle = self
            .context
            .metrics_level
            .pin_poll_interval()
            .and_then(|interval| {
                self.context
                    .llm_registry
                    .start_pin_polling(graph_id.clone(), interval)
            });

        let mut result = self.execute_dag_inner(dag).await;

        // Stop polling before pre_release so the final fold sees the full peak.
        if let Some(handle) = pin_poll_handle {
            handle.abort();
        }

        // Capture graph status from graph-aware backends before releasing pins.
        let graph_status_snapshots = self
            .context
            .llm_registry
            .pre_release_status_all(&graph_id)
            .await;

        // Release graph from backends (both success and error paths)
        self.context.llm_registry.release_graph_all(&graph_id).await;

        if let Ok(ref mut exec_result) = result {
            exec_result.graph_status_snapshots = graph_status_snapshots;
        }

        // Layer 2 safety net — drain any agent scopes still on the stack
        // at DAG exit. The handoff/communicate handlers SHOULD pop their
        // child's scope on control return, but a panic/early-error path
        // could leave scopes behind. Emit `subagent_done` (success) or
        // `subagent_failed` (error) for each leftover so observers see a
        // balanced begin/end pair per scope. This is additive: when the
        // clean-exit pop did its job, the stack is already empty.
        if let Some(emitter) = &self.context.event_emitter {
            while let Some(scope) = self.context.agent_scope_stack.pop() {
                match &result {
                    Ok(_) => emitter.emit_subagent_done(&scope.agent_code, 0, 0, 0, None),
                    Err(err) => emitter.emit_subagent_failed(
                        &scope.agent_code,
                        super::agent_scope::LAYER2_ERROR_CLASS_RUNTIME,
                        &err.to_string(),
                    ),
                }
            }
        }

        // Layer 2 — emit the matching turn terminal event. Success path
        // is `turn_complete`; error path is `turn_aborted` with a
        // safe-to-surface error message.
        if let Some(emitter) = &self.context.event_emitter {
            let duration_ms = turn_start.elapsed().as_millis() as u64;
            match &result {
                Ok(exec_result) => {
                    let had_answer = !exec_result.results.is_empty();
                    emitter.emit_turn_complete(&self.context.execution_id, duration_ms, had_answer);
                }
                Err(err) => {
                    emitter.emit_turn_aborted(
                        &self.context.execution_id,
                        duration_ms,
                        "error",
                        Some(&err.to_string()),
                    );
                }
            }
        }

        result
    }

    /// Inner DAG execution logic. Tries the parallel dataflow scheduler
    /// first; on error, either propagates it (default) or falls back to
    /// sequential execution, gated by
    /// `SchedulerConfig::allow_sequential_fallback` — see that field's doc
    /// comment. The fallback is off by default so a real scheduler bug fails
    /// loud instead of silently bimodalizing
    /// latency, but still available as an explicit, logged, alertable opt-in
    /// for a deployment that depends on it.
    async fn execute_dag_inner(&self, dag: ExecutionDag) -> Result<ExecutionResult> {
        if dag.nodes.len() > 1 {
            match self.execute_dag_parallel(dag.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if !self.context.scheduler_config.allow_sequential_fallback {
                        tracing::error!(
                            execution_id = %self.context.execution_id,
                            error = %e,
                            "Dataflow scheduler failed; sequential fallback is disabled \
                             (SchedulerConfig::allow_sequential_fallback = false) — propagating error"
                        );
                        return Err(e);
                    }
                    tracing::warn!(
                        execution_id = %self.context.execution_id,
                        error = %e,
                        "Dataflow scheduler failed, falling back to sequential execution \
                         (SchedulerConfig::allow_sequential_fallback = true)"
                    );
                    if let Some(emitter) = &self.context.event_emitter {
                        emitter.emit_warning(
                            "scheduler_fallback_triggered",
                            &format!(
                                "dataflow scheduler failed for execution {}, falling back to \
                                 sequential execution: {e}",
                                self.context.execution_id
                            ),
                        );
                    }
                }
            }
        }

        self.execute_dag_sequential(dag).await
    }

    /// Build and register graph metadata with all backends.
    async fn register_graph_metadata(&self, dispatch_ir: &crate::dispatch::v1::DispatchIrV1) {
        let metadata = graph_metadata_from_dispatch_ir(dispatch_ir);

        self.context.llm_registry.register_graph_all(metadata).await;
    }

    /// Execute a DAG using the dataflow scheduler for automatic parallelism.
    async fn execute_dag_parallel(&self, dag: ExecutionDag) -> Result<ExecutionResult> {
        let config = self.context.scheduler_config.clone();
        let scheduler = DataflowScheduler::new(config);

        let executor = Arc::new(ExecutorEngine::new(self.context.clone()));

        // Partial replay is permitted only when complete boundary values are
        // present and no completed authority/effect operation is being skipped.
        let replay_seed =
            crate::scheduler::ReplaySeed::from_metadata_checked(&self.context.metadata, &dag)
                .map_err(|error| RuntimeError::Scheduler {
                    message: format!("partial replay rejected: {error}"),
                })?;

        let (results, stats, _scheduler_metrics, _, _) = scheduler
            .execute_with_hooks_and_seed(
                dag,
                executor,
                self.context.clone(),
                vec![],
                ExecutionHookContext::default(),
                replay_seed.as_ref(),
            )
            .await?;

        Ok(ExecutionResult {
            results,
            stats,
            token_snapshot: self.context.token_accountant.snapshot(),
            graph_status_snapshots: vec![],
        })
    }

    /// Sequential fallback executor for DAGs.
    ///
    /// Processes a normalized typed-edge order, suitable for single-node DAGs,
    /// testing, and as a fallback when the dataflow scheduler encounters an
    /// error. Replay seeds use the same checked boundary contract as the
    /// parallel scheduler; the fallback must not rerun completed upstream work
    /// or discard effect/control ordering.
    async fn execute_dag_sequential(&self, dag: ExecutionDag) -> Result<ExecutionResult> {
        let start_time = std::time::Instant::now();
        let node_statuses = Arc::new(RwLock::new(HashMap::<u64, NodeStatus>::new()));
        let replay_seed =
            crate::scheduler::ReplaySeed::from_metadata_checked(&self.context.metadata, &dag)
                .map_err(|error| RuntimeError::Scheduler {
                    message: format!("partial replay rejected: {error}"),
                })?;
        let execution_order =
            crate::scheduler::ReplaySeed::normalized_sequential_order(&dag, replay_seed.as_ref())
                .map_err(|error| RuntimeError::Scheduler {
                message: format!("sequential fallback replay normalization rejected: {error}"),
            })?;
        let seeded_results = replay_seed
            .as_ref()
            .map(|seed| seed.seed_tokens.clone())
            .unwrap_or_default();
        let node_results = Arc::new(RwLock::new(seeded_results));
        let nodes_by_id: HashMap<_, _> = dag.nodes.iter().map(|node| (node.id, node)).collect();

        for node_id in execution_order {
            let node = nodes_by_id
                .get(&node_id)
                .copied()
                .expect("normalized execution order only contains declared nodes");
            let node_start = std::time::Instant::now();
            let ready_at_ms = start_time.elapsed().as_millis();
            let priority = sequential_priority_label(node.metadata.priority);

            // Mark as running
            {
                let mut statuses = node_statuses.write().await;
                statuses.insert(
                    node.id,
                    NodeStatus {
                        node_id: node.id,
                        status: OpStatus::Running,
                        retries: 0,
                        last_error: None,
                        ready_at_ms: Some(ready_at_ms),
                        started_at_ms: Some(ready_at_ms),
                        finished_at_ms: None,
                        duration_ms: None,
                        queue_wait_ms: Some(0),
                        priority: Some(priority.clone()),
                        input_tokens: None,
                        output_tokens: None,
                    },
                );
            }

            // Gather inputs from dependencies
            let mut inputs = Vec::new();
            for input_token_id in &node.input_tokens {
                let results = node_results.read().await;
                if let Some(value) = results.get(input_token_id) {
                    inputs.push(value.clone());
                }
            }

            // Execute operation
            let result = OperationDispatcher::dispatch(&self.context, node, inputs).await;

            let node_end = node_start.elapsed();
            let finished_at_ms = start_time.elapsed().as_millis();

            // Store result and update status
            match result {
                Ok(value) => {
                    // Store result for output tokens
                    {
                        let mut results = node_results.write().await;
                        for output_token_id in &node.output_tokens {
                            results.insert(*output_token_id, value.clone());
                        }
                    }
                    if let Some(emitter) = &self.context.event_emitter {
                        emitter.emit_node_output_with_name(
                            node.id,
                            node.metadata.name.as_deref(),
                            &value,
                        );
                        // Emit GRAPH_EDGE for every static
                        // downstream consumer that just became eligible.
                        // The edge kind is derived from the consumer's op
                        // type so observers can color the graph without
                        // re-inspecting the DAG.
                        for output_token in &node.output_tokens {
                            for downstream in dag
                                .nodes
                                .iter()
                                .filter(|n| n.input_tokens.iter().any(|t| t == output_token))
                            {
                                let kind = graph_edge_kind_for_consumer(downstream.op_type);
                                emitter.emit_graph_edge(node.id, downstream.id, kind);
                            }
                        }
                    }

                    // Mark as completed
                    {
                        let mut statuses = node_statuses.write().await;
                        statuses.insert(
                            node.id,
                            NodeStatus {
                                node_id: node.id,
                                status: OpStatus::Completed,
                                retries: 0,
                                last_error: None,
                                ready_at_ms: Some(ready_at_ms),
                                started_at_ms: Some(ready_at_ms),
                                finished_at_ms: Some(finished_at_ms),
                                duration_ms: Some(node_end.as_millis()),
                                queue_wait_ms: Some(0),
                                priority: Some(priority.clone()),
                                input_tokens: None,
                                output_tokens: None,
                            },
                        );
                    }
                }
                Err(e) => {
                    // Mark as failed
                    {
                        let mut statuses = node_statuses.write().await;
                        statuses.insert(
                            node.id,
                            NodeStatus {
                                node_id: node.id,
                                status: OpStatus::Failed,
                                retries: 0,
                                last_error: Some(e.to_string()),
                                ready_at_ms: Some(ready_at_ms),
                                started_at_ms: Some(ready_at_ms),
                                finished_at_ms: Some(finished_at_ms),
                                duration_ms: Some(node_end.as_millis()),
                                queue_wait_ms: Some(0),
                                priority: Some(priority.clone()),
                                input_tokens: None,
                                output_tokens: None,
                            },
                        );
                    }

                    tracing::error!(
                        execution_id = %self.context.execution_id,
                        node_id = node.id,
                        task_source_id = ?node.metadata.task_source_id,
                        error = %e,
                        "Node execution failed"
                    );

                    return Err(e);
                }
            }
        }

        // Collect final results from exit nodes
        let mut final_results = HashMap::new();
        {
            let results = node_results.read().await;
            for exit_node_id in &dag.exit_nodes {
                // Find the output token for this exit node
                if let Some(node) = dag.nodes.iter().find(|n| n.id == *exit_node_id) {
                    for output_token_id in &node.output_tokens {
                        if let Some(value) = results.get(output_token_id) {
                            final_results.insert(*output_token_id, value.clone());
                        }
                    }
                }
            }
        }

        // Build execution stats
        let statuses = node_statuses.read().await;
        let mut stats = ExecutionStats {
            executed_nodes: statuses.len(),
            failed_nodes: statuses
                .values()
                .filter(|s| matches!(s.status, OpStatus::Failed))
                .count(),
            duration_ms: start_time.elapsed().as_millis(),
            node_statuses: statuses.values().cloned().collect(),
            observed_graph: None,
        };
        stats.attach_observed_graph_metrics(&dag);

        tracing::info!(
            execution_id = %self.context.execution_id,
            duration_ms = stats.duration_ms,
            executed = stats.executed_nodes,
            failed = stats.failed_nodes,
            "DAG execution completed"
        );

        Ok(ExecutionResult {
            results: final_results,
            stats,
            token_snapshot: self.context.token_accountant.snapshot(),
            graph_status_snapshots: vec![],
        })
    }

    /// Execute a single node (for testing/debugging)
    pub async fn execute_node(
        &self,
        node: &apxm_core::types::execution::Node,
        inputs: Vec<Value>,
    ) -> Result<Value> {
        let outcome = self
            .execute_with_context(node, inputs, &self.context)
            .await?;
        Ok(outcome.value)
    }

    /// Execute a sub-DAG identified by a flow registry label.
    ///
    /// Drives `TRY_CATCH` branches without circular dependencies. Creates a
    /// child execution context and runs the
    /// referenced sub-DAG, returning the first non-null exit value.
    pub async fn run_subgraph_by_label(&self, label: &str, inputs: Vec<Value>) -> Result<Value> {
        // Labels are stored as "AgentName.flowName" or just "flowName".
        // Search the flow registry for a matching flow.
        let sub_dag = self
            .context
            .flow_registry
            .find_flow_by_label(label)
            .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
                op_type: apxm_core::types::operations::AISOperationType::TryCatch,
                message: format!("Sub-DAG label '{}' not found in flow registry", label),
            })?;

        let child_ctx = self.context.child();

        // Store inputs in STM for sub-DAG entry nodes
        for (i, input) in inputs.iter().enumerate() {
            let _ = child_ctx
                .memory
                .write_scoped(
                    crate::memory::MemorySpace::Stm,
                    child_ctx.scope_id(),
                    format!("flow_arg_{}", i),
                    input.clone(),
                )
                .await;
        }

        let child_engine = ExecutorEngine::new(child_ctx);
        let dag_to_execute = (*sub_dag).clone();
        let result = child_engine.execute_dag(dag_to_execute).await?;

        // Return first non-null exit value
        Ok(result
            .results
            .values()
            .find(|v| !matches!(v, Value::Null))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Get a reference to the execution context.
    pub fn context(&self) -> &ExecutionContext {
        &self.context
    }
}

/// Map a downstream op type to a GRAPH_EDGE `kind` label for colouring edges in
/// the observer view.
///
/// - `tool_invocation` when the consumer is an INV_CAP (the producer
///   feeds a tool call)
/// - `dispatch` when the consumer is a multi-agent op (SPAWN_AGENT,
///   COMMUNICATE, HANDOFF, DELEGATE)
/// - `synthesis_feed` everywhere else (data flowing into a synthesis
///   or aggregation node)
fn graph_edge_kind_for_consumer(op: AISOperationType) -> &'static str {
    match op {
        AISOperationType::InvCap => "tool_invocation",
        AISOperationType::SpawnAgent
        | AISOperationType::Communicate
        | AISOperationType::Handoff
        | AISOperationType::Delegate => "dispatch",
        _ => "synthesis_feed",
    }
}

fn sequential_priority_label(priority: u32) -> String {
    crate::scheduler::Priority::from_u8(priority.min(u8::MAX as u32) as u8)
        .as_str()
        .to_string()
}

/// Outcome of a single operation execution
#[derive(Debug, Clone)]
pub struct OperationOutcome {
    pub value: Value,
}

/// Result of DAG execution
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Final results mapped by token ID
    pub results: HashMap<u64, Value>,
    /// Execution statistics
    pub stats: ExecutionStats,
    /// Aggregate token usage collected during execution. Empty if no LLM nodes ran.
    pub token_snapshot: crate::executor::token_accounting::TokenAccountingSnapshot,
    /// Backend graph status snapshots captured before graph release.
    pub graph_status_snapshots: Vec<GraphStatusSnapshot>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::types::{DependencyType, Edge, Node, NodeMetadata};
    use std::collections::HashMap as Map;

    fn nop_node(id: u64, inputs: Vec<u64>, outputs: Vec<u64>) -> Node {
        Node {
            id,
            op_type: AISOperationType::Nop,
            attributes: Map::new(),
            input_tokens: inputs,
            output_tokens: outputs,
            metadata: NodeMetadata::default(),
        }
    }

    /// Linear chain `1 --t10--> 2 --t20--> 3` of NOP passthrough nodes.
    ///
    /// NOP returns its first input verbatim (or `Null` for an input-less entry
    /// node). That makes re-execution observable: if upstream node 1 is *not*
    /// re-run, the seeded value for token 10 flows through nodes 2 and 3 to the
    /// exit token; if node 1 *were* re-run, token 10 would become `Null` (it has
    /// no inputs) and the exit value would be `Null` instead.
    fn nop_chain() -> ExecutionDag {
        let mut dag = ExecutionDag::new();
        dag.add_node(nop_node(1, vec![], vec![10])).unwrap();
        dag.add_node(nop_node(2, vec![10], vec![20])).unwrap();
        dag.add_node(nop_node(3, vec![20], vec![30])).unwrap();
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .unwrap();
        dag.add_edge(Edge::new(2, 3, 20, DependencyType::Data))
            .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag
    }

    async fn test_context() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("in-memory memory system"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam)
    }

    /// RESIDUAL 1 regression: the `ExecutorEngine` fallback scheduler entry
    /// (`execute_dag_parallel`, reached through `execute_dag` for multi-node
    /// graphs) must honor the same replay-seed metadata as the server path. With
    /// `replay_from_node`/`replay_token_values` stamped, only `from_node` and its
    /// descendants execute; the upstream node is pre-completed from the prior
    /// run's boundary value and is never re-invoked.
    #[tokio::test]
    async fn fallback_path_honors_replay_seed() {
        let dag = nop_chain();

        let mut ctx = test_context().await;
        // Replay from node 2: node 1 is upstream and must be pre-completed.
        ctx.metadata.insert(
            crate::metadata_keys::REPLAY_FROM_NODE.to_string(),
            "2".to_string(),
        );
        // Prior run's captured value for the boundary token 10 (node 1's output).
        let prior = serde_json::json!({ "10": "seeded-upstream" }).to_string();
        ctx.metadata
            .insert(crate::metadata_keys::REPLAY_TOKEN_VALUES.to_string(), prior);

        let engine = ExecutorEngine::new(ctx);
        let result = engine.execute_dag(dag).await.expect("replay execution");

        // Only nodes 2 and 3 re-execute; node 1 is pre-completed, not counted.
        assert_eq!(
            result.stats.executed_nodes, 2,
            "fallback path must re-execute from_node + descendants only, not the upstream node"
        );

        // The exit token (30) carries the seeded upstream value passed through
        // nodes 2 and 3. Had node 1 been re-run, token 10 would be Null and the
        // exit value would be Null — proving node 1's handler was not re-invoked.
        assert_eq!(
            result.results.get(&30),
            Some(&Value::String("seeded-upstream".to_string())),
            "upstream node must NOT be re-executed; its prior output must flow through"
        );
    }

    #[tokio::test]
    async fn sequential_fallback_normalizes_replay_edges_before_dispatch() {
        let dag = nop_chain();
        let mut ctx = test_context().await;
        ctx.metadata.insert(
            crate::metadata_keys::REPLAY_FROM_NODE.to_string(),
            "2".to_string(),
        );
        ctx.metadata.insert(
            crate::metadata_keys::REPLAY_TOKEN_VALUES.to_string(),
            serde_json::json!({ "10": "seeded-upstream" }).to_string(),
        );

        let result = ExecutorEngine::new(ctx)
            .execute_dag_sequential(dag)
            .await
            .expect("normalized sequential replay execution");

        assert_eq!(result.stats.executed_nodes, 2);
        assert_eq!(
            result.results.get(&30),
            Some(&Value::String("seeded-upstream".to_string()))
        );
    }

    /// A requested partial replay may not skip a completed capability invocation
    /// until the runtime can load a persisted effect and approval receipt for it.
    /// Token values alone prove dataflow continuity, not authority parity.
    #[tokio::test]
    async fn fallback_path_rejects_replay_that_skips_a_capability_effect() {
        let mut dag = nop_chain();
        dag.nodes[0].op_type = AISOperationType::InvCap;
        dag.nodes[0].attributes.insert(
            apxm_core::constants::graph::attrs::CAPABILITY.to_string(),
            Value::String("calendar.write".to_string()),
        );

        let mut ctx = test_context().await;
        ctx.metadata.insert(
            crate::metadata_keys::REPLAY_FROM_NODE.to_string(),
            "2".to_string(),
        );
        ctx.metadata.insert(
            crate::metadata_keys::REPLAY_TOKEN_VALUES.to_string(),
            serde_json::json!({ "10": "prior-capability-output" }).to_string(),
        );
        ctx.metadata.insert(
            crate::metadata_keys::REPLAY_SOURCE_EXECUTION_ID.to_string(),
            "execution-1".to_string(),
        );

        let error = ExecutorEngine::new(ctx)
            .execute_dag(dag)
            .await
            .expect_err("partial replay must reject an unverified capability effect");
        assert_eq!(
            error.to_string(),
            "Scheduler error: partial replay rejected: partial replay cannot reuse completed capability node 1: durable host-effect evidence is missing"
        );
    }

    /// Control: without replay metadata, the same fallback path runs the whole
    /// graph. The input-less entry NOP produces `Null`, which flows to the exit —
    /// distinct from the seeded-replay result above, confirming the assertion in
    /// `fallback_path_honors_replay_seed` is load-bearing.
    #[tokio::test]
    async fn fallback_path_full_run_without_seed_executes_all_nodes() {
        let dag = nop_chain();
        let ctx = test_context().await;
        let engine = ExecutorEngine::new(ctx);
        let result = engine.execute_dag(dag).await.expect("full execution");

        assert_eq!(
            result.stats.executed_nodes, 3,
            "without a replay seed every node executes"
        );
        assert_eq!(
            result.results.get(&30),
            Some(&Value::Null),
            "the input-less entry NOP produces Null on a full re-run"
        );
    }

    /// An `INV_CAP` node invoking a capability that is never registered with
    /// the (nothing-registered) test `CapabilitySystem`, chained to a NOP.
    /// Every attempt fails identically, so with `scheduler_config.max_retries
    /// = 0` the parallel dataflow scheduler exhausts its retries on the very
    /// first attempt and returns `RuntimeError::SchedulerRetryExhausted`
    /// deterministically (`worker.rs`) — no timing race, no timeout. The
    /// sequential executor, by contrast, just records the node's error
    /// without aborting the whole DAG (see `execute_dag_sequential`'s
    /// `Err(error)` arm below), so it still completes.
    fn failing_inv_cap_dag() -> ExecutionDag {
        let mut inv_cap = Node::new(1, AISOperationType::InvCap);
        inv_cap.output_tokens = vec![10];
        inv_cap.set_attribute(
            apxm_core::constants::graph::attrs::CAPABILITY.to_string(),
            Value::String("nonexistent_capability_for_scheduler_fallback_test".to_string()),
        );
        inv_cap.set_attribute(
            apxm_core::constants::graph::attrs::PARAMS_JSON.to_string(),
            Value::String("{}".to_string()),
        );

        let mut dag = ExecutionDag::new();
        dag.add_node(inv_cap).unwrap();
        dag.add_node(nop_node(2, vec![10], vec![20])).unwrap();
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag
    }

    /// Captures every `emit_warning` call; every other method uses the trait's
    /// default no-op.
    #[derive(Default, Clone)]
    struct WarningCapturingEmitter {
        warnings: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }

    impl super::super::events::ExecutionEventEmitter for WarningCapturingEmitter {
        fn emit_llm_token(&self, _content: &str) {}
        fn emit_tool_start(&self, _name: &str, _args: &Map<String, Value>) {}
        fn emit_tool_end(&self, _name: &str, _result: &Value) {}
        fn emit_warning(&self, code: &str, message: &str) {
            self.warnings
                .lock()
                .unwrap()
                .push((code.to_string(), message.to_string()));
        }
    }

    /// Regression — fallback flag off by default: `SchedulerConfig::default()`
    /// has `allow_sequential_fallback = false`, and a forced parallel-scheduler
    /// error propagates as an `Err` instead of silently running the DAG
    /// sequentially.
    #[tokio::test]
    async fn scheduler_fallback_disabled_by_default_propagates_error() {
        let mut ctx = test_context().await;
        assert!(
            !ctx.scheduler_config.allow_sequential_fallback,
            "SchedulerConfig::default() must have the fallback disabled"
        );
        // No retries: the forced failure is deterministic on the first
        // attempt, not dependent on retry-backoff timing.
        ctx.scheduler_config.max_retries = 0;

        let engine = ExecutorEngine::new(ctx);
        let result = engine.execute_dag(failing_inv_cap_dag()).await;

        assert!(
            result.is_err(),
            "a forced scheduler error must propagate when the fallback flag is off, \
             not silently run sequential: {result:?}"
        );
    }

    /// With the flag explicitly on, the same forced scheduler error falls
    /// back to `execute_dag_sequential` unchanged — proven by the identical
    /// unregistered-capability failure surfacing through the sequential
    /// executor's own (pre-existing, unchanged) "a node error aborts the DAG"
    /// behavior, rather than the parallel scheduler's
    /// `SchedulerRetryExhausted` — and the alert-event fires exactly once for
    /// the one trigger.
    #[tokio::test]
    async fn scheduler_fallback_flag_on_runs_sequential_and_alerts_once() {
        let mut ctx = test_context().await;
        ctx.scheduler_config.allow_sequential_fallback = true;
        ctx.scheduler_config.max_retries = 0;
        let emitter = WarningCapturingEmitter::default();
        let warnings = Arc::clone(&emitter.warnings);
        ctx.event_emitter = Some(Arc::new(emitter));

        let engine = ExecutorEngine::new(ctx);
        let result = engine.execute_dag(failing_inv_cap_dag()).await;

        assert!(
            result.is_err(),
            "the unregistered capability must still fail via the (unchanged) \
             sequential executor: {result:?}"
        );

        let captured = warnings.lock().unwrap().clone();
        assert_eq!(
            captured.len(),
            1,
            "the alert-event must fire exactly once per trigger: {captured:?}"
        );
        assert_eq!(captured[0].0, "scheduler_fallback_triggered");
    }
}
