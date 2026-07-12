//! Serializable scheduler-state snapshots.
//!
//! This module deliberately projects the live scheduler internals into stable
//! data-transfer structs instead of serializing internal types that contain
//! process-local state such as `Instant`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use apxm_core::types::{ExecutionDag, NodeId, OpStatus, TokenId, Value};
use crossbeam_deque::Worker;
use serde::{Deserialize, Serialize};

use crate::observability::MetricsCollector;
use crate::scheduler::config::SchedulerConfig;
use crate::scheduler::internal_state::{ExecutionFrame, PromiseState, TokenState};
use crate::scheduler::queue::Priority;
use crate::scheduler::replay::ReplaySeed;
use crate::scheduler::state::SchedulerState;

type RuntimeResult<T> = Result<T, apxm_core::error::RuntimeError>;

pub const SCHEDULER_SNAPSHOT_VERSION: u32 = 1;
/// Reason string once [`SchedulerState::restore`] is implemented and tested.
/// Restore rehydrates bookkeeping (tokens/ops/pending-inputs/promises/
/// execution-stack/delegated-tokens) against a freshly recompiled DAG; it does
/// NOT resume a node that was captured `Running` mid-handler — that in-flight
/// work is gone with the worker that was doing it, so such a node is restored
/// `Ready` for an idempotent re-execution rather than falsely reported as
/// still `Running` with nothing driving it forward.
const REPLAY_SUPPORTED_NOTE: &str = "restore rehydrates tokens/ops/pending-inputs/promises/execution-stack/delegated-tokens against a recompiled DAG; nodes captured Running are restored Ready (re-executed, not resumed mid-handler) since no in-flight worker state survives a process restart";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchedulerSnapshot {
    pub version: u32,
    pub execution_id: Option<String>,
    pub graph_id: Option<String>,
    pub elapsed_ms: u64,
    pub scheduler_config: SchedulerConfig,
    pub counters: SchedulerSnapshotCounters,
    pub tokens: Vec<SchedulerSnapshotToken>,
    pub ops: Vec<SchedulerSnapshotOp>,
    pub pending_inputs: Vec<SchedulerSnapshotPendingInput>,
    pub promises: Vec<SchedulerSnapshotPromise>,
    pub execution_stack: Vec<SchedulerSnapshotExecutionFrame>,
    pub delegated_tokens: Vec<SchedulerSnapshotDelegatedToken>,
    pub node_output_map: Vec<SchedulerSnapshotNodeOutputs>,
    pub replay_supported: bool,
    pub replay_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerSnapshotCounters {
    pub executed_nodes: usize,
    pub failed_nodes: usize,
    pub remaining_nodes: usize,
    pub ready_queue_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchedulerSnapshotToken {
    pub token_id: TokenId,
    pub ready: bool,
    pub value: Option<Value>,
    pub consumers: Vec<NodeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerSnapshotOp {
    pub node_id: NodeId,
    pub status: OpStatus,
    pub retries: u32,
    pub last_error: Option<String>,
    pub ready_at_ms: Option<u64>,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub priority: String,
    pub input_tokens: Vec<TokenId>,
    pub output_tokens: Vec<TokenId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerSnapshotPendingInput {
    pub node_id: NodeId,
    pub pending_inputs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchedulerSnapshotPromise {
    pub token_id: TokenId,
    pub target_agent: String,
    pub target_flow: String,
    pub created_at_ms: u64,
    pub resolved: bool,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerSnapshotExecutionFrame {
    pub execution_id: String,
    pub flow_name: String,
    pub parent_promise: Option<TokenId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerSnapshotDelegatedToken {
    pub node_id: NodeId,
    pub token_id: TokenId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerSnapshotNodeOutputs {
    pub node_id: NodeId,
    pub output_tokens: Vec<TokenId>,
}

impl SchedulerState {
    /// Capture a stable read-only projection of the current scheduler state.
    ///
    /// The returned structure is suitable for persistence and observability, but
    /// it is intentionally marked as non-replayable until a restore path can
    /// rehydrate queues, backend state, and side-effect safety.
    pub fn capture_snapshot(&self) -> SchedulerSnapshot {
        let start = self.start;

        SchedulerSnapshot {
            version: SCHEDULER_SNAPSHOT_VERSION,
            execution_id: empty_to_none(self.hooks.execution_id()),
            graph_id: empty_to_none(self.hooks.graph_id()),
            elapsed_ms: elapsed_ms_since(start, Instant::now()),
            scheduler_config: self.cfg.clone(),
            counters: SchedulerSnapshotCounters {
                executed_nodes: self.executed.load(std::sync::atomic::Ordering::Relaxed),
                failed_nodes: self.failed.load(std::sync::atomic::Ordering::Relaxed),
                remaining_nodes: self.remaining.load(std::sync::atomic::Ordering::SeqCst),
                ready_queue_len: self.queue.len(),
            },
            tokens: self.snapshot_tokens(),
            ops: self.snapshot_ops(start),
            pending_inputs: self.snapshot_pending_inputs(),
            promises: self.snapshot_promises(start),
            execution_stack: self.snapshot_execution_stack(),
            delegated_tokens: self.snapshot_delegated_tokens(),
            node_output_map: self.snapshot_node_output_map(),
            replay_supported: true,
            replay_notes: vec![REPLAY_SUPPORTED_NOTE.to_string()],
        }
    }

    /// Rehydrate a [`SchedulerState`] from a captured [`SchedulerSnapshot`]
    /// against a freshly recompiled `dag` (same node/token ids as the
    /// original run — the deterministic skill-compilation case, matching the
    /// precondition [`crate::scheduler::replay::ReplaySeed`] already relies
    /// on).
    ///
    /// Reuses the proven partial-replay seeding path
    /// ([`SchedulerState::new_with_replay`]): nodes captured `Completed` are
    /// treated as pre-completed (never re-enqueued) and every `ready` token's
    /// captured value seeds the readiness computation, so pending-input
    /// counts for not-yet-run nodes land exactly where they were pre-capture.
    /// Token/promise/execution-stack/delegated-token state is then overlaid
    /// verbatim from the snapshot so restored state matches the captured
    /// state field-for-field (see [`REPLAY_SUPPORTED_NOTE`] for the one
    /// documented exception: a node captured `Running` is restored `Ready`,
    /// not `Running`, since no in-flight worker survives a process restart).
    pub fn restore(
        snapshot: &SchedulerSnapshot,
        dag: Arc<ExecutionDag>,
        cfg: SchedulerConfig,
        metrics: Arc<MetricsCollector>,
    ) -> RuntimeResult<(Arc<SchedulerState>, Vec<Worker<NodeId>>)> {
        let start = Instant::now();
        let hooks = crate::executor::hooks::ExecutionHookContext::default();

        let completed_nodes: HashSet<NodeId> = snapshot
            .ops
            .iter()
            .filter(|op| op.status == OpStatus::Completed)
            .map(|op| op.node_id)
            .collect();
        let seed_tokens: HashMap<TokenId, Value> = snapshot
            .tokens
            .iter()
            .filter(|token| token.ready)
            .filter_map(|token| token.value.clone().map(|value| (token.token_id, value)))
            .collect();
        let seed = ReplaySeed {
            from_node: 0,
            replayed_nodes: HashSet::new(),
            completed_nodes,
            seed_tokens,
        };

        let (state, workers) = SchedulerState::new_with_replay(
            (*dag).clone(),
            cfg,
            metrics,
            start,
            vec![],
            hooks,
            Some(&seed),
        )?;

        // Overlay the full token map verbatim (ready flag, value, consumers):
        // covers both statically-declared DAG tokens (already correct from the
        // seed above) and dynamically-allocated promise tokens (never part of
        // `dag.nodes`, so `materialize_graph_state` never created them).
        for token in &snapshot.tokens {
            state.tokens.insert(
                token.token_id,
                TokenState {
                    ready: token.ready,
                    value: token.value.clone(),
                    consumers: token.consumers.clone(),
                },
            );
        }

        // Overlay op bookkeeping the readiness recompute above cannot derive:
        // retry counts and the last error message. Completed nodes already got
        // their status from the seed; a captured `Running` node is restored
        // `Ready` (see `REPLAY_SUPPORTED_NOTE`) rather than left `Running` with
        // no worker ever attached to it again.
        for op in &snapshot.ops {
            if let Some(mut entry) = state.op_states.get_mut(&op.node_id) {
                entry.retries = op.retries;
                entry.last_error = op.last_error.clone();
                entry.status = match op.status {
                    OpStatus::Running => OpStatus::Ready,
                    other => other,
                };
            }
        }

        // Promises: `created_at` cannot survive a process boundary as an
        // `Instant`; re-base it to this restore's `start` (callers needing
        // wall-clock promise age should use the snapshot's `created_at_ms`).
        for promise in &snapshot.promises {
            state.pending_promises.insert(
                promise.token_id,
                PromiseState {
                    target_agent: promise.target_agent.clone(),
                    target_flow: promise.target_flow.clone(),
                    created_at: start,
                    resolved: promise.resolved,
                    value: promise.value.clone(),
                },
            );
        }

        // Execution stack + delegated tokens restored verbatim.
        {
            let mut stack = state.execution_stack.lock();
            *stack = snapshot
                .execution_stack
                .iter()
                .map(|frame| ExecutionFrame {
                    execution_id: frame.execution_id.clone(),
                    flow_name: frame.flow_name.clone(),
                    parent_promise: frame.parent_promise,
                })
                .collect();
        }
        for delegated in &snapshot.delegated_tokens {
            state
                .delegated_tokens
                .insert((delegated.node_id, delegated.token_id));
        }

        Ok((Arc::new(state), workers))
    }

    fn snapshot_tokens(&self) -> Vec<SchedulerSnapshotToken> {
        let mut tokens: Vec<_> = self
            .tokens
            .iter()
            .map(|entry| {
                let mut consumers = entry.consumers.clone();
                consumers.sort_unstable();
                SchedulerSnapshotToken {
                    token_id: *entry.key(),
                    ready: entry.ready,
                    value: entry.value.clone(),
                    consumers,
                }
            })
            .collect();
        tokens.sort_unstable_by_key(|token| token.token_id);
        tokens
    }

    fn snapshot_ops(&self, start: Instant) -> Vec<SchedulerSnapshotOp> {
        let mut ops: Vec<_> = self
            .op_states
            .iter()
            .map(|entry| {
                let node_id = *entry.key();
                let priority = self
                    .priorities
                    .get(&node_id)
                    .map(|priority| priority.as_str().to_string())
                    .unwrap_or_else(|| Priority::Normal.as_str().to_string());
                let (input_tokens, output_tokens) = self
                    .nodes
                    .get(&node_id)
                    .map(|node| (node.input_tokens.clone(), node.output_tokens.clone()))
                    .unwrap_or_default();

                SchedulerSnapshotOp {
                    node_id,
                    status: entry.status,
                    retries: entry.retries,
                    last_error: entry.last_error.clone(),
                    ready_at_ms: entry
                        .ready_at
                        .map(|instant| elapsed_ms_since(start, instant)),
                    started_at_ms: entry
                        .started_at
                        .map(|instant| elapsed_ms_since(start, instant)),
                    finished_at_ms: entry
                        .finished_at
                        .map(|instant| elapsed_ms_since(start, instant)),
                    priority,
                    input_tokens,
                    output_tokens,
                }
            })
            .collect();
        ops.sort_unstable_by_key(|op| op.node_id);
        ops
    }

    fn snapshot_pending_inputs(&self) -> Vec<SchedulerSnapshotPendingInput> {
        self.ready_set
            .snapshot()
            .into_iter()
            .map(|(node_id, pending_inputs)| SchedulerSnapshotPendingInput {
                node_id,
                pending_inputs,
            })
            .collect()
    }

    fn snapshot_promises(&self, start: Instant) -> Vec<SchedulerSnapshotPromise> {
        let mut promises: Vec<_> = self
            .pending_promises
            .iter()
            .map(|entry| SchedulerSnapshotPromise {
                token_id: *entry.key(),
                target_agent: entry.target_agent.clone(),
                target_flow: entry.target_flow.clone(),
                created_at_ms: elapsed_ms_since(start, entry.created_at),
                resolved: entry.resolved,
                value: entry.value.clone(),
            })
            .collect();
        promises.sort_unstable_by_key(|promise| promise.token_id);
        promises
    }

    fn snapshot_execution_stack(&self) -> Vec<SchedulerSnapshotExecutionFrame> {
        self.execution_stack
            .lock()
            .iter()
            .map(|frame| SchedulerSnapshotExecutionFrame {
                execution_id: frame.execution_id.clone(),
                flow_name: frame.flow_name.clone(),
                parent_promise: frame.parent_promise,
            })
            .collect()
    }

    fn snapshot_delegated_tokens(&self) -> Vec<SchedulerSnapshotDelegatedToken> {
        let mut delegated_tokens: Vec<_> = self
            .delegated_tokens
            .iter()
            .map(|entry| SchedulerSnapshotDelegatedToken {
                node_id: entry.0,
                token_id: entry.1,
            })
            .collect();
        delegated_tokens.sort_unstable_by_key(|entry| (entry.node_id, entry.token_id));
        delegated_tokens
    }

    fn snapshot_node_output_map(&self) -> Vec<SchedulerSnapshotNodeOutputs> {
        let mut node_output_map: Vec<_> = self
            .node_output_map()
            .into_iter()
            .map(|(node_id, mut output_tokens)| {
                output_tokens.sort_unstable();
                SchedulerSnapshotNodeOutputs {
                    node_id,
                    output_tokens,
                }
            })
            .collect();
        node_output_map.sort_unstable_by_key(|entry| entry.node_id);
        node_output_map
    }
}

fn empty_to_none(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn elapsed_ms_since(start: Instant, instant: Instant) -> u64 {
    instant
        .saturating_duration_since(start)
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::MetricsCollector;
    use crate::scheduler::config::SchedulerConfig;
    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::{DependencyType, Edge, Node};
    use std::collections::HashMap as StdHashMap;

    fn test_config() -> SchedulerConfig {
        SchedulerConfig::new()
            .with_max_concurrency(2)
            .with_max_inflight(4)
    }

    fn make_node(id: NodeId, input_tokens: Vec<TokenId>, output_tokens: Vec<TokenId>) -> Node {
        Node {
            id,
            op_type: AISOperationType::Nop,
            attributes: StdHashMap::new(),
            input_tokens,
            output_tokens,
            metadata: NodeMetadata::default(),
        }
    }

    /// 1 --t10--> 2 --t20--> 3 (t30 exit).
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

    /// Positive/recovery: capture a snapshot mid-execution (one node completed,
    /// one captured `Running` with retry/error history, one still blocked on an
    /// unready input, plus one unresolved promise), restore it into a fresh
    /// `SchedulerState` built against the same recompiled DAG, and assert the
    /// token/op/promise state matches pre-capture (module the one documented
    /// Running->Ready transform: no in-flight worker survives a restart, so a
    /// captured `Running` node is restored `Ready` for re-execution, not left
    /// `Running` with nothing driving it — see `REPLAY_SUPPORTED_NOTE`).
    #[test]
    fn restore_roundtrip_tokens_ops_promises() {
        let dag = three_node_chain();
        let metrics = Arc::new(MetricsCollector::new());
        let (state, _workers) =
            SchedulerState::new(dag.clone(), test_config(), metrics, Instant::now(), vec![])
                .unwrap();

        // Node 1: completed, published its output token.
        {
            let mut op = state.op_states.get_mut(&1).unwrap();
            op.status = OpStatus::Completed;
        }
        {
            let mut tok = state.tokens.get_mut(&10).unwrap();
            tok.ready = true;
            tok.value = Some(Value::String("n1-output".to_string()));
        }

        // Node 2: mid-handler when the snapshot was captured (simulating a
        // kill -9 while a worker was running it); it had already retried once
        // with a transient error.
        {
            let mut op = state.op_states.get_mut(&2).unwrap();
            op.status = OpStatus::Running;
            op.retries = 2;
            op.last_error = Some("transient timeout".to_string());
        }

        // Node 3: still blocked — its input token (20) is not ready — models
        // a genuinely "pending" op at capture time.
        assert!(!state.tokens.get(&20).unwrap().ready);

        // An unresolved flow-call promise (dynamically-allocated token, not
        // part of the static DAG at all).
        let promise_token = state.create_promise_token("agent_x".to_string(), "flow_y".to_string());

        let snapshot = state.capture_snapshot();
        assert!(
            snapshot.replay_supported,
            "restore is implemented and tested; snapshot must not under-claim"
        );

        let metrics2 = Arc::new(MetricsCollector::new());
        let (restored, _workers2) =
            SchedulerState::restore(&snapshot, Arc::new(dag), test_config(), metrics2)
                .expect("restore succeeds against the recompiled DAG");

        // Tokens: node 1's output survived verbatim; node 2's output is still
        // not ready (never produced); the promise token round-trips too.
        let restored_t10 = restored.tokens.get(&10).unwrap();
        assert!(restored_t10.ready);
        assert_eq!(
            restored_t10.value,
            Some(Value::String("n1-output".to_string()))
        );
        let restored_t20 = restored.tokens.get(&20).unwrap();
        assert!(!restored_t20.ready);
        assert!(restored.tokens.contains_key(&promise_token));

        // Ops: node 1 stays Completed; node 2's retry/error history survives
        // even though its status is intentionally re-mapped Running -> Ready;
        // node 3 is still Pending (blocked on token 20).
        assert_eq!(
            restored.op_states.get(&1).unwrap().status,
            OpStatus::Completed
        );
        let restored_op2 = restored.op_states.get(&2).unwrap();
        assert_eq!(
            restored_op2.status,
            OpStatus::Ready,
            "captured Running is restored Ready (re-executed, not resumed mid-handler)"
        );
        assert_eq!(restored_op2.retries, 2, "retry count survives restore");
        assert_eq!(
            restored_op2.last_error.as_deref(),
            Some("transient timeout"),
            "last error survives restore"
        );
        assert_eq!(
            restored.op_states.get(&3).unwrap().status,
            OpStatus::Pending
        );

        // Promise: unresolved, target agent/flow preserved.
        let restored_promise = restored.pending_promises.get(&promise_token).unwrap();
        assert!(!restored_promise.resolved);
        assert!(restored_promise.value.is_none());
        assert_eq!(restored_promise.target_agent, "agent_x");
        assert_eq!(restored_promise.target_flow, "flow_y");

        // Regression guard: a naive from-scratch rebuild (ignoring the
        // snapshot entirely) would report node 1 as NOT completed and its
        // output token as not ready — the opposite of what actually happened
        // pre-capture. This is exactly the fail-open shape the ledger test
        // guards against one layer up; assert restore is not a no-op.
        let (naive, _naive_workers) = SchedulerState::new(
            three_node_chain(),
            test_config(),
            Arc::new(MetricsCollector::new()),
            Instant::now(),
            vec![],
        )
        .unwrap();
        assert_ne!(
            naive.op_states.get(&1).unwrap().status,
            OpStatus::Completed,
            "sanity: an unrestored fresh state must NOT already show node 1 completed"
        );
        assert!(!naive.tokens.get(&10).unwrap().ready);
    }
}
