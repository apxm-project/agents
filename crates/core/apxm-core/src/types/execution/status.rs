//! Execution status types.
//!
//! Types for tracking the status of operations during DAG execution.

use std::collections::{HashMap, HashSet};

use crate::types::{ExecutionDag, NodeId};

/// Operation execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum OpStatus {
    Pending,
    Ready,
    Running,
    Completed,
    Failed,
}

/// Status information for a single node.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeStatus {
    pub node_id: NodeId,
    pub status: OpStatus,
    pub retries: u32,
    pub last_error: Option<String>,
    /// Milliseconds since execution start when all dependencies became ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_at_ms: Option<u128>,
    /// Milliseconds since execution start.
    pub started_at_ms: Option<u128>,
    /// Milliseconds since execution start.
    pub finished_at_ms: Option<u128>,
    pub duration_ms: Option<u128>,
    /// Time spent ready but not yet running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_wait_ms: Option<u128>,
    /// Scheduler priority bucket used for admission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<usize>,
}

/// Observed graph timing derived from real runtime node timings.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ObservedGraphMetrics {
    pub critical_path: ObservedCriticalPath,
    pub queue_wait: ObservedQueueWait,
}

/// Runtime critical path computed from completed node durations and DAG edges.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ObservedCriticalPath {
    pub duration_ms: u128,
    pub finish_ms: u128,
    pub node_count: usize,
    pub nodes: Vec<NodeId>,
}

/// Queue-wait rollups from scheduler readiness timestamps.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ObservedQueueWait {
    pub total_ms: u128,
    pub mean_ms: f64,
    pub p95_ms: u128,
    pub max_ms: u128,
    pub critical_path_total_ms: u128,
    pub critical_path_mean_ms: f64,
}

/// Execution statistics for a completed DAG.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionStats {
    pub executed_nodes: usize,
    pub failed_nodes: usize,
    pub duration_ms: u128,
    pub node_statuses: Vec<NodeStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_graph: Option<ObservedGraphMetrics>,
}

impl ExecutionStats {
    pub fn total_nodes(&self) -> usize {
        self.executed_nodes + self.failed_nodes
    }

    /// Returns success rate as a percentage (0.0–100.0). Returns 100.0 when no nodes exist.
    pub fn success_rate(&self) -> f64 {
        let total = self.total_nodes();
        if total == 0 {
            return 100.0;
        }
        (self.executed_nodes as f64 / total as f64) * 100.0
    }

    pub fn is_success(&self) -> bool {
        self.failed_nodes == 0
    }

    /// Compute observed critical-path and queue-wait metrics from the executed
    /// DAG and runtime node statuses.
    pub fn attach_observed_graph_metrics(&mut self, dag: &ExecutionDag) {
        self.observed_graph = ObservedGraphMetrics::from_dag_and_statuses(dag, &self.node_statuses);
    }
}

impl ObservedGraphMetrics {
    pub fn from_dag_and_statuses(dag: &ExecutionDag, statuses: &[NodeStatus]) -> Option<Self> {
        let mut duration_by_node: HashMap<NodeId, u128> = HashMap::new();
        let mut finish_by_node: HashMap<NodeId, u128> = HashMap::new();
        for status in statuses {
            if matches!(status.status, OpStatus::Completed) {
                if let Some(duration) = status.duration_ms {
                    duration_by_node.insert(status.node_id, duration);
                }
                if let Some(finished_at) = status.finished_at_ms {
                    finish_by_node.insert(status.node_id, finished_at);
                }
            }
        }

        if duration_by_node.is_empty() {
            return None;
        }

        let mut predecessors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for edge in &dag.edges {
            predecessors.entry(edge.to).or_default().push(edge.from);
        }

        let mut best_duration: HashMap<NodeId, u128> = HashMap::new();
        let mut previous: HashMap<NodeId, NodeId> = HashMap::new();
        for node_id in topological_order(dag) {
            let Some(node_duration) = duration_by_node.get(&node_id).copied() else {
                continue;
            };
            let mut best_predecessor: Option<(NodeId, u128)> = None;
            for pred in predecessors.get(&node_id).into_iter().flatten() {
                if let Some(pred_duration) = best_duration.get(pred).copied()
                    && best_predecessor
                        .map(|(_, current)| pred_duration > current)
                        .unwrap_or(true)
                {
                    best_predecessor = Some((*pred, pred_duration));
                }
            }
            let upstream_duration = best_predecessor.map(|(_, value)| value).unwrap_or(0);
            if let Some((pred, _)) = best_predecessor {
                previous.insert(node_id, pred);
            }
            best_duration.insert(node_id, upstream_duration + node_duration);
        }

        let (terminal_node, critical_duration) = best_duration
            .iter()
            .max_by_key(|(_, duration)| *duration)
            .map(|(node, duration)| (*node, *duration))?;

        let mut critical_nodes = Vec::new();
        let mut cursor = Some(terminal_node);
        while let Some(node_id) = cursor {
            critical_nodes.push(node_id);
            cursor = previous.get(&node_id).copied();
        }
        critical_nodes.reverse();

        let critical_set: HashSet<NodeId> = critical_nodes.iter().copied().collect();
        let queue_wait_values: Vec<u128> = statuses
            .iter()
            .filter_map(|status| status.queue_wait_ms)
            .collect();
        let critical_queue_wait_values: Vec<u128> = statuses
            .iter()
            .filter(|status| critical_set.contains(&status.node_id))
            .filter_map(|status| status.queue_wait_ms)
            .collect();

        Some(Self {
            critical_path: ObservedCriticalPath {
                duration_ms: critical_duration,
                finish_ms: critical_nodes
                    .iter()
                    .filter_map(|node_id| finish_by_node.get(node_id).copied())
                    .max()
                    .unwrap_or(0),
                node_count: critical_nodes.len(),
                nodes: critical_nodes,
            },
            queue_wait: ObservedQueueWait {
                total_ms: queue_wait_values.iter().sum(),
                mean_ms: mean_u128(&queue_wait_values),
                p95_ms: percentile_u128(queue_wait_values.clone(), 0.95),
                max_ms: queue_wait_values.iter().copied().max().unwrap_or(0),
                critical_path_total_ms: critical_queue_wait_values.iter().sum(),
                critical_path_mean_ms: mean_u128(&critical_queue_wait_values),
            },
        })
    }
}

fn topological_order(dag: &ExecutionDag) -> Vec<NodeId> {
    let mut indegree: HashMap<NodeId, usize> = dag.nodes.iter().map(|node| (node.id, 0)).collect();
    let mut adjacency: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for edge in &dag.edges {
        adjacency.entry(edge.from).or_default().push(edge.to);
        *indegree.entry(edge.to).or_insert(0) += 1;
    }

    let mut ready: Vec<NodeId> = indegree
        .iter()
        .filter_map(|(node_id, degree)| (*degree == 0).then_some(*node_id))
        .collect();
    ready.sort_unstable();

    let mut order = Vec::with_capacity(dag.nodes.len());
    while let Some(node_id) = ready.first().copied() {
        ready.remove(0);
        order.push(node_id);
        if let Some(children) = adjacency.get(&node_id) {
            for child in children {
                if let Some(degree) = indegree.get_mut(child) {
                    *degree = degree.saturating_sub(1);
                    if *degree == 0 {
                        ready.push(*child);
                    }
                }
            }
            ready.sort_unstable();
        }
    }

    if order.len() == dag.nodes.len() {
        order
    } else {
        dag.nodes.iter().map(|node| node.id).collect()
    }
}

fn mean_u128(values: &[u128]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<u128>() as f64 / values.len() as f64
}

fn percentile_u128(mut values: Vec<u128>, percentile: f64) -> u128 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let index = ((values.len() - 1) as f64 * percentile).ceil() as usize;
    values[index.min(values.len() - 1)]
}

