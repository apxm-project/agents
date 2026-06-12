//! Runtime agent model.
//!
//! Agents are the top-level runtime unit that own named flows. Each flow can
//! preserve its original task DAG representation and its lowered execution
//! DAG used by the scheduler.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{ExecutionDag, FlowParameter, TaskDag};
use crate::error::runtime::RuntimeError;

/// Unique identifier for an agent.
pub type AgentId = String;

/// Runtime agent containing one or more flows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    /// Human-readable agent name.
    pub name: AgentId,
    /// Agent-level declarations (memory, capabilities, tools, context).
    #[serde(default)]
    pub metadata: AgentMetadata,
    /// Named flows owned by this agent.
    #[serde(default)]
    pub flows: HashMap<String, AgentFlow>,
}

impl Agent {
    /// Create a new runtime agent with no flows.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            metadata: AgentMetadata::default(),
            flows: HashMap::new(),
        }
    }

    /// Add (or replace) a flow in this agent.
    pub fn add_flow(mut self, flow: AgentFlow) -> Self {
        self.flows.insert(flow.name.clone(), flow);
        self
    }

    /// Get the designated entry flow, if present.
    pub fn entry_flow(&self) -> Option<&AgentFlow> {
        self.flows.values().find(|flow| flow.is_entry)
    }

    /// Get a named flow by name.
    pub fn get_flow(&self, flow_name: &str) -> Option<&AgentFlow> {
        self.flows.get(flow_name)
    }
}

/// A single flow within an agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentFlow {
    /// Flow name.
    pub name: String,
    /// Whether this flow is the entry flow for its artifact.
    #[serde(default)]
    pub is_entry: bool,
    /// Flow parameters (for entry argument validation).
    #[serde(default)]
    pub parameters: Vec<FlowParameter>,
    /// Optional original task DAG representation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_dag: Option<TaskDag>,
    /// Lowered execution DAG used by runtime scheduling.
    pub execution_dag: ExecutionDag,
}

impl AgentFlow {
    /// Build an agent flow from a task DAG while preserving both views.
    pub fn from_task_dag(
        name: impl Into<String>,
        task_dag: TaskDag,
        is_entry: bool,
    ) -> Result<Self, RuntimeError> {
        let name = name.into();
        let mut execution_dag = task_dag.to_execution_dag()?;
        execution_dag.metadata.name = Some(name.clone());
        execution_dag.metadata.is_entry = is_entry;
        let parameters = execution_dag.metadata.parameters.clone();

        Ok(Self {
            name,
            is_entry,
            parameters,
            task_dag: Some(task_dag),
            execution_dag,
        })
    }
}

/// Runtime-level declarations associated with an agent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentMetadata {
    /// Declared memories.
    #[serde(default)]
    pub memories: Vec<MemoryDeclaration>,
    /// Declared capabilities.
    #[serde(default)]
    pub capabilities: Vec<CapabilityDeclaration>,
    /// Declared tool names.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Discoverable capability/tool names.
    #[serde(default)]
    pub discoverable: Vec<String>,
    /// Optional contextual prompt/details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Declares an agent memory by name and tier.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryDeclaration {
    pub name: String,
    pub tier: String,
}

/// Declares an agent capability.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityDeclaration {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

