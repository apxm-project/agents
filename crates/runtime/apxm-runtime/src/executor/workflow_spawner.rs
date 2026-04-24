//! Workflow spawn bridge for child graph/artifact/workflow execution.

use apxm_core::{
    error::RuntimeError,
    types::{WorkflowInvocation, values::Value},
};
use async_trait::async_trait;

/// Result returned by a workflow-spawn bridge.
#[derive(Debug, Clone)]
pub struct WorkflowSpawnResult {
    pub value: Value,
    pub session_dir: Option<String>,
}

/// Bridge trait that lets the runtime delegate child execution to the host.
#[async_trait]
pub trait WorkflowSpawner: Send + Sync {
    async fn spawn_workflow(
        &self,
        invocation: WorkflowInvocation,
    ) -> Result<WorkflowSpawnResult, RuntimeError>;
}

/// No-op bridge for runtimes that do not support cross-workflow execution.
pub struct NoOpWorkflowSpawner;

#[async_trait]
impl WorkflowSpawner for NoOpWorkflowSpawner {
    async fn spawn_workflow(
        &self,
        _invocation: WorkflowInvocation,
    ) -> Result<WorkflowSpawnResult, RuntimeError> {
        Err(RuntimeError::State(
            "Workflow spawning not supported in this context".to_string(),
        ))
    }
}
