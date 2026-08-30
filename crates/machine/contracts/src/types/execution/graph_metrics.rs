//! Provider-neutral graph execution metrics.
//!
//! The hierarchy is graph -> node -> runtime activity. Aggregates are derived
//! from node records so APXM can explain which graph node caused each spawned
//! process cost or latency before rolling those numbers up.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::operations::AISOperationType;

/// Runtime process kind for graph node process metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnedProcessKind {
    Local,
    External,
}

/// Aggregate counters for operation execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OperationMetricTotals {
    pub attempts: usize,
    pub successes: usize,
    pub failures: usize,
    pub total_duration_ms: u64,
}

impl OperationMetricTotals {
    pub fn record_operation(&mut self, metric: &OperationMetric) {
        self.attempts += 1;
        self.total_duration_ms += metric.duration_ms;
        if metric.success {
            self.successes += 1;
        } else {
            self.failures += 1;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.attempts == 0
    }
}

/// Aggregate counters for spawned-process activity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessMetricTotals {
    pub spawn_count: usize,
    pub spawn_failures: usize,
    pub prompts: usize,
    pub prompt_failures: usize,
    pub total_spawn_duration_ms: u64,
    pub total_prompt_duration_ms: u64,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub total_tokens: usize,
    pub response_bytes: usize,
}

impl ProcessMetricTotals {
    pub fn record_spawn(&mut self, metric: &ProcessSpawnMetric) {
        self.spawn_count += 1;
        self.total_spawn_duration_ms += metric.duration_ms;
        if !metric.success {
            self.spawn_failures += 1;
        }
    }

    pub fn record_prompt(&mut self, metric: &ProcessPromptMetric) {
        self.prompts += 1;
        self.total_prompt_duration_ms += metric.duration_ms;
        if !metric.success {
            self.prompt_failures += 1;
        }
        if let Some(input) = metric.input_tokens {
            self.input_tokens += input;
        }
        if let Some(output) = metric.output_tokens {
            self.output_tokens += output;
        }
        self.total_tokens = self.input_tokens + self.output_tokens;
        self.response_bytes += metric.response_bytes;
    }

    pub fn is_empty(&self) -> bool {
        self.spawn_count == 0 && self.prompts == 0
    }
}

/// Aggregate counters for a graph, node, or derived aggregate dimension.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphMetricTotals {
    pub operation: OperationMetricTotals,
    pub processes: ProcessMetricTotals,
}

impl GraphMetricTotals {
    pub fn record_operation(&mut self, metric: &OperationMetric) {
        self.operation.record_operation(metric);
    }

    pub fn record_spawn(&mut self, metric: &ProcessSpawnMetric) {
        self.processes.record_spawn(metric);
    }

    pub fn record_prompt(&mut self, metric: &ProcessPromptMetric) {
        self.processes.record_prompt(metric);
    }

    pub fn is_empty(&self) -> bool {
        self.operation.is_empty() && self.processes.is_empty()
    }
}

/// One node operation execution measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationMetric {
    pub node_id: u64,
    pub op_type: AISOperationType,
    pub duration_ms: u64,
    pub success: bool,
}

/// One SPAWN_AGENT lifecycle measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSpawnMetric {
    pub node_id: u64,
    pub agent_name: String,
    pub process_id: Option<String>,
    pub parent_process_id: Option<String>,
    pub profile: Option<String>,
    pub process_kind: SpawnedProcessKind,
    pub duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
}

/// One prompt sent to a spawned agent process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessPromptMetric {
    pub node_id: u64,
    pub agent_name: String,
    pub process_id: String,
    pub protocol: String,
    pub session_id: Option<String>,
    pub prompt_index: Option<u64>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub duration_ms: u64,
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub response_bytes: usize,
    pub success: bool,
    pub error: Option<String>,
}

/// Spawned-process activity owned by a single graph node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeProcessMetrics {
    pub totals: ProcessMetricTotals,
    pub process_spawns: Vec<ProcessSpawnMetric>,
    pub prompts: Vec<ProcessPromptMetric>,
}

impl NodeProcessMetrics {
    pub fn record_spawn(&mut self, metric: ProcessSpawnMetric) {
        self.totals.record_spawn(&metric);
        self.process_spawns.push(metric);
    }

    pub fn record_prompt(&mut self, metric: ProcessPromptMetric) {
        self.totals.record_prompt(&metric);
        self.prompts.push(metric);
    }
}

/// Runtime activity owned by a single graph node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeMetrics {
    pub node_id: u64,
    pub operation: OperationMetricTotals,
    pub processes: NodeProcessMetrics,
}

impl NodeMetrics {
    pub fn new(node_id: u64) -> Self {
        Self {
            node_id,
            operation: OperationMetricTotals::default(),
            processes: NodeProcessMetrics::default(),
        }
    }

    pub fn record_operation(&mut self, metric: OperationMetric) {
        self.operation.record_operation(&metric);
    }

    pub fn record_spawn(&mut self, metric: ProcessSpawnMetric) {
        self.processes.record_spawn(metric);
    }

    pub fn record_prompt(&mut self, metric: ProcessPromptMetric) {
        self.processes.record_prompt(metric);
    }
}

/// Derived aggregate views over node-owned graph metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphMetricAggregates {
    pub by_agent: BTreeMap<String, ProcessMetricTotals>,
}

/// Serializable graph snapshot for metrics emission.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphMetricsSnapshot {
    pub graph: GraphMetricTotals,
    pub nodes: BTreeMap<u64, NodeMetrics>,
    pub aggregates: GraphMetricAggregates,
}

impl GraphMetricsSnapshot {
    pub fn is_empty(&self) -> bool {
        self.graph.is_empty()
    }

    pub fn to_json(&self) -> serde_json::Value {
        use crate::constants::metrics::metrics_keys as mk;
        use mk::graph_metric_keys as ak;

        serde_json::json!({
            mk::RUNTIME_GRAPH_METRICS: {
                ak::GRAPH: self.graph,
                ak::NODES: self.nodes,
                ak::AGGREGATES: self.aggregates,
            }
        })
    }
}
