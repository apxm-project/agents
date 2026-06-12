//! Flow Registry for multi-agent flow management.
//!
//! The FlowRegistry stores compiled flow DAGs for all agents, enabling
//! cross-agent flow calls and multi-agent execution.

use std::sync::Arc;

use apxm_core::types::{Agent, ExecutionDag};
use dashmap::DashMap;

/// Registry for storing and retrieving compiled flow DAGs.
///
/// This enables cross-agent flow calls by providing a way to look up
/// and execute flows from other agents.
#[derive(Debug, Default)]
pub struct FlowRegistry {
    /// Map of (agent_name, flow_name) -> compiled DAG
    flows: DashMap<(String, String), Arc<ExecutionDag>>,
    /// Map of agent_name -> runtime agent
    agents: DashMap<String, Arc<Agent>>,
}

impl FlowRegistry {
    /// Create a new empty flow registry.
    pub fn new() -> Self {
        Self {
            flows: DashMap::new(),
            agents: DashMap::new(),
        }
    }

    /// Register an agent and all of its flows.
    ///
    /// This stores the full runtime `Agent` model and populates the
    /// `(agent, flow) -> ExecutionDag` lookup used by flow calls.
    pub fn register_agent(&self, agent: Agent) {
        let agent_name = agent.name.clone();
        let stale_keys: Vec<(String, String)> = self
            .flows
            .iter()
            .filter(|entry| entry.key().0 == agent_name)
            .map(|entry| entry.key().clone())
            .collect();
        for key in stale_keys {
            self.flows.remove(&key);
        }

        let shared_agent = Arc::new(agent);
        for flow in shared_agent.flows.values() {
            self.flows.insert(
                (agent_name.clone(), flow.name.clone()),
                Arc::new(flow.execution_dag.clone()),
            );
        }
        self.agents.insert(agent_name, shared_agent);
    }

    /// Get a runtime agent by name.
    pub fn get_agent(&self, name: &str) -> Option<Arc<Agent>> {
        self.agents.get(name).map(|agent| Arc::clone(&agent))
    }

    /// List all registered agent names.
    pub fn list_agents(&self) -> Vec<String> {
        self.agents
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Register a flow in the registry.
    ///
    /// # Arguments
    /// * `agent` - The agent name that owns this flow
    /// * `flow` - The flow name
    /// * `dag` - The compiled execution DAG for this flow
    pub fn register_flow(&self, agent: &str, flow: &str, dag: ExecutionDag) {
        self.flows
            .insert((agent.to_string(), flow.to_string()), Arc::new(dag));
    }

    /// Get a flow from the registry.
    ///
    /// Returns a clone of the DAG if found, `None` otherwise.
    /// The DAG is wrapped in Arc for efficient sharing.
    pub fn get_flow(&self, agent: &str, flow: &str) -> Option<Arc<ExecutionDag>> {
        self.flows
            .get(&(agent.to_string(), flow.to_string()))
            .map(|dag| Arc::clone(&dag))
    }

    /// Check if a flow exists in the registry.
    pub fn has_flow(&self, agent: &str, flow: &str) -> bool {
        self.flows
            .contains_key(&(agent.to_string(), flow.to_string()))
    }

    /// List all registered flows.
    pub fn list_flows(&self) -> Vec<(String, String)> {
        self.flows.iter().map(|entry| entry.key().clone()).collect()
    }

    /// Get all flows for a specific agent.
    pub fn flows_for_agent(&self, agent: &str) -> Vec<String> {
        self.flows
            .iter()
            .filter(|entry| entry.key().0 == agent)
            .map(|entry| entry.key().1.clone())
            .collect()
    }

    /// Remove a flow from the registry.
    pub fn remove_flow(&self, agent: &str, flow: &str) -> Option<Arc<ExecutionDag>> {
        self.flows
            .remove(&(agent.to_string(), flow.to_string()))
            .map(|(_, dag)| dag)
    }

    /// Clear all registered flows.
    pub fn clear(&self) {
        self.flows.clear();
        self.agents.clear();
    }

    /// Find a flow by label.
    ///
    /// Labels may be "AgentName.flowName" (dotted) or just "flowName"
    /// (searches all agents for a match).
    pub fn find_flow_by_label(&self, label: &str) -> Option<Arc<ExecutionDag>> {
        if let Some((agent, flow)) = label.split_once('.') {
            return self.get_flow(agent, flow);
        }
        // Search all agents for a flow with this name
        self.flows
            .iter()
            .find(|entry| entry.key().1 == label)
            .map(|entry| Arc::clone(&entry))
    }

    /// Get the number of registered flows.
    pub fn len(&self) -> usize {
        self.flows.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }
}

