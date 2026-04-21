use std::fmt;

use serde::{Deserialize, Serialize};

/// Status of a session or completed node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Running,
    Completed,
    Failed,
    Pending,
    Resumed,
    Submitted,
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Pending => write!(f, "pending"),
            Self::Resumed => write!(f, "resumed"),
            Self::Submitted => write!(f, "submitted"),
        }
    }
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Pending => "pending",
            Self::Resumed => "resumed",
            Self::Submitted => "submitted",
        }
    }
}

/// Session execution manifest — written to `manifest.json` in each session directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManifest {
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_name: Option<String>,
    pub timestamp: String,
    pub status: SessionStatus,
    pub duration_ms: u128,
    pub node_count: usize,
    pub success: bool,
    /// Scope identifier for session isolation. `None` means global scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
}

/// Live session state — updated in real-time as nodes complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSessionState {
    pub status: SessionStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running_nodes: Vec<NodeInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_nodes: Vec<CompletedNodeInfo>,
    pub completed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    pub elapsed_ms: u128,
    /// Only meaningful when status != running.
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_phase: Option<String>,
}

/// Node info for live tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: u64,
    pub name: String,
    pub op: String,
}

/// Completed node info for live tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedNodeInfo {
    pub id: u64,
    pub name: String,
    pub op: String,
    pub duration_ms: u64,
    pub status: SessionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<usize>,
}
