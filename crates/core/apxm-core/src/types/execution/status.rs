//! Execution status types.
//!
//! Types for tracking the status of operations during DAG execution.

use crate::types::NodeId;

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
    /// Milliseconds since execution start.
    pub started_at_ms: Option<u128>,
    /// Milliseconds since execution start.
    pub finished_at_ms: Option<u128>,
    pub duration_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<usize>,
}

/// Execution statistics for a completed DAG.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionStats {
    pub executed_nodes: usize,
    pub failed_nodes: usize,
    pub duration_ms: u128,
    pub node_statuses: Vec<NodeStatus>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_stats_is_success() {
        let success = ExecutionStats {
            executed_nodes: 10,
            failed_nodes: 0,
            duration_ms: 1000,
            node_statuses: vec![],
        };
        assert!(success.is_success());

        let failure = ExecutionStats {
            executed_nodes: 8,
            failed_nodes: 2,
            duration_ms: 1000,
            node_statuses: vec![],
        };
        assert!(!failure.is_success());
    }
}
