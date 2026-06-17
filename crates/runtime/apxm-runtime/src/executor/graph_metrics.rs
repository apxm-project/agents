//! Runtime collection for provider-neutral graph metrics.
//!
//! The collector stores node-owned records first. Graph and aggregate views are
//! derived in `snapshot()` so the metrics hierarchy stays consistent.

use std::collections::BTreeMap;

use apxm_core::types::{
    GraphMetricAggregates, GraphMetricTotals, GraphMetricsSnapshot, NodeMetrics, OperationMetric,
    ProcessMetricTotals, ProcessPromptMetric, ProcessSpawnMetric,
};
use parking_lot::RwLock;

/// Thread-safe collector shared across child execution contexts.
#[derive(Debug, Default)]
pub struct GraphMetricsTracker {
    nodes: RwLock<BTreeMap<u64, NodeMetrics>>,
}

impl GraphMetricsTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_operation(&self, metric: OperationMetric) {
        self.nodes
            .write()
            .entry(metric.node_id)
            .or_insert_with(|| NodeMetrics::new(metric.node_id))
            .record_operation(metric);
    }

    pub fn record_spawn(&self, metric: ProcessSpawnMetric) {
        self.nodes
            .write()
            .entry(metric.node_id)
            .or_insert_with(|| NodeMetrics::new(metric.node_id))
            .record_spawn(metric);
    }

    pub fn record_turn(&self, metric: ProcessPromptMetric) {
        self.nodes
            .write()
            .entry(metric.node_id)
            .or_insert_with(|| NodeMetrics::new(metric.node_id))
            .record_turn(metric);
    }

    pub fn get_node(&self, node_id: u64) -> Option<NodeMetrics> {
        self.nodes.read().get(&node_id).cloned()
    }

    pub fn snapshot(&self) -> GraphMetricsSnapshot {
        let nodes = self.nodes.read().clone();
        let mut graph = GraphMetricTotals::default();
        let mut by_agent = BTreeMap::<String, ProcessMetricTotals>::new();

        for node in nodes.values() {
            graph.operation.attempts += node.operation.attempts;
            graph.operation.successes += node.operation.successes;
            graph.operation.failures += node.operation.failures;
            graph.operation.total_duration_ms += node.operation.total_duration_ms;

            for spawn in &node.processes.process_spawns {
                graph.record_spawn(spawn);
                by_agent
                    .entry(spawn.agent_name.clone())
                    .or_default()
                    .record_spawn(spawn);
            }
            for turn in &node.processes.prompt_turns {
                graph.record_turn(turn);
                by_agent
                    .entry(turn.agent_name.clone())
                    .or_default()
                    .record_turn(turn);
            }
        }

        GraphMetricsSnapshot {
            graph,
            nodes,
            aggregates: GraphMetricAggregates { by_agent },
        }
    }
}
