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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{
        MOCK_AGENT_NAME, MOCK_AGENT_PROFILE, MOCK_MODEL_NAME, MOCK_SESSION_ID, MOCK_STOP_REASON,
    };
    use apxm_core::types::CommunicateProtocol;
    use apxm_core::types::{AISOperationType, SpawnedProcessKind};
    use std::path::Path;

    fn documented_example_tracker() -> GraphMetricsTracker {
        let tracker = GraphMetricsTracker::new();
        tracker.record_operation(OperationMetric {
            node_id: 1,
            op_type: AISOperationType::SpawnAgent,
            duration_ms: 12,
            success: true,
        });
        tracker.record_spawn(ProcessSpawnMetric {
            node_id: 1,
            agent_name: MOCK_AGENT_NAME.to_string(),
            process_id: Some("process-1".to_string()),
            parent_process_id: None,
            profile: Some(MOCK_AGENT_PROFILE.to_string()),
            process_kind: SpawnedProcessKind::External,
            duration_ms: 12,
            success: true,
            error: None,
        });
        tracker.record_operation(OperationMetric {
            node_id: 2,
            op_type: AISOperationType::Communicate,
            duration_ms: 34,
            success: true,
        });
        tracker.record_turn(ProcessPromptMetric {
            node_id: 2,
            agent_name: MOCK_AGENT_NAME.to_string(),
            process_id: "process-1".to_string(),
            protocol: CommunicateProtocol::Acp.as_str().to_string(),
            session_id: Some(MOCK_SESSION_ID.to_string()),
            turn: Some(1),
            model: Some(MOCK_MODEL_NAME.to_string()),
            stop_reason: Some(MOCK_STOP_REASON.to_string()),
            duration_ms: 34,
            input_tokens: Some(18),
            output_tokens: Some(7),
            response_bytes: 29,
            success: true,
            error: None,
        });
        tracker.record_operation(OperationMetric {
            node_id: 3,
            op_type: AISOperationType::Print,
            duration_ms: 3,
            success: true,
        });
        tracker
    }

    #[test]
    fn snapshots_graph_node_and_aggregate_hierarchy() {
        let tracker = documented_example_tracker();
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.graph.operation.attempts, 3);
        assert_eq!(snapshot.graph.processes.spawn_count, 1);
        assert_eq!(snapshot.graph.processes.prompt_turns, 1);
        assert_eq!(snapshot.graph.processes.total_tokens, 25);
        assert_eq!(snapshot.nodes[&2].processes.totals.input_tokens, 18);
        assert_eq!(
            snapshot.aggregates.by_agent[MOCK_AGENT_NAME].response_bytes,
            29
        );
    }

    #[test]
    fn documented_metrics_example_matches_fixture() {
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("examples/metrics/expected_graph_metrics.json");
        let expected: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(fixture_path).expect("fixture"))
                .expect("fixture json");
        let actual = documented_example_tracker().snapshot().to_json();
        assert_eq!(actual, expected);
    }
}
