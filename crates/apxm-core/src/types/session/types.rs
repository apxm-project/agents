use serde::{Deserialize, Serialize};

/// Session execution manifest — written to `manifest.json` in each session directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManifest {
    /// Unique execution identifier.
    pub execution_id: String,
    /// Name of the graph that was executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_name: Option<String>,
    /// ISO 8601 timestamp of when the session started.
    pub timestamp: String,
    /// Current status (running/completed/failed).
    pub status: String,
    /// Total execution duration in milliseconds.
    pub duration_ms: u128,
    /// Number of nodes executed.
    pub node_count: usize,
    /// Whether execution succeeded.
    pub success: bool,
}

/// Live session state — updated in real-time as nodes complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSessionState {
    /// Current status (running/completed/failed).
    pub status: String,
    /// Node IDs currently executing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running_nodes: Vec<NodeInfo>,
    /// Nodes that have completed (most recent first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_nodes: Vec<CompletedNodeInfo>,
    /// Total number of nodes completed so far.
    pub completed: usize,
    /// Total number of nodes in the graph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    /// Elapsed time in milliseconds since session start.
    pub elapsed_ms: u128,
    /// Overall success flag (only meaningful when status != running).
    pub success: bool,
    /// Current execution phase description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_phase: Option<String>,
}

/// Information about a node (for live tracking).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    /// Node ID.
    pub id: u64,
    /// Node name.
    pub name: String,
    /// Operation type.
    pub op: String,
}

/// Information about a completed node (for live tracking).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedNodeInfo {
    /// Node ID.
    pub id: u64,
    /// Node name.
    pub name: String,
    /// Operation type.
    pub op: String,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the node succeeded.
    pub status: String,
}

