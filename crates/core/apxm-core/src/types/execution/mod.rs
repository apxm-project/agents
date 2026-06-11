//! Execution graph types module.
//!
//! Contains types for representing execution DAGs and their components.

mod agent;
mod dag;
mod edge;
mod graph_metrics;
mod node;
mod status;
mod task;
mod workflow;

pub use agent::{
    Agent, AgentFlow, AgentId, AgentMetadata, CapabilityDeclaration, MemoryDeclaration,
};
pub use dag::{DagMetadata, ExecutionDag, FlowParameter};
pub use edge::{DependencyType, Edge};
pub use graph_metrics::{
    GraphMetricAggregates, GraphMetricTotals, GraphMetricsSnapshot, NodeMetrics,
    NodeProcessMetrics, OperationMetric, OperationMetricTotals, ProcessMetricTotals,
    ProcessPromptMetric, ProcessSpawnMetric, SpawnedProcessKind,
};
pub use node::{LatencyTierConfig, Node, NodeId, NodeMetadata};
pub use status::{
    ExecutionStats, NodeStatus, ObservedCriticalPath, ObservedGraphMetrics, ObservedQueueWait,
    OpStatus,
};
pub use task::{Task, TaskDag, TaskId, TaskMetadata};
pub use workflow::{
    WORKFLOW_SPAWN_PATH_TARGET_KINDS, WORKFLOW_TARGET_KIND_AIR_PATH,
    WORKFLOW_TARGET_KIND_ARTIFACT_PATH, WORKFLOW_TARGET_KIND_REGISTERED_FLOW,
    WORKFLOW_TARGET_KIND_WORKFLOW_PATH, WorkflowInvocation, WorkflowInvocationKind, WorkflowNode,
    WorkflowTarget,
};
