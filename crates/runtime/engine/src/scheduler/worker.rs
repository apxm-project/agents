//! Worker loop for the dataflow scheduler.
//!
//! Workers continuously steal and execute operations until the DAG completes.
//!
//! When the `metrics` feature is enabled, this module instruments key operations
//! to measure scheduler overhead breakdown:
//! - Work stealing
//! - Input collection
//! - Operation dispatch
//! - Token routing

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use apxm_core::types::{AISOperationType, Node, NodeId, Number, OpStatus, TokenId, Value};
use apxm_core::{apxm_op, apxm_token};
use crossbeam_deque::Worker;

use crate::executor::ExecutionContext;
use crate::executor::ExecutorEngine;
use crate::executor::pipeline::{is_blocking_wait_op, is_pure_llm_op};
use crate::observability::WorkerLocalMetrics;
use crate::scheduler::context_view::SchedulerCtx;
use crate::scheduler::internal_state::TokenState;
use crate::scheduler::queue::Priority;
use crate::scheduler::state::SchedulerState;
use crate::timed;
use apxm_core::error::RuntimeError;

const HOL_BLOCK_THRESHOLD_MS: u64 = 50;

/// Main worker loop.
///
/// Each worker thread runs this loop, stealing work and executing operations
/// until the DAG completes or execution is cancelled.
pub async fn worker_loop(
    worker_id: usize,
    local_queue: Worker<NodeId>,
    state: Arc<SchedulerState>,
    executor: Arc<ExecutorEngine>,
    base_ctx: ExecutionContext,
) {
    tracing::debug!(worker = worker_id, "Worker starting");

    // Per-worker accumulator — flushed in batches via `maybe_flush()` and on
    // Drop. Replaces per-op atomic CAS on `state.metrics` for additive counters.
    let mut local_metrics = WorkerLocalMetrics::new(Arc::clone(&state.metrics));

    loop {
        // Check termination conditions
        let remaining = state.remaining.load(Ordering::SeqCst);
        let cancelled = state.is_cancelled();
        tracing::trace!(
            worker = worker_id,
            remaining = remaining,
            cancelled = cancelled,
            "Worker checking termination"
        );
        if remaining == 0 || cancelled {
            tracing::info!(
                worker = worker_id,
                remaining = remaining,
                cancelled = cancelled,
                "Worker exiting"
            );
            break;
        }

        // Try to steal work — only record timing on successful steals to avoid
        // inflating work_stealing_us with idle spin time across all workers.
        let steal_start = std::time::Instant::now();
        let work_ready = state.work_notify.notified();
        tokio::pin!(work_ready);
        // Register the idle waiter before the steal attempt so a concurrent
        // `mark_done()` / ready-node wake cannot be lost between future
        // creation and its first poll.
        work_ready.as_mut().enable();
        let stolen = state.work_stealing.steal_next(&local_queue, worker_id);
        let Some(node_id) = stolen else {
            tracing::trace!(worker = worker_id, "No work found, waiting");
            work_ready.as_mut().await;
            continue;
        };

        // Record work-stealing time only on successful steals
        local_metrics.record_work_stealing(steal_start.elapsed());

        // Get node up-front so we can pick the correct semaphore. If the node
        // is missing, fall through and skip without ever acquiring a permit.
        let Some(node) = state.nodes.get(&node_id).map(|n| n.value().clone()) else {
            continue;
        };

        // Acquire concurrency permits from separate LLM, long-wait, and compute
        // pools so one category cannot starve another. LLM admission bounds
        // individual requests; it does not construct a batch.
        let semaphore = if is_blocking_wait_op(&node) {
            &state.blocking_concurrency
        } else if is_pure_llm_op(&node.op_type) {
            &state.llm_concurrency
        } else {
            &state.concurrency
        };
        let permit = match semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => break, // Cancelled
        };

        // Mark operation as running and emit scheduler-side observability
        // before dispatch so wait time is measured from ready -> running.
        let child_ctx = base_ctx.child();
        op_start(&state, node_id, &node, worker_id);
        emit_scheduler_events(&state, &child_ctx, node_id);

        // Record progress so the watchdog knows the scheduler is alive during
        // long-running operations (e.g. LLM calls).
        state.record_progress();

        apxm_op!(debug,
         worker = worker_id,
         node_id = node_id,
         op_type = ?node.op_type,
         inputs = node.input_tokens.len(),
         "Dispatching operation"
        );

        // Collect inputs (must all be ready) - timed when metrics enabled
        let collected = timed!(local_metrics, record_input_collection, {
            collect_inputs(&state.tokens, &node)
        });
        let Some(inputs) = collected else {
            // Inputs not ready - requeue at lowest priority and continue
            // This shouldn't normally happen since readiness is tracked,
            // but handle gracefully in case of race conditions
            apxm_op!(
                trace,
                worker = worker_id,
                node_id = node_id,
                "Inputs not ready, requeuing"
            );
            state
                .queue
                .push(node_id, crate::scheduler::queue::Priority::Low);
            drop(permit);
            tokio::task::yield_now().await;
            continue;
        };

        // Execute operation with retries
        let outputs = node.output_tokens.clone();

        let outcome = execute_with_retries(
            &state,
            &executor,
            &node,
            &inputs,
            &child_ctx,
            worker_id,
            &mut local_metrics,
        )
        .await;

        // Handle outcome
        match outcome {
            ExecutionOutcome::Success {
                value,
                attempts,
                start_time,
            } => {
                let event = WorkerEvent {
                    state: &state,
                    ctx: &child_ctx,
                    node_id,
                    node: &node,
                    outputs: &outputs,
                    start_time,
                };

                handle_success(&event, value, attempts, &mut local_metrics).await;
            }
            ExecutionOutcome::Failed {
                error,
                attempts,
                start_time,
            } => {
                let event = WorkerEvent {
                    state: &state,
                    ctx: &child_ctx,
                    node_id,
                    node: &node,
                    outputs: &outputs,
                    start_time,
                };

                let should_abort = handle_failure(&event, error, attempts).await;

                if should_abort {
                    drop(permit);
                    break;
                }
            }
            ExecutionOutcome::Parked { wait_key, attempts } => {
                // Yield the lane: register a waker, do NOT finish_one (the node
                // hasn't completed), and drop the permit so a parked wait holds
                // neither a worker nor a concurrency slot. `enter_parked` also
                // releases the execution's cross-execution admission slot on the
                // 0->1 edge, so many parked agents don't occupy admission capacity
                // they aren't using. The node is re-injected when
                // park_registry::wake(wait_key) makes its output token ready.
                state.enter_parked();
                // Narrow park-observability signal: fire ONLY when this
                // park's wait_key is exactly the conversation-loop's
                // session-recv key for this execution's session — not for any
                // other park reason (PAUSE, generic recv-with-url, etc). A
                // caller awaiting `SchedulerState::subscribe_session_parked()`
                // learns "this execution just parked waiting for turn input"
                // without polling and without conflating it with unrelated
                // parks.
                if let Some(session_id) = child_ctx.session_id()
                    && wait_key == crate::scheduler::park_registry::session_input_key(session_id)
                {
                    state.notify_session_parked(session_id.to_string());
                }
                // An AWAIT_INPUT node may re-arm an explicitly declared
                // continuation flow. Other parks use the plain waker.
                let waker = match input_rearm_spec(&node, &child_ctx) {
                    Some(spec) => crate::scheduler::park_registry::ParkWaker::for_rearming_node(
                        Arc::clone(&state),
                        node_id,
                        outputs.clone(),
                        attempts,
                        spec,
                    ),
                    None => crate::scheduler::park_registry::ParkWaker::for_node(
                        Arc::clone(&state),
                        node_id,
                        outputs.clone(),
                        attempts,
                    ),
                };
                if let Err(error) = crate::scheduler::park_registry::register(wait_key, waker) {
                    state.set_first_error(RuntimeError::Scheduler {
                        message: error.to_string(),
                    });
                    state.mark_done();
                    drop(permit);
                    break;
                }
                // Keep the watchdog from flagging this idle-by-design moment.
                state.record_progress();
                apxm_op!(
                    debug,
                    worker = worker_id,
                    node_id = node_id,
                    "Operation parked; lane + permit yielded"
                );
            }
        }

        drop(permit);
        local_metrics.maybe_flush();
    }
}

/// If `node` is an AWAIT_INPUT re-arm with an explicit continuation flow and
/// the execution has a session id, return the generic re-arm specification so
/// its wake splices the continuation plus a fresh input wait.
/// Otherwise `None` (a plain one-shot park).
fn input_rearm_spec(
    node: &std::sync::Arc<apxm_core::types::Node>,
    ctx: &dyn SchedulerCtx,
) -> Option<crate::scheduler::park_registry::RearmSpec> {
    use apxm_core::types::operations::AISOperationType;
    if node.op_type != AISOperationType::AwaitInput {
        return None;
    }
    let attr = |k: &str| node.attributes.get(k).and_then(|v| v.as_str());
    if attr(apxm_core::constants::graph::attrs::REARM) != Some("true") {
        return None;
    }
    let session_id = ctx.session_id()?.to_string();
    let flow_name = attr(apxm_core::constants::graph::attrs::FLOW_NAME)?.to_string();
    let agent_name = attr(apxm_core::constants::graph::attrs::AGENT_NAME)?.to_string();
    let input_name = node
        .attributes
        .get(apxm_core::constants::graph::attrs::INPUT_NAMES)
        .and_then(|value| value.as_array())
        .and_then(|names| names.first())
        .and_then(|value| value.as_str())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)?;
    // Bound the loop to max_iterations (default 100), so re-arm cannot splice
    // continuation and wait nodes without limit.
    let max_resumes = node
        .attributes
        .get(apxm_core::constants::graph::attrs::MAX_ITERATIONS)
        .and_then(|v| v.as_u64())
        .unwrap_or(100);
    Some(crate::scheduler::park_registry::RearmSpec {
        await_node: std::sync::Arc::clone(node),
        agent_name,
        flow_name,
        input_name,
        session_id,
        max_resumes,
    })
}

/// Collect input values for an operation.
///
/// Returns None if any inputs are not ready (shouldn't happen due to readiness tracking).
fn collect_inputs(
    tokens: &dashmap::DashMap<TokenId, TokenState>,
    node: &Node,
) -> Option<Vec<Value>> {
    let mut values = Vec::with_capacity(node.input_tokens.len());

    for &token_id in &node.input_tokens {
        let token = tokens.get(&token_id)?;
        if !token.ready {
            return None;
        }
        values.push(token.value.as_ref()?.clone());
    }

    Some(values)
}

/// Mark an operation as running.
#[inline]
fn op_start(state: &SchedulerState, node_id: NodeId, node: &Node, worker_id: usize) {
    if let Some(mut op_state) = state.op_states.get_mut(&node_id) {
        op_state.status = OpStatus::Running;
        if op_state.started_at.is_none() {
            op_state.started_at = Some(Instant::now());
        }
    }
    state.emit_node_started(node_id, node, worker_id);
}

fn emit_scheduler_events(state: &SchedulerState, ctx: &dyn SchedulerCtx, node_id: NodeId) {
    let Some(emitter) = ctx.event_emitter() else {
        return;
    };

    let chosen_priority = state
        .priorities
        .get(&node_id)
        .map(|priority| *priority)
        .unwrap_or(Priority::Low);

    let delay = {
        let Some(op_state) = state.op_states.get(&node_id) else {
            return;
        };
        let Some(ready_at) = op_state.ready_at else {
            return;
        };
        let started_at = op_state.started_at.unwrap_or_else(Instant::now);
        started_at.saturating_duration_since(ready_at)
    };

    let counts = ready_priority_counts(state, node_id, chosen_priority);
    let total_ready = counts.iter().sum::<usize>().max(1);
    let median_priority = median_priority_from_counts(&counts).unwrap_or(chosen_priority);
    let rank = higher_priority_count(&counts, chosen_priority) + 1;

    emitter.emit_scheduler_decision(
        node_id,
        delay,
        &format!(
            "chosen={}; median={}; rank={}_of_{}",
            chosen_priority.as_str(),
            median_priority.as_str(),
            rank,
            total_ready
        ),
    );

    let wait_ms = delay.as_millis().min(u64::MAX as u128) as u64;
    if wait_ms < HOL_BLOCK_THRESHOLD_MS {
        return;
    }

    let Some((blocker_node, blocker_priority)) = highest_priority_running_node(state, node_id)
    else {
        return;
    };

    emitter.emit_head_of_line_block(
        blocker_node,
        node_id,
        wait_ms,
        &format!(
            "wait_ms={}; threshold_ms={}; blocker={}; blocked={}",
            wait_ms,
            HOL_BLOCK_THRESHOLD_MS,
            blocker_priority.as_str(),
            chosen_priority.as_str()
        ),
    );
}

fn ready_priority_counts(
    state: &SchedulerState,
    chosen_node_id: NodeId,
    chosen_priority: Priority,
) -> [usize; Priority::COUNT] {
    let mut counts = [0usize; Priority::COUNT];
    counts[chosen_priority.as_index()] += 1;

    for entry in state.op_states.iter() {
        if *entry.key() == chosen_node_id || entry.status != OpStatus::Ready {
            continue;
        }

        if let Some(priority) = state.priorities.get(entry.key()).map(|priority| *priority) {
            counts[priority.as_index()] += 1;
        }
    }

    counts
}

fn higher_priority_count(counts: &[usize; Priority::COUNT], chosen_priority: Priority) -> usize {
    counts
        .iter()
        .enumerate()
        .filter(|(index, _)| *index > chosen_priority.as_index())
        .map(|(_, count)| *count)
        .sum()
}

fn median_priority_from_counts(counts: &[usize; Priority::COUNT]) -> Option<Priority> {
    let total: usize = counts.iter().sum();
    if total == 0 {
        return None;
    }

    let median_index = total / 2;
    let ordered = [
        Priority::Low,
        Priority::Normal,
        Priority::High,
        Priority::Critical,
    ];

    let mut seen = 0usize;
    for priority in ordered {
        seen += counts[priority.as_index()];
        if seen > median_index {
            return Some(priority);
        }
    }

    Some(Priority::Critical)
}

fn highest_priority_running_node(
    state: &SchedulerState,
    blocked_node_id: NodeId,
) -> Option<(NodeId, Priority)> {
    let mut best: Option<(NodeId, Priority)> = None;

    for entry in state.op_states.iter() {
        let node_id = *entry.key();
        if node_id == blocked_node_id || entry.status != OpStatus::Running {
            continue;
        }

        let priority = state
            .priorities
            .get(&node_id)
            .map(|priority| *priority)
            .unwrap_or(Priority::Low);

        if best.is_none_or(|(_, best_priority)| priority > best_priority) {
            best = Some((node_id, priority));
        }
    }

    best
}

/// Execution outcome.
enum ExecutionOutcome {
    Success {
        value: Value,
        attempts: u32,
        start_time: Instant,
    },
    Failed {
        error: RuntimeError,
        attempts: u32,
        start_time: Instant,
    },
    /// The handler PARKED on an external event: the worker yields its lane +
    /// permit (no `finish_one`) and registers a waker under `wait_key`; the node
    /// is re-injected when [`crate::scheduler::park_registry::wake`] fires.
    Parked { wait_key: String, attempts: u32 },
}

/// Execute an operation with retry logic.
#[allow(unused_variables)] // worker_id only used in tracing
async fn execute_with_retries(
    state: &SchedulerState,
    executor: &ExecutorEngine,
    node: &Node,
    inputs: &[Value],
    ctx: &ExecutionContext,
    worker_id: usize,
    local_metrics: &mut WorkerLocalMetrics,
) -> ExecutionOutcome {
    let start_time = Instant::now();

    // Record operation start
    record_event(ctx, node.id, node, "op_start", 0, None).await;

    let mut attempt = 0;
    let mut last_error = None;
    let max_retries = max_scheduler_retries_for_node(state.cfg.max_retries, node);

    while attempt <= max_retries {
        // Record scheduling
        #[cfg(feature = "metrics")]
        state.metrics.record_schedule();

        // Execute operation
        #[cfg(feature = "metrics")]
        let exec_start = Instant::now();

        apxm_op!(
            trace,
            worker = worker_id,
            node_id = node.id,
            attempt = attempt + 1,
            "Executing operation"
        );

        let result = if node.op_type == AISOperationType::Checkpoint {
            let snapshot = state.capture_snapshot();
            crate::scheduler::snapshot::with_checkpoint_scheduler_snapshot(
                snapshot,
                executor.execute_with_context(node, inputs.to_vec(), ctx),
            )
            .await
        } else {
            executor
                .execute_with_context(node, inputs.to_vec(), ctx)
                .await
        };

        #[cfg(feature = "metrics")]
        let exec_duration = exec_start.elapsed();

        match result {
            Ok(outcome) => {
                #[cfg(feature = "metrics")]
                {
                    local_metrics.record_completion();
                    state.metrics.release_in_flight();
                    local_metrics.record_execution_time(exec_duration);
                }
                return ExecutionOutcome::Success {
                    value: outcome.value,
                    attempts: attempt + 1,
                    start_time,
                };
            }
            // Not a failure: the handler parked on an external event. Surface it
            // immediately (no retry) so the worker can yield its lane.
            Err(RuntimeError::OperationParked { wait_key }) => {
                return ExecutionOutcome::Parked {
                    wait_key,
                    attempts: attempt + 1,
                };
            }
            Err(error) => {
                #[cfg(feature = "metrics")]
                {
                    local_metrics.record_failure();
                    state.metrics.release_in_flight();
                    local_metrics.record_execution_time(exec_duration);
                }

                apxm_op!(debug,
                 worker = worker_id,
                 node_id = node.id,
                 attempt = attempt + 1,
                 error = %error,
                 "Operation attempt failed"
                );

                last_error = Some(error);
                attempt += 1;

                // Check if we should retry
                if attempt > max_retries {
                    break;
                }

                // Exponential backoff with cap
                let backoff_ms = calculate_backoff(&state.cfg, attempt);
                apxm_op!(
                    trace,
                    worker = worker_id,
                    node_id = node.id,
                    backoff_ms = backoff_ms,
                    "Retry backoff"
                );
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }

    ExecutionOutcome::Failed {
        error: last_error.unwrap_or_else(|| RuntimeError::Scheduler {
            message: "Unknown error".to_string(),
        }),
        attempts: attempt,
        start_time,
    }
}

fn max_scheduler_retries_for_node(configured_max_retries: u32, node: &Node) -> u32 {
    if is_pure_llm_op(&node.op_type) || is_non_retryable_side_effect_op(&node.op_type) {
        0
    } else {
        configured_max_retries
    }
}

fn is_non_retryable_side_effect_op(op: &AISOperationType) -> bool {
    matches!(
        op,
        AISOperationType::InvCap
            | AISOperationType::WorkflowSpawn
            | AISOperationType::SpawnAgent
            | AISOperationType::Communicate
    )
}

/// Calculate exponential backoff delay in milliseconds.
#[inline]
fn calculate_backoff(cfg: &crate::scheduler::config::SchedulerConfig, attempt: u32) -> u64 {
    let pow = (attempt - 1).min(31); // Prevent overflow
    cfg.retry_backoff_ms
        .saturating_mul(2u64.saturating_pow(pow))
        .min(cfg.retry_backoff_max_ms)
}

struct WorkerEvent<'a> {
    state: &'a SchedulerState,
    ctx: &'a dyn SchedulerCtx,
    node_id: NodeId,
    node: &'a Node,
    outputs: &'a [TokenId],
    start_time: Instant,
}

/// Handle successful operation execution.
async fn handle_success(
    event: &WorkerEvent<'_>,
    value: Value,
    attempts: u32,
    #[cfg_attr(not(feature = "metrics"), allow(unused_variables))]
    local_metrics: &mut WorkerLocalMetrics,
) {
    let duration_ms = event.start_time.elapsed().as_millis();

    apxm_op!(debug,
     node_id = event.node_id,
     op_type = ?event.node.op_type,
     duration_ms = duration_ms,
     attempts = attempts,
     output_tokens = event.outputs.len(),
     "Operation completed successfully"
    );

    // Update counters
    event.state.executed.fetch_add(1, Ordering::Relaxed);
    event.state.record_progress();

    mark_operation_completed(event.state, event.node_id, event.node, attempts);

    // A dependent becomes ready only after its producer's terminal event is
    // visible, so execution traces preserve the actual dataflow boundary.
    #[cfg(feature = "metrics")]
    let routing_start = std::time::Instant::now();
    publish_outputs(event.state, event.node_id, event.outputs, value.clone());
    #[cfg(feature = "metrics")]
    local_metrics.record_token_routing(routing_start.elapsed());

    if let Some(emitter) = event.ctx.event_emitter() {
        emitter.emit_node_output(event.node_id, &value);
    }

    // Record success event
    record_event(
        event.ctx,
        event.node_id,
        event.node,
        "op_success",
        attempts,
        Some(duration_ms),
    )
    .await;

    // Decrement remaining count
    finish_one(event.state);
}

/// Handle failed operation execution.
///
/// Returns true if execution should abort.
async fn handle_failure(event: &WorkerEvent<'_>, error: RuntimeError, attempts: u32) -> bool {
    let duration_ms = event.start_time.elapsed().as_millis();

    apxm_op!(error,
     node_id = event.node_id,
     op_type = ?event.node.op_type,
     duration_ms = duration_ms,
     attempts = attempts,
     error = %error,
     "Operation failed"
    );

    // Update counters
    event.state.failed.fetch_add(1, Ordering::Relaxed);

    // Mark operation as failed
    if let Some(mut op_state) = event.state.op_states.get_mut(&event.node_id) {
        op_state.status = OpStatus::Failed;
        op_state.retries = attempts;
        op_state.last_error = Some(error.to_string());
        op_state.finished_at = Some(Instant::now());
    }
    event
        .state
        .emit_node_finished(event.node_id, event.node, attempts);

    // Record failure event
    record_event(
        event.ctx,
        event.node_id,
        event.node,
        "op_failure",
        attempts,
        Some(duration_ms),
    )
    .await;

    // Set first error
    event
        .state
        .set_first_error(RuntimeError::SchedulerRetryExhausted {
            node_id: event.node_id,
            reason: error.to_string(),
        });

    // Check for fallback value
    if let Some(fallback) = event.node.attributes.get("fallback").cloned() {
        apxm_op!(info, node_id = event.node_id, "Using fallback value");
        // Use fallback value instead of failing
        publish_outputs(event.state, event.node_id, event.outputs, fallback);
        finish_one(event.state);
        false // Don't abort
    } else {
        // No fallback - abort execution
        apxm_op!(
            warn,
            node_id = event.node_id,
            "No fallback, aborting execution"
        );
        event.state.mark_done();
        true // Abort
    }
}

/// Publish output tokens and propagate readiness to downstream consumers.
///
/// If a token is marked as delegated by this node, we skip publishing
/// (a spliced sub-DAG will produce the actual value).
fn publish_outputs(state: &SchedulerState, node_id: u64, outputs: &[TokenId], value: Value) {
    for &token_id in outputs {
        // Fast path: check sparse delegation set (almost always empty)
        if state.delegated_tokens.contains(&(node_id, token_id)) {
            apxm_token!(
                trace,
                token_id = token_id,
                delegator = node_id,
                "Token delegated by this node; skipping publish"
            );
            continue;
        }

        // Mark token as ready with value
        if let Some(mut token) = state.tokens.get_mut(&token_id) {
            if token.ready {
                apxm_token!(
                    trace,
                    token_id = token_id,
                    "Token already ready; skipping duplicate publish"
                );
                continue;
            }
            token.ready = true;
            token.value = Some(value.clone());
        } else {
            let mut token_state = TokenState::new();
            token_state.ready = true;
            token_state.value = Some(value.clone());
            state.tokens.insert(token_id, token_state);
        }

        apxm_token!(trace, token_id = token_id, "Token produced and ready");

        // Propagate readiness to consumers
        let ready_nodes = state.ready_set.on_token_ready(
            token_id,
            &state.tokens,
            &state.priorities,
            &state.op_states,
            &state.queue,
        );
        if let Ok(ready_nodes) = ready_nodes {
            state.emit_node_ready_batch(&ready_nodes);
            if !ready_nodes.is_empty() {
                state.work_notify.notify_waiters();
            }
        }
    }
}

/// Mark a node terminal before any dependent can observe its output token.
fn mark_operation_completed(state: &SchedulerState, node_id: NodeId, node: &Node, attempts: u32) {
    if let Some(mut op_state) = state.op_states.get_mut(&node_id) {
        op_state.status = OpStatus::Completed;
        op_state.finished_at = Some(Instant::now());
    }
    state.emit_node_finished(node_id, node, attempts);
}

/// Decrement remaining count and notify if complete.
///
/// Use the previous atomic value returned by fetch_sub to avoid underflow
/// and reliably detect the transition to zero.
#[inline]
fn finish_one(state: &SchedulerState) {
    state.finish_one();
}

/// Record an episodic memory event.
async fn record_event(
    ctx: &dyn SchedulerCtx,
    node_id: NodeId,
    node: &Node,
    event_type: &str,
    attempts: u32,
    duration_ms: Option<u128>,
) {
    let mut fields = vec![
        (
            "node_id".to_string(),
            Value::Number(Number::from(node_id as i64)),
        ),
        (
            "op".to_string(),
            Value::String(format!("{:?}", node.op_type)),
        ),
        (
            "attempts".to_string(),
            Value::Number(Number::from(attempts as i64)),
        ),
    ];

    if let Some(duration) = duration_ms {
        let clamped = duration.min(i64::MAX as u128) as i64;
        fields.push((
            "duration_ms".to_string(),
            Value::Number(Number::from(clamped)),
        ));
    }

    let _ = ctx
        .memory()
        .record_episodic_event(
            ctx.execution_id().to_string(),
            event_type,
            Value::Object(fields.into_iter().collect()),
            Some(node_id),
            None, // session_dir not available in worker context
        )
        .await;
}

#[cfg(test)]
mod tests {
    //! Worker trace tests cover terminal-to-readiness ordering.

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::{DependencyType, Edge, ExecutionDag};

    use super::*;
    use crate::executor::hooks::{
        ExecutionHook, ExecutionHookContext, NodeFinishedEvent, NodeReadyEvent, NodeStartedEvent,
    };
    use crate::observability::MetricsCollector;
    use crate::scheduler::config::SchedulerConfig;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TraceEvent {
        Ready(NodeId),
        Started(NodeId),
        Finished(NodeId),
    }

    #[derive(Default)]
    struct TraceHook {
        events: Mutex<Vec<TraceEvent>>,
    }

    impl TraceHook {
        fn clear(&self) {
            self.events.lock().expect("trace lock").clear();
        }

        fn events(&self) -> Vec<TraceEvent> {
            self.events.lock().expect("trace lock").clone()
        }
    }

    impl ExecutionHook for TraceHook {
        fn on_node_ready(&self, event: &NodeReadyEvent) {
            self.events
                .lock()
                .expect("trace lock")
                .push(TraceEvent::Ready(event.node_id));
        }

        fn on_node_started(&self, event: &NodeStartedEvent) {
            self.events
                .lock()
                .expect("trace lock")
                .push(TraceEvent::Started(event.node_id));
        }

        fn on_node_finished(&self, event: &NodeFinishedEvent) {
            self.events
                .lock()
                .expect("trace lock")
                .push(TraceEvent::Finished(event.node_id));
        }
    }

    fn node(id: NodeId, inputs: Vec<TokenId>, outputs: Vec<TokenId>) -> Node {
        Node {
            id,
            op_type: AISOperationType::Nop,
            attributes: HashMap::new(),
            input_tokens: inputs,
            output_tokens: outputs,
            metadata: NodeMetadata::default(),
        }
    }

    fn two_node_dag() -> ExecutionDag {
        let mut dag = ExecutionDag::new();
        dag.add_node(node(1, Vec::new(), vec![10]))
            .expect("source node");
        dag.add_node(node(2, vec![10], vec![20]))
            .expect("dependent node");
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .expect("data edge");
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();
        dag
    }

    #[test]
    fn trace_finishes_a_producer_before_releasing_its_dependent() {
        let trace = Arc::new(TraceHook::default());
        let hook: Arc<dyn ExecutionHook> = trace.clone();
        let hooks = ExecutionHookContext::new("execution", "graph", vec![hook]);
        let (state, _workers) = SchedulerState::new_with_hooks(
            two_node_dag(),
            SchedulerConfig::new()
                .with_max_concurrency(1)
                .with_max_inflight(1)
                .with_llm_inflight(1),
            Arc::new(MetricsCollector::new()),
            Instant::now(),
            Vec::new(),
            hooks,
        )
        .expect("scheduler state");
        trace.clear();

        let source = state.nodes.get(&1).expect("source node");
        op_start(&state, 1, source.value().as_ref(), 0);
        mark_operation_completed(&state, 1, source.value().as_ref(), 1);
        publish_outputs(&state, 1, &[10], Value::String("source output".to_string()));

        assert_eq!(
            trace.events(),
            vec![
                TraceEvent::Started(1),
                TraceEvent::Finished(1),
                TraceEvent::Ready(2),
            ]
        );
    }

    #[test]
    fn capability_effects_never_receive_scheduler_retries() {
        let effect = Node::new(1, AISOperationType::InvCap);
        let inert = Node::new(2, AISOperationType::Nop);

        assert_eq!(
            max_scheduler_retries_for_node(3, &effect),
            0,
            "a lost external-effect response is outcome-unknown, not retryable",
        );
        assert_eq!(
            max_scheduler_retries_for_node(3, &inert),
            3,
            "ordinary data-only nodes retain the configured retry policy",
        );
    }
}
