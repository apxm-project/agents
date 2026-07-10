//! Dataflow scheduler that executes DAG nodes when all inputs are ready.

use std::sync::Arc;
use std::time::{Duration, Instant};

use apxm_core::types::{ExecutionDag, ExecutionStats, Value};
use apxm_core::{apxm_dag, apxm_sched};
use tokio::task::JoinHandle;

use crate::executor::ExecutorEngine;
use crate::executor::{ExecutionContext, ExecutionHookContext};
use crate::observability::{MetricsCollector, SchedulerMetrics};
use crate::scheduler::config::SchedulerConfig;
use crate::scheduler::state::SchedulerState;
use crate::scheduler::worker;
use apxm_core::error::RuntimeError;

type RuntimeResult<T> = Result<T, RuntimeError>;

/// Result payload shared by [`DataflowScheduler::execute_with_hooks_and_seed`]
/// and the `Completed` arm of [`SchedulerOutcome`].
type SchedulerExecutionResult = (
    std::collections::HashMap<u64, Value>,
    ExecutionStats,
    SchedulerMetrics,
    Option<std::collections::HashMap<u64, Value>>,
    Option<std::collections::HashMap<u64, Vec<u64>>>,
);

/// Outcome of [`DataflowScheduler::execute_or_park`]: either the DAG ran to
/// completion, or a node parked on the conversation-loop's session-recv key
/// before completion ( narrow park observability).
///
/// `Parked` does NOT mean execution stopped — the spawned workers keep running
/// in the background so the session loop continues to make progress; this
/// variant only reports that the caller no longer needs to keep waiting on
/// this particular execution future to know "a turn boundary was reached".
pub enum SchedulerOutcome {
    Completed(SchedulerExecutionResult),
    Parked {
        session_id: String,
        /// Handle to the detached background task that joins the still-running
        /// workers and finishes scheduler-level bookkeeping (stats/hooks) once
        /// the DAG eventually completes for real (or is cancelled). A caller
        /// that owns additional per-execution cleanup tied to full completion
        /// (e.g. releasing backend graph lifecycle state) MUST chain onto this
        /// handle rather than doing that cleanup immediately — the execution
        /// is NOT done just because it parked.
        background: JoinHandle<()>,
    },
}

/// Dataflow scheduler for executing DAGs with automatic parallelism.
///
/// Uses token-based dataflow execution:
/// - Operations execute when all input tokens are ready
/// - Completed operations publish output tokens
/// - Downstream consumers become ready when their inputs arrive
pub struct DataflowScheduler {
    config: SchedulerConfig,
}

impl DataflowScheduler {
    /// Create a new dataflow scheduler.
    pub fn new(config: SchedulerConfig) -> Self {
        Self { config }
    }

    /// Execute a DAG to completion, optionally with input values.
    ///
    /// When `inputs` is non-empty, values are injected into flow parameter tokens.
    /// Returns the exit values, execution statistics, and scheduler metrics.
    pub async fn execute(
        &self,
        dag: ExecutionDag,
        executor: Arc<ExecutorEngine>,
        ctx: ExecutionContext,
        inputs: Vec<Value>,
    ) -> RuntimeResult<(
        std::collections::HashMap<u64, Value>,
        ExecutionStats,
        SchedulerMetrics,
        Option<std::collections::HashMap<u64, Value>>,
        Option<std::collections::HashMap<u64, Vec<u64>>>,
    )> {
        self.execute_with_hooks(dag, executor, ctx, inputs, ExecutionHookContext::default())
            .await
    }

    /// Execute a DAG with scheduler-level execution hooks.
    pub async fn execute_with_hooks(
        &self,
        dag: ExecutionDag,
        executor: Arc<ExecutorEngine>,
        ctx: ExecutionContext,
        inputs: Vec<Value>,
        hooks: ExecutionHookContext,
    ) -> RuntimeResult<(
        std::collections::HashMap<u64, Value>,
        ExecutionStats,
        SchedulerMetrics,
        Option<std::collections::HashMap<u64, Value>>,
        Option<std::collections::HashMap<u64, Vec<u64>>>,
    )> {
        self.execute_with_hooks_and_seed(dag, executor, ctx, inputs, hooks, None)
            .await
    }

    /// Partial replay (`rerun-from-node`): execute only `replay_seed.from_node`
    /// and its descendants, reusing the prior run's boundary token values for the
    /// pre-completed upstream nodes. When `replay_seed` is `None` this is an
    /// ordinary full run.
    pub async fn execute_with_hooks_and_seed(
        &self,
        dag: ExecutionDag,
        executor: Arc<ExecutorEngine>,
        mut ctx: ExecutionContext,
        inputs: Vec<Value>,
        hooks: ExecutionHookContext,
        replay_seed: Option<&crate::scheduler::replay::ReplaySeed>,
    ) -> RuntimeResult<(
        std::collections::HashMap<u64, Value>,
        ExecutionStats,
        SchedulerMetrics,
        Option<std::collections::HashMap<u64, Value>>,
        Option<std::collections::HashMap<u64, Vec<u64>>>,
    )> {
        let start = Instant::now();

        apxm_sched!(info,
            execution_id = %ctx.execution_id,
            nodes = dag.nodes.len(),
            inputs = inputs.len(),
            max_concurrency = self.config.max_concurrency,
            max_inflight = self.config.max_inflight,
            "Starting DAG execution"
        );

        apxm_dag!(debug,
            entry_nodes = ?dag.entry_nodes,
            exit_nodes = ?dag.exit_nodes,
            "DAG structure loaded"
        );

        // Apply runtime latency tier overrides before cost enforcement
        let dag = self.apply_latency_overrides(dag);

        // Validate DAG cost budget early
        self.enforce_cost_budget(&dag)?;
        hooks.emit_graph_started(dag.nodes.len());

        // Create a new MetricsCollector for each execution to avoid accumulating
        // metrics across multiple workflow runs (fix for work_stealing timer overflow)
        let metrics = Arc::new(MetricsCollector::new());

        // Build shared scheduler state
        let (mut state, workers) = SchedulerState::new_with_replay(
            dag,
            self.config.clone(),
            metrics.clone(),
            start,
            inputs,
            hooks.clone(),
            replay_seed,
        )?;
        // Carry the host admission key (if any) so a parked execution releases its
        // cross-execution admission slot and reacquires it on wake.
        state.admission_id = ctx
            .metadata
            .get(crate::metadata_keys::ADMISSION_ID)
            .cloned();
        state.cancellation_token = ctx.cancellation_token.clone();
        // Project AAM goal priorities onto scheduler node priorities.
        // This bridges the two priority systems: compile-time node.metadata.priority
        // and runtime Goal.priority in the AAM.
        state.apply_goal_priorities(&ctx.aam);

        let state = Arc::new(state);

        ctx.dag_splicer = Arc::new(super::splicing::SchedulerDagSplicer::new(Arc::clone(
            &state,
        )));

        // Register the completion waiter BEFORE spawning workers. A fast DAG
        // can otherwise complete and call `notify_done.notify_waiters()` before
        // this task first polls `notified()`, losing the wakeup (tokio `Notify`
        // does not store `notify_waiters` permits) and hanging the execute
        // future forever. `enable()` registers the waiter up front so the
        // notification cannot be missed.
        let mut done = std::pin::pin!(state.notify_done.notified());
        done.as_mut().enable();

        // Spawn watchdog for deadlock detection
        spawn_watchdog(Arc::clone(&state));

        // Spawn worker threads
        let worker_handles = spawn_workers(state.clone(), workers, executor, ctx);

        apxm_sched!(
            debug,
            workers_spawned = worker_handles.len(),
            "All workers spawned, waiting for completion"
        );

        // Wait for completion, failure, or host-owned cancellation (skip if already complete).
        if state.remaining.load(std::sync::atomic::Ordering::SeqCst) != 0 && !state.is_cancelled() {
            let cancellation_token = state.cancellation_token.clone();
            tokio::select! {
                _ = done => {}
                _ = cancellation_token.cancelled() => {
                    state.set_first_error(RuntimeError::SchedulerCancelled);
                    state.mark_done();
                }
            }
        }

        // Clean shutdown: wait for all workers to finish
        for handle in worker_handles {
            let _ = handle.await;
        }

        apxm_sched!(debug, "All workers terminated");

        // Check for errors
        let mut first_error = state.first_error.lock();
        if let Some(error) = first_error.take() {
            apxm_sched!(error, error = %error, "DAG execution failed");
            let stats = state.build_stats();
            hooks.emit_graph_finished(
                stats.executed_nodes,
                stats.failed_nodes,
                stats.duration_ms,
                false,
            );
            return Err(error);
        }

        // Collect exit values
        let results = state.collect_exit_values()?;

        let (all_outputs, node_output_map) = if self.config.collect_all_outputs {
            (
                Some(state.collect_all_values()?),
                Some(state.node_output_map()),
            )
        } else {
            (None, None)
        };

        // Build statistics
        let stats = state.build_stats();
        hooks.emit_graph_finished(
            stats.executed_nodes,
            stats.failed_nodes,
            stats.duration_ms,
            stats.failed_nodes == 0,
        );

        // Capture scheduler metrics snapshot
        let scheduler_metrics = SchedulerMetrics::from_collector(&state.metrics);

        apxm_sched!(
            info,
            duration_ms = start.elapsed().as_millis(),
            executed = stats.executed_nodes,
            failed = stats.failed_nodes,
            "DAG execution completed"
        );

        Ok((
            results,
            stats,
            scheduler_metrics,
            all_outputs,
            node_output_map,
        ))
    }

    /// Execute a DAG, but return as soon as EITHER the DAG completes OR a node
    /// parks on the conversation-loop's session-recv key ( narrow park
    /// observability) — whichever happens first.
    ///
    /// This is a genuinely new entry point: it does not change the behavior of
    /// [`Self::execute`] / [`Self::execute_with_hooks`] /
    /// [`Self::execute_with_hooks_and_seed`], which still block until full DAG
    /// completion exactly as before. Existing callers (`/v1/execute/stream`,
    /// `apxm chat`) are unaffected.
    ///
    /// If a session-recv park fires first, the already-spawned workers are NOT
    /// cancelled or joined here — they are hand off to a detached background
    /// task that waits for the eventual real completion (or host
    /// cancellation) and finalizes bookkeeping (stats, hooks) there, since no
    /// caller is left waiting on this future's result. The DAG keeps making
    /// progress; only the "please block until fully done" behavior is relaxed
    /// for this one entry point.
    pub async fn execute_or_park(
        &self,
        dag: ExecutionDag,
        executor: Arc<ExecutorEngine>,
        mut ctx: ExecutionContext,
        inputs: Vec<Value>,
        hooks: ExecutionHookContext,
        replay_seed: Option<&crate::scheduler::replay::ReplaySeed>,
    ) -> RuntimeResult<SchedulerOutcome> {
        let start = Instant::now();

        apxm_sched!(info,
            execution_id = %ctx.execution_id,
            nodes = dag.nodes.len(),
            inputs = inputs.len(),
            max_concurrency = self.config.max_concurrency,
            max_inflight = self.config.max_inflight,
            "Starting DAG execution (park-observable)"
        );

        let dag = self.apply_latency_overrides(dag);
        self.enforce_cost_budget(&dag)?;
        hooks.emit_graph_started(dag.nodes.len());

        let metrics = Arc::new(MetricsCollector::new());

        let (mut state, workers) = SchedulerState::new_with_replay(
            dag,
            self.config.clone(),
            metrics.clone(),
            start,
            inputs,
            hooks.clone(),
            replay_seed,
        )?;
        state.admission_id = ctx
            .metadata
            .get(crate::metadata_keys::ADMISSION_ID)
            .cloned();
        state.cancellation_token = ctx.cancellation_token.clone();
        state.apply_goal_priorities(&ctx.aam);

        let state = Arc::new(state);

        ctx.dag_splicer = Arc::new(super::splicing::SchedulerDagSplicer::new(Arc::clone(
            &state,
        )));

        // Register the completion waiter and the park-observability
        // subscription BEFORE spawning workers, for the same lost-wakeup
        // reason as `execute_with_hooks_and_seed`: a fast DAG (or an
        // immediate park) can otherwise fire before this task first polls,
        // and `Notify`/`watch` do not replay missed edges to a late waiter.
        let mut done = std::pin::pin!(state.notify_done.notified());
        done.as_mut().enable();
        let mut session_parked_rx = state.subscribe_session_parked();

        spawn_watchdog(Arc::clone(&state));
        let worker_handles = spawn_workers(state.clone(), workers, executor, ctx);

        apxm_sched!(
            debug,
            workers_spawned = worker_handles.len(),
            "All workers spawned, racing completion vs session-recv park"
        );

        let already_done =
            state.remaining.load(std::sync::atomic::Ordering::SeqCst) == 0 || state.is_cancelled();

        if !already_done {
            let cancellation_token = state.cancellation_token.clone();
            tokio::select! {
                _ = &mut done => {}
                _ = cancellation_token.cancelled() => {
                    state.set_first_error(RuntimeError::SchedulerCancelled);
                    state.mark_done();
                }
                changed = session_parked_rx.changed() => {
                    if changed.is_ok()
                        && let Some(session_id) = session_parked_rx.borrow_and_update().clone()
                    {
                        apxm_sched!(
                            info,
                            %session_id,
                            "Execution parked on session-recv; returning Parked without waiting for full completion"
                        );
                        let background = tokio::spawn(finalize_parked_background(
                            Arc::clone(&state),
                            hooks,
                            worker_handles,
                        ));
                        return Ok(SchedulerOutcome::Parked {
                            session_id,
                            background,
                        });
                    }
                    // Sender closed or a spurious/None wakeup (should not
                    // happen — the sender only ever sends `Some`): fall back
                    // to waiting for the real completion signal.
                    done.await;
                }
            }
        }

        for handle in worker_handles {
            let _ = handle.await;
        }

        apxm_sched!(debug, "All workers terminated");

        let mut first_error = state.first_error.lock();
        if let Some(error) = first_error.take() {
            apxm_sched!(error, error = %error, "DAG execution failed");
            let stats = state.build_stats();
            hooks.emit_graph_finished(
                stats.executed_nodes,
                stats.failed_nodes,
                stats.duration_ms,
                false,
            );
            return Err(error);
        }
        drop(first_error);

        let results = state.collect_exit_values()?;
        let (all_outputs, node_output_map) = if self.config.collect_all_outputs {
            (
                Some(state.collect_all_values()?),
                Some(state.node_output_map()),
            )
        } else {
            (None, None)
        };

        let stats = state.build_stats();
        hooks.emit_graph_finished(
            stats.executed_nodes,
            stats.failed_nodes,
            stats.duration_ms,
            stats.failed_nodes == 0,
        );

        let scheduler_metrics = SchedulerMetrics::from_collector(&state.metrics);

        Ok(SchedulerOutcome::Completed((
            results,
            stats,
            scheduler_metrics,
            all_outputs,
            node_output_map,
        )))
    }

    /// Apply runtime latency tier overrides to DAG nodes.
    ///
    /// For each node that has a `"backend"` attribute matching a key in the
    /// configured `latency_tiers`, its `estimated_latency` is replaced with
    /// the runtime-configured value.  This runs before cost-budget
    /// enforcement so that budgets reflect real-world backend latencies.
    ///
    /// When the latency tier config is empty (default), this is a no-op.
    fn apply_latency_overrides(&self, mut dag: ExecutionDag) -> ExecutionDag {
        if self.config.latency_tiers.is_empty() {
            return dag;
        }

        let tiers = &self.config.latency_tiers;
        let mut overridden = 0usize;

        for node in dag.nodes.iter_mut() {
            let backend = node
                .attributes
                .get(apxm_core::constants::graph::attrs::BACKEND)
                .and_then(|v| v.as_string().map(|s| s.to_string()));

            if let Some(ref backend_name) = backend {
                if let Some(latency) = tiers.resolve(backend_name) {
                    node.metadata.estimated_latency = Some(latency);
                    overridden += 1;
                }
            }
        }

        if overridden > 0 {
            tracing::info!(
                overridden_nodes = overridden,
                "Applied runtime latency tier overrides"
            );
        }

        dag
    }

    /// Enforce the cost budget for the DAG.
    ///
    /// Returns an error if the total estimated cost exceeds max_cost.
    fn enforce_cost_budget(&self, dag: &ExecutionDag) -> RuntimeResult<()> {
        if self.config.max_cost == 0 {
            return Ok(()); // No limit
        }

        let total_cost: usize = dag
            .nodes
            .iter()
            .map(|n| n.metadata.estimated_latency.unwrap_or(0) as usize)
            .sum();

        if total_cost > self.config.max_cost {
            return Err(RuntimeError::Scheduler {
                message: format!(
                    "DAG cost {} exceeds max_cost budget of {}",
                    total_cost, self.config.max_cost
                ),
            });
        }

        Ok(())
    }
}

/// Finish out a parked execution's bookkeeping in the background once its
/// caller has already returned [`SchedulerOutcome::Parked`].
///
/// No one is awaiting a result at this point, so this only joins the worker
/// handles (letting the DAG run to its eventual real completion / host
/// cancellation) and emits the same finishing hooks / logs that
/// [`DataflowScheduler::execute_or_park`] would have emitted had it stayed
/// blocked — it does not resurrect the discarded result for anyone to
/// consume.
async fn finalize_parked_background(
    state: Arc<SchedulerState>,
    hooks: ExecutionHookContext,
    worker_handles: Vec<JoinHandle<()>>,
) {
    for handle in worker_handles {
        let _ = handle.await;
    }

    let error = state.first_error.lock().take();
    let stats = state.build_stats();
    hooks.emit_graph_finished(
        stats.executed_nodes,
        stats.failed_nodes,
        stats.duration_ms,
        error.is_none() && stats.failed_nodes == 0,
    );

    if let Some(error) = error {
        apxm_sched!(
            error,
            error = %error,
            "Background (previously-parked) execution finished with an error"
        );
    } else {
        apxm_sched!(
            info,
            executed = stats.executed_nodes,
            failed = stats.failed_nodes,
            "Background (previously-parked) execution completed"
        );
    }
}

/// Spawn watchdog for deadlock detection.
///
/// Periodically checks if progress is being made. If no progress occurs
/// for `deadlock_timeout_ms`, execution is aborted.
fn spawn_watchdog(state: Arc<SchedulerState>) {
    let cfg = state.cfg.clone();

    tokio::spawn(async move {
        let interval = Duration::from_millis(cfg.watchdog_interval_ms);
        loop {
            // Edge-triggered: workers wake us on progress/done. The timeout is
            // the safety net so deadlock detection still fires when idle.
            let _ = tokio::time::timeout(interval, state.watchdog_notify.notified()).await;

            // Check if execution is complete
            if state.remaining.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                break;
            }

            // Check if cancelled
            if state.is_cancelled() {
                break;
            }

            // Check for deadlock
            let now_ms = state.elapsed_ms() as u64;
            let last_progress_ms = state
                .last_progress_ms
                .load(std::sync::atomic::Ordering::Relaxed);

            if now_ms.saturating_sub(last_progress_ms) >= cfg.deadlock_timeout_ms {
                // A DAG with PARKED nodes is legitimately waiting on an external
                // event (e.g. a human resuming a PAUSE), not deadlocked — it may
                // wait arbitrarily long. Reset the timer and keep waiting.
                if state.parked_count() > 0 {
                    state.record_progress();
                    continue;
                }

                // Before declaring a deadlock, check if any operations are
                // actively running. Long-running operations (e.g. LLM calls)
                // are not deadlocks — the scheduler is alive, just waiting.
                if state.has_running_ops() {
                    // Operations are in-flight; reset the progress timer
                    // so we don't keep re-checking every watchdog interval.
                    state.record_progress();
                    continue;
                }

                // True deadlock: nothing is running and no progress was made.
                let remaining = state.remaining.load(std::sync::atomic::Ordering::SeqCst);

                state.set_first_error(RuntimeError::SchedulerDeadlock {
                    timeout_ms: cfg.deadlock_timeout_ms,
                    remaining,
                });

                state.mark_done();
                break;
            }
        }
    });
}

/// Spawn worker threads.
///
/// Each worker runs the worker loop, stealing work and executing operations.
fn spawn_workers(
    state: Arc<SchedulerState>,
    workers: Vec<crossbeam_deque::Worker<apxm_core::types::NodeId>>,
    executor: Arc<ExecutorEngine>,
    base_ctx: ExecutionContext,
) -> Vec<JoinHandle<()>> {
    workers
        .into_iter()
        .enumerate()
        .map(|(worker_id, local_worker)| {
            let state = Arc::clone(&state);
            let executor = Arc::clone(&executor);
            let base_ctx = base_ctx.clone();

            apxm_sched!(debug, worker = worker_id, "Spawning worker");

            tokio::spawn(async move {
                apxm_sched!(debug, worker = worker_id, "Worker started");
                worker::worker_loop(worker_id, local_worker, state, executor, base_ctx).await;
                apxm_sched!(debug, worker = worker_id, "Worker stopped");
            })
        })
        .collect()
}

/// W2.6 exactly-N-iterations conformance: real end-to-end proof that the ONE
/// production iteration mechanism left after `LOOP_START`/`LOOP_END` were
/// deleted — graph splicing (`SchedulerState::splice_dag`, driven here the
/// same way `rearm_session_turn` drives it on every real wake) — dispatches a
/// loop body through the real dispatcher exactly the number of times the
/// external driver splices it. Unlike the (deleted) `LOOP_START`/`LOOP_END`
/// pair, nothing here is compiled-but-ignored: every spliced node runs
/// through the same worker pool and dispatcher production traffic uses.
/// See `docs/plans/tasks/W2.6.md`.
#[cfg(test)]
mod loop_conformance_tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use crate::scheduler::splicing::SpliceConfig;
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::values::Number;
    use apxm_core::types::{ExecutionDag, Node, Value};
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::time::timeout;

    fn test_scheduler_config() -> SchedulerConfig {
        SchedulerConfig::new()
            .with_max_concurrency(2)
            .with_max_inflight(4)
    }

    async fn test_context() -> (ExecutionContext, Aam) {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("in-memory memory system"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        let ctx = ExecutionContext::new(
            memory,
            Arc::new(LLMRegistry::new()),
            capability_system,
            aam.clone(),
        );
        (ctx, aam)
    }

    /// The minimal seed DAG: a single entry NOP. The "loop" itself never
    /// appears in the compiled graph — exactly the point: there is no
    /// `LOOP_START`/`LOOP_END` IR to lie about iteration. Every body
    /// execution below arrives dynamically via `splice_dag`.
    fn seed_dag() -> ExecutionDag {
        let mut dag = ExecutionDag::new();
        dag.add_node(Node {
            id: 1,
            op_type: AISOperationType::Nop,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: NodeMetadata::default(),
        })
        .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag
    }

    /// Spawn a real scheduler run against `seed_dag()` and hand back the live
    /// `Arc<SchedulerState>` so the test driver can splice into it — playing
    /// the same role the production worker park path plays when it calls
    /// `rearm_session_turn` on every wake (`worker.rs`'s `ExecutionOutcome::
    /// Parked` arm / `session_loop_rearm_spec`), just invoked directly instead
    /// of through a parked RECV node. No graph-native back-edge is created;
    /// the driver alone decides whether another iteration happens.
    async fn spawn_seeded_execution(
        ctx: ExecutionContext,
    ) -> (Arc<SchedulerState>, Vec<tokio::task::JoinHandle<()>>) {
        let metrics = Arc::new(MetricsCollector::new());
        let (state, workers) = SchedulerState::new(
            seed_dag(),
            test_scheduler_config(),
            metrics,
            Instant::now(),
            vec![],
        )
        .unwrap();
        // Hold the DAG open for the test's duration: the 1-node seed DAG
        // would otherwise complete almost instantly, drive `remaining` to 0,
        // and let every worker observe that and terminate its loop — exactly
        // the way a real production execution stays open only because the
        // session-recv node PARKS (defers its completion) rather than
        // finishing outright. This phantom unit plays that same role without
        // needing a real parking handler (which would need a live session /
        // HTTP checkpoint server); `finish()` releases it via `mark_done()`.
        state.remaining.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let state = Arc::new(state);
        let executor = Arc::new(ExecutorEngine::new(ctx.clone()));
        let handles = spawn_workers(Arc::clone(&state), workers, executor, ctx);
        (state, handles)
    }

    /// A UMEM body node with no input tokens, so its handler (`umem.rs`)
    /// always uses the literal `VALUE` attribute rather than an input — the
    /// driver controls the counter value deterministically per splice.
    fn counter_body_node(key: &str, iteration: i64) -> Node {
        let mut attrs = HashMap::new();
        attrs.insert(graph_attrs::KEY.to_string(), Value::String(key.to_string()));
        attrs.insert(
            graph_attrs::VALUE.to_string(),
            Value::Number(Number::Integer(iteration)),
        );
        Node {
            id: 1, // local id; splice_dag remaps to a fresh live id
            op_type: AISOperationType::UMem,
            attributes: attrs,
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: NodeMetadata::default(),
        }
    }

    /// Splice one loop-body iteration and wait (bounded) for the real worker
    /// pool to have actually dispatched it, observed via the AAM belief the
    /// body's UMEM handler writes for real. This is the "wake" half of a
    /// splice-based iteration: the driver does not offer iteration N+1 until
    /// iteration N has genuinely run.
    async fn splice_iteration_and_await(
        state: &Arc<SchedulerState>,
        aam: &Aam,
        key: &str,
        iteration: i64,
    ) {
        let mut inner = ExecutionDag::new();
        inner.add_node(counter_body_node(key, iteration)).unwrap();
        state
            .splice_dag(SpliceConfig {
                inner_dag: inner,
                token_connections: HashMap::new(),
                node_id_offset: None,
                token_id_offset: None,
            })
            .expect("splice_dag succeeds");

        let expected = Value::Number(Number::Integer(iteration));
        timeout(Duration::from_secs(5), async {
            loop {
                if aam.get_belief(key).as_ref() == Some(&expected) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("spliced loop-body iteration did not execute in time");
    }

    /// Let the still-running worker pool wind down and join it, bounded so a
    /// bug can't hang the test suite.
    async fn finish(state: &Arc<SchedulerState>, handles: Vec<tokio::task::JoinHandle<()>>) {
        state.mark_done();
        for h in handles {
            let _ = timeout(Duration::from_secs(5), h).await;
        }
    }

    /// Positive (exactly-N-iterations conformance, the required evidence
    /// artifact): `max_iterations = 3`, a body node incrementing an
    /// observable counter (AAM belief); assert exactly 3 runs.
    #[tokio::test]
    async fn loop_body_executes_exactly_n_times() {
        let (ctx, aam) = test_context().await;
        let (state, handles) = spawn_seeded_execution(ctx).await;
        let key = "w26_loop_conformance_exact_n";

        for i in 1..=3i64 {
            splice_iteration_and_await(&state, &aam, key, i).await;
        }

        assert_eq!(aam.get_belief(key), Some(Value::Number(Number::Integer(3))));
        let stats = state.build_stats();
        assert_eq!(
            stats.executed_nodes, 4,
            "the 1-node seed plus exactly 3 spliced body executions, no more"
        );

        finish(&state, handles).await;
    }

    /// Condition false after 2 of 3 allowed iterations: assert exactly 2
    /// executions. The "condition" is evaluated by the driver between
    /// splices — precisely how a real re-arm loop decides whether to splice
    /// again (`ParkWaker::fire`'s turn-cap check), not a graph back-edge.
    #[tokio::test]
    async fn loop_terminates_on_condition_before_max_iterations() {
        let (ctx, aam) = test_context().await;
        let (state, handles) = spawn_seeded_execution(ctx).await;
        let key = "w26_loop_conformance_early_stop";
        let max_allowed = 3i64;

        let mut executed = 0i64;
        for i in 1..=max_allowed {
            splice_iteration_and_await(&state, &aam, key, i).await;
            executed = i;
            let condition_holds = i < 2; // false starting at iteration 2
            if !condition_holds {
                break;
            }
        }

        assert_eq!(executed, 2, "the driver stopped after iteration 2, not 3");
        assert_eq!(aam.get_belief(key), Some(Value::Number(Number::Integer(2))));
        let stats = state.build_stats();
        assert_eq!(
            stats.executed_nodes, 3,
            "the 1-node seed plus exactly 2 spliced body executions"
        );

        finish(&state, handles).await;
    }

    /// Bound/condition false immediately: the body never dispatches.
    #[tokio::test]
    async fn zero_iteration_loop_executes_body_zero_times() {
        let (ctx, aam) = test_context().await;
        let (state, handles) = spawn_seeded_execution(ctx).await;
        let key = "w26_loop_conformance_zero_iterations";

        // Let the seed settle without ever offering a first iteration.
        timeout(Duration::from_secs(5), async {
            loop {
                if state.build_stats().executed_nodes >= 1 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("seed node did not execute in time");

        assert_eq!(
            aam.get_belief(key),
            None,
            "a loop-body belief must not exist when the condition never holds"
        );
        assert_eq!(
            state.build_stats().executed_nodes,
            1,
            "only the 1-node seed ran; the body executed zero times"
        );

        finish(&state, handles).await;
    }
}
