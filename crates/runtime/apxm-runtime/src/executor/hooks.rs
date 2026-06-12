//! Scheduler-level execution hooks.
//!
//! These hooks complement dispatcher middleware. Middleware wraps the actual
//! operation handler, while execution hooks observe scheduler lifecycle events
//! such as ready, start, finish, and graph completion.

use std::sync::Arc;

use apxm_core::types::{AISOperationType, NodeId, OpStatus};

/// Observer interface for runtime execution lifecycle events.
pub trait ExecutionHook: Send + Sync {
    /// Stable hook name for diagnostics.
    fn name(&self) -> &str {
        "execution-hook"
    }

    /// Called when a graph starts running.
    fn on_graph_started(&self, _event: &GraphStartedEvent) {}

    /// Called when a node becomes ready and is admitted to the scheduler queue.
    fn on_node_ready(&self, _event: &NodeReadyEvent) {}

    /// Called when a worker starts executing a ready node.
    fn on_node_started(&self, _event: &NodeStartedEvent) {}

    /// Called when a node reaches a terminal scheduler state.
    fn on_node_finished(&self, _event: &NodeFinishedEvent) {}

    /// Called when a graph finishes or fails.
    fn on_graph_finished(&self, _event: &GraphFinishedEvent) {}
}

/// Immutable hook set for one graph execution.
#[derive(Clone, Default)]
pub struct ExecutionHookContext {
    execution_id: String,
    graph_id: String,
    hooks: Vec<Arc<dyn ExecutionHook>>,
}

impl ExecutionHookContext {
    pub fn new(
        execution_id: impl Into<String>,
        graph_id: impl Into<String>,
        hooks: Vec<Arc<dyn ExecutionHook>>,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            graph_id: graph_id.into(),
            hooks,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }

    pub fn graph_id(&self) -> &str {
        &self.graph_id
    }

    pub fn emit_graph_started(&self, node_count: usize) {
        if self.is_empty() {
            return;
        }
        let event = GraphStartedEvent {
            execution_id: self.execution_id.clone(),
            graph_id: self.graph_id.clone(),
            node_count,
        };
        for hook in &self.hooks {
            hook.on_graph_started(&event);
        }
    }

    pub fn emit_node_ready(&self, event: NodeReadyEvent) {
        if self.is_empty() {
            return;
        }
        for hook in &self.hooks {
            hook.on_node_ready(&event);
        }
    }

    pub fn emit_node_started(&self, event: NodeStartedEvent) {
        if self.is_empty() {
            return;
        }
        for hook in &self.hooks {
            hook.on_node_started(&event);
        }
    }

    pub fn emit_node_finished(&self, event: NodeFinishedEvent) {
        if self.is_empty() {
            return;
        }
        for hook in &self.hooks {
            hook.on_node_finished(&event);
        }
    }

    pub fn emit_graph_finished(
        &self,
        executed_nodes: usize,
        failed_nodes: usize,
        duration_ms: u128,
        success: bool,
    ) {
        if self.is_empty() {
            return;
        }
        let event = GraphFinishedEvent {
            execution_id: self.execution_id.clone(),
            graph_id: self.graph_id.clone(),
            executed_nodes,
            failed_nodes,
            duration_ms,
            success,
        };
        for hook in &self.hooks {
            hook.on_graph_finished(&event);
        }
    }
}

#[derive(Debug, Clone)]
pub struct GraphStartedEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub node_count: usize,
}

#[derive(Debug, Clone)]
pub struct NodeReadyEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: NodeId,
    pub op_type: AISOperationType,
    pub priority: String,
    pub ready_at_ms: u128,
}

#[derive(Debug, Clone)]
pub struct NodeStartedEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: NodeId,
    pub op_type: AISOperationType,
    pub priority: String,
    pub worker_id: Option<usize>,
    pub ready_at_ms: Option<u128>,
    pub started_at_ms: u128,
    pub queue_wait_ms: Option<u128>,
}

#[derive(Debug, Clone)]
pub struct NodeFinishedEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: NodeId,
    pub op_type: AISOperationType,
    pub status: OpStatus,
    pub attempts: u32,
    pub started_at_ms: Option<u128>,
    pub finished_at_ms: u128,
    pub duration_ms: Option<u128>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GraphFinishedEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub executed_nodes: usize,
    pub failed_nodes: usize,
    pub duration_ms: u128,
    pub success: bool,
}

