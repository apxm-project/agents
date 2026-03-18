//! Execution graph types module.
//!
//! Contains types for representing execution DAGs and their components.

mod agent;
mod task;
mod dag;
mod edge;
mod node;
mod status;
mod workflow;

pub use agent::{
    Agent, AgentFlow, AgentId, AgentMetadata, CapabilityDeclaration, MemoryDeclaration,
};
pub use task::{Task, TaskDag, TaskId, TaskMetadata};
pub use dag::{DagMetadata, ExecutionDag, FlowParameter};
pub use edge::{DependencyType, Edge};
pub use node::{LatencyTierConfig, Node, NodeId, NodeMetadata};
pub use status::{ExecutionStats, NodeStatus, OpStatus};
pub use workflow::WorkflowNode;
