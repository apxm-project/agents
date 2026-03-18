//! Dataflow scheduler that executes DAG nodes when all inputs are ready.

use std::sync::Arc;
use std::time::{Duration, Instant};

use apxm_core::types::{ExecutionDag, ExecutionStats, Value};
use apxm_core::{apxm_dag, apxm_sched};
use tokio::task::JoinHandle;

use crate::executor::ExecutionContext;
use crate::executor::ExecutorEngine;
use crate::observability::{MetricsCollector, SchedulerMetrics};
use crate::scheduler::config::SchedulerConfig;
use crate::scheduler::state::SchedulerState;
use crate::scheduler::worker;
use apxm_core::error::RuntimeError;

type RuntimeResult<T> = Result<T, RuntimeError>;

/// Dataflow scheduler for executing DAGs with automatic parallelism.
///
/// Uses token-based dataflow execution:
/// - Operations execute when all input tokens are ready
/// - Completed operations publish output tokens
/// - Downstream consumers become ready when their inputs arrive
pub struct DataflowScheduler {
    config: SchedulerConfig,
    metrics: Arc<MetricsCollector>,
}

impl DataflowScheduler {
    /// Create a new dataflow scheduler.
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            metrics: Arc::new(MetricsCollector::new()),
        }
    }

    /// Execute a DAG to completion, optionally with input values.
    ///
    /// When `inputs` is non-empty, values are injected into flow parameter tokens.
    /// Returns the exit values, execution statistics, and scheduler metrics.
    pub async fn execute(
        &self,
        dag: ExecutionDag,
        executor: Arc<ExecutorEngine>,
        mut ctx: ExecutionContext,
        inputs: Vec<Value>,
    ) -> RuntimeResult<(
        std::collections::HashMap<u64, Value>,
        ExecutionStats,
        SchedulerMetrics,
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

        // Validate DAG cost budget early
        self.enforce_cost_budget(&dag)?;

        // Build shared scheduler state
        let (state, workers) = SchedulerState::new(
            dag,
            self.config.clone(),
            self.metrics.clone(),
            start,
            inputs,
        )?;
        // Project AAM goal priorities onto scheduler node priorities.
        // This bridges the two priority systems: compile-time node.metadata.priority
        // and runtime Goal.priority in the AAM.
        state.apply_goal_priorities(&ctx.aam);

        let state = Arc::new(state);

        ctx.dag_splicer = Arc::new(super::splicing::SchedulerDagSplicer::new(Arc::clone(
            &state,
        )));

        // Spawn watchdog for deadlock detection
        spawn_watchdog(Arc::clone(&state));

        // Spawn worker threads
        let worker_handles = spawn_workers(state.clone(), workers, executor, ctx);

        apxm_sched!(
            debug,
            workers_spawned = worker_handles.len(),
            "All workers spawned, waiting for completion"
        );

        // Wait for completion or failure
        state.notify_done.notified().await;

        // Clean shutdown: wait for all workers to finish
        for handle in worker_handles {
            let _ = handle.await;
        }

        apxm_sched!(debug, "All workers terminated");

        // Check for errors
        let mut first_error = state.first_error.lock();
        if let Some(error) = first_error.take() {
            apxm_sched!(error, error = %error, "DAG execution failed");
            return Err(error);
        }

        // Collect exit values
        let results = state.collect_exit_values()?;

        // Build statistics
        let stats = state.build_stats();

        // Capture scheduler metrics snapshot
        let scheduler_metrics = SchedulerMetrics::from_collector(&state.metrics);

        apxm_sched!(
            info,
            duration_ms = start.elapsed().as_millis(),
            executed = stats.executed_nodes,
            failed = stats.failed_nodes,
            "DAG execution completed"
        );

        Ok((results, stats, scheduler_metrics))
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

/// Spawn watchdog for deadlock detection.
///
/// Periodically checks if progress is being made. If no progress occurs
/// for `deadlock_timeout_ms`, execution is aborted.
fn spawn_watchdog(state: Arc<SchedulerState>) {
    let cfg = state.cfg.clone();

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(cfg.watchdog_interval_ms)).await;

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

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::Node;
    use std::collections::HashMap;

    /// Helper: create a node with an estimated latency (used as cost).
    fn make_costed_node(id: u64, cost: u64) -> Node {
        Node {
            id,
            op_type: AISOperationType::Nop,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: NodeMetadata {
                priority: 0,
                estimated_latency: Some(cost),
                task_source_id: None,
            },
        }
    }

    fn make_node(id: u64) -> Node {
        Node {
            id,
            op_type: AISOperationType::Nop,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: NodeMetadata::default(),
        }
    }

    // ── enforce_cost_budget tests ──────────────────────────────────────

    #[test]
    fn test_enforce_cost_budget_no_limit() {
        let scheduler = DataflowScheduler::new(SchedulerConfig::new().with_max_cost(0));

        let mut dag = ExecutionDag::new();
        dag.add_node(make_costed_node(1, 999_999)).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        // max_cost == 0 means unlimited
        assert!(scheduler.enforce_cost_budget(&dag).is_ok());
    }

    #[test]
    fn test_enforce_cost_budget_within_limit() {
        let scheduler = DataflowScheduler::new(SchedulerConfig::new().with_max_cost(100));

        let mut dag = ExecutionDag::new();
        dag.add_node(make_costed_node(1, 30)).unwrap();
        dag.add_node(make_costed_node(2, 40)).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        // Total cost = 70, limit = 100 -> OK
        assert!(scheduler.enforce_cost_budget(&dag).is_ok());
    }

    #[test]
    fn test_enforce_cost_budget_exceeds_limit() {
        let scheduler = DataflowScheduler::new(SchedulerConfig::new().with_max_cost(50));

        let mut dag = ExecutionDag::new();
        dag.add_node(make_costed_node(1, 30)).unwrap();
        dag.add_node(make_costed_node(2, 40)).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        // Total cost = 70, limit = 50 -> error
        let result = scheduler.enforce_cost_budget(&dag);
        assert!(result.is_err());
        match result.unwrap_err() {
            RuntimeError::Scheduler { message } => {
                assert!(message.contains("70"));
                assert!(message.contains("50"));
            }
            e => panic!("Expected Scheduler error, got: {:?}", e),
        }
    }

    #[test]
    fn test_enforce_cost_budget_at_exact_limit() {
        let scheduler = DataflowScheduler::new(SchedulerConfig::new().with_max_cost(70));

        let mut dag = ExecutionDag::new();
        dag.add_node(make_costed_node(1, 30)).unwrap();
        dag.add_node(make_costed_node(2, 40)).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        // Total cost = 70, limit = 70 -> OK (not strictly greater)
        assert!(scheduler.enforce_cost_budget(&dag).is_ok());
    }

    #[test]
    fn test_enforce_cost_budget_nodes_without_latency() {
        let scheduler = DataflowScheduler::new(SchedulerConfig::new().with_max_cost(10));

        let mut dag = ExecutionDag::new();
        // Nodes without estimated_latency contribute 0 cost
        dag.add_node(make_node(1)).unwrap();
        dag.add_node(make_node(2)).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        assert!(scheduler.enforce_cost_budget(&dag).is_ok());
    }

    #[test]
    fn test_enforce_cost_budget_mixed_latency() {
        let scheduler = DataflowScheduler::new(SchedulerConfig::new().with_max_cost(100));

        let mut dag = ExecutionDag::new();
        dag.add_node(make_costed_node(1, 60)).unwrap();
        dag.add_node(make_node(2)).unwrap(); // 0 cost
        dag.add_node(make_costed_node(3, 50)).unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        // Total = 60 + 0 + 50 = 110, limit = 100 -> error
        let result = scheduler.enforce_cost_budget(&dag);
        assert!(result.is_err());
    }

    // ── DataflowScheduler construction tests ───────────────────────────

    #[test]
    fn test_dataflow_scheduler_creation() {
        let config = SchedulerConfig::new().with_max_concurrency(4);
        let scheduler = DataflowScheduler::new(config);
        assert_eq!(scheduler.config.max_concurrency, 4);
    }

    #[test]
    fn test_dataflow_scheduler_default_config() {
        let scheduler = DataflowScheduler::new(SchedulerConfig::default());
        assert!(scheduler.config.max_concurrency > 0);
        assert!(scheduler.config.validate().is_ok());
    }
}
