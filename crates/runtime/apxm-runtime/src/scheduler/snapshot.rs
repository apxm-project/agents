//! Serializable scheduler-state snapshots.
//!
//! This module deliberately projects the live scheduler internals into stable
//! data-transfer structs instead of serializing internal types that contain
//! process-local state such as `Instant`.

use std::time::Instant;

use apxm_core::types::{NodeId, OpStatus, TokenId, Value};
use serde::{Deserialize, Serialize};

use crate::scheduler::config::SchedulerConfig;
use crate::scheduler::queue::Priority;
use crate::scheduler::state::SchedulerState;

pub const SCHEDULER_SNAPSHOT_VERSION: u32 = 1;
const REPLAY_UNSUPPORTED_REASON: &str =
    "capture-only scheduler snapshot; restore/replay semantics are not implemented";

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
            replay_supported: false,
            replay_notes: vec![REPLAY_UNSUPPORTED_REASON.to_string()],
        }
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
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use apxm_core::types::execution::{DependencyType, Edge, ExecutionDag, Node, NodeMetadata};
    use apxm_core::types::operations::AISOperationType;

    use crate::executor::ExecutionHookContext;
    use crate::observability::MetricsCollector;
    use crate::scheduler::config::SchedulerConfig;
    use crate::scheduler::internal_state::ExecutionFrame;
    use crate::scheduler::state::SchedulerState;

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

    fn parameterized_two_node_dag() -> ExecutionDag {
        let n1 = make_node(1, vec![50], vec![10]);
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

    fn build_state() -> SchedulerState {
        let metrics = Arc::new(MetricsCollector::new());
        let hooks = ExecutionHookContext::new("exec-snapshot", "graph-snapshot", Vec::new());
        let (state, _) = SchedulerState::new_with_hooks(
            parameterized_two_node_dag(),
            SchedulerConfig::new()
                .with_max_concurrency(2)
                .with_max_inflight(4)
                .with_collect_all_outputs(true),
            metrics,
            Instant::now(),
            vec![Value::String("seed".to_string())],
            hooks,
        )
        .unwrap();
        state
    }

    #[test]
    fn capture_snapshot_projects_ready_pending_and_token_state() {
        let state = build_state();
        let snapshot = state.capture_snapshot();

        assert_eq!(snapshot.version, SCHEDULER_SNAPSHOT_VERSION);
        assert_eq!(snapshot.execution_id.as_deref(), Some("exec-snapshot"));
        assert_eq!(snapshot.graph_id.as_deref(), Some("graph-snapshot"));
        assert!(!snapshot.replay_supported);
        assert_eq!(snapshot.counters.remaining_nodes, 2);

        let token = snapshot
            .tokens
            .iter()
            .find(|token| token.token_id == 50)
            .expect("input token should be captured");
        assert!(token.ready);
        assert_eq!(token.value, Some(Value::String("seed".to_string())));
        assert_eq!(token.consumers, vec![1]);

        let node_1 = snapshot
            .ops
            .iter()
            .find(|op| op.node_id == 1)
            .expect("node 1 should be captured");
        assert_eq!(node_1.status, OpStatus::Ready);

        let node_2 = snapshot
            .ops
            .iter()
            .find(|op| op.node_id == 2)
            .expect("node 2 should be captured");
        assert_eq!(node_2.status, OpStatus::Pending);

        assert_eq!(
            snapshot.pending_inputs,
            vec![SchedulerSnapshotPendingInput {
                node_id: 2,
                pending_inputs: 1,
            }]
        );
        assert_eq!(
            snapshot.node_output_map,
            vec![
                SchedulerSnapshotNodeOutputs {
                    node_id: 1,
                    output_tokens: vec![10],
                },
                SchedulerSnapshotNodeOutputs {
                    node_id: 2,
                    output_tokens: vec![20],
                },
            ]
        );

        serde_json::to_value(&snapshot).expect("scheduler snapshot should be JSON serializable");
    }

    #[test]
    fn capture_snapshot_projects_runtime_progress_and_auxiliary_state() {
        let state = build_state();
        let start = state.start;

        state.executed.store(1, Ordering::Relaxed);
        state.failed.store(1, Ordering::Relaxed);
        state.remaining.store(0, Ordering::SeqCst);
        state.delegated_tokens.insert((2, 20));
        state.push_execution_frame(ExecutionFrame {
            execution_id: "child-exec".to_string(),
            flow_name: "child-flow".to_string(),
            parent_promise: Some(1_000_000),
        });
        let promise_token = state.create_promise_token("agent".to_string(), "flow".to_string());
        state
            .resolve_promise(promise_token, Value::String("done".to_string()))
            .unwrap();

        if let Some(mut op) = state.op_states.get_mut(&1) {
            op.status = OpStatus::Running;
            op.started_at = Some(start + Duration::from_millis(10));
        }
        if let Some(mut op) = state.op_states.get_mut(&2) {
            op.status = OpStatus::Failed;
            op.retries = 2;
            op.last_error = Some("boom".to_string());
            op.ready_at = Some(start + Duration::from_millis(20));
            op.started_at = Some(start + Duration::from_millis(30));
            op.finished_at = Some(start + Duration::from_millis(40));
        }

        let snapshot = state.capture_snapshot();
        assert_eq!(snapshot.counters.executed_nodes, 1);
        assert_eq!(snapshot.counters.failed_nodes, 1);
        assert_eq!(snapshot.counters.remaining_nodes, 0);
        assert_eq!(
            snapshot.delegated_tokens,
            vec![SchedulerSnapshotDelegatedToken {
                node_id: 2,
                token_id: 20,
            }]
        );
        assert_eq!(
            snapshot.execution_stack,
            vec![SchedulerSnapshotExecutionFrame {
                execution_id: "child-exec".to_string(),
                flow_name: "child-flow".to_string(),
                parent_promise: Some(1_000_000),
            }]
        );

        let promise = snapshot
            .promises
            .iter()
            .find(|promise| promise.token_id == promise_token)
            .expect("promise should be captured");
        assert!(promise.resolved);
        assert_eq!(promise.value, Some(Value::String("done".to_string())));

        let failed = snapshot
            .ops
            .iter()
            .find(|op| op.node_id == 2)
            .expect("failed op should be captured");
        assert_eq!(failed.status, OpStatus::Failed);
        assert_eq!(failed.retries, 2);
        assert_eq!(failed.last_error.as_deref(), Some("boom"));
        assert_eq!(failed.ready_at_ms, Some(20));
        assert_eq!(failed.started_at_ms, Some(30));
        assert_eq!(failed.finished_at_ms, Some(40));
    }
}
