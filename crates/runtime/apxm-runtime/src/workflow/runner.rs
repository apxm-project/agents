//! Workflow result types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Result of workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub workflow_name: String,
    pub status: WorkflowStatus,
    pub step_results: HashMap<String, StepResult>,
    pub output: Option<String>,
    pub duration_ms: u64,
}

/// Overall workflow execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowStatus {
    Success,
    PartialFailure,
    Failed,
}

/// Result of a single step execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub id: String,
    pub status: StepStatus,
    pub output: Option<String>,
    pub duration_ms: u64,
    pub session_dir: Option<PathBuf>,
    pub error: Option<String>,
}

impl Default for StepResult {
    fn default() -> Self {
        Self {
            id: String::new(),
            status: StepStatus::Skipped,
            output: None,
            duration_ms: 0,
            session_dir: None,
            error: None,
        }
    }
}

/// Status of a single step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Success,
    Failed,
    Skipped,
}
