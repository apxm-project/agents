//! Agent Abstract Machine (AAM) Model
//!
//! The AAM formalizes the essential state of an autonomous agent as:
//!
//! ```text
//! AAM = (B, G, C)
//! ```
//!
//! Where:
//! - **B (Beliefs)**: Key-value store of agent's knowledge
//! - **G (Goals)**: Priority queue of objectives
//! - **C (Capabilities)**: Map of tool names to function signatures

use crate::types::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Unique identifier for an AAM goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoalId(Uuid);

impl GoalId {
    pub fn new() -> Self {
        GoalId(Uuid::now_v7())
    }
}

impl Default for GoalId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for GoalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Status of a goal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalStatus {
    /// Goal is pending execution.
    Pending,
    /// Goal is currently being worked on.
    Active,
    /// Goal has been completed successfully.
    Completed,
    /// Goal has failed.
    Failed,
    /// Goal has been cancelled.
    Cancelled,
}

/// Goal descriptor with priority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    /// Unique identifier for the goal.
    pub id: GoalId,
    /// Description of the goal.
    pub description: String,
    /// Priority (higher = more important).
    pub priority: u32,
    /// Current status.
    pub status: GoalStatus,
    /// Optional parent goal (for hierarchical goals).
    pub parent_id: Option<GoalId>,
}

/// Capability (tool) specification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Capability {
    /// Unique name of the capability.
    pub name: String,
    /// Description of what the capability does.
    pub description: String,
    /// Input parameter schema (JSON Schema format).
    pub input_schema: Option<serde_json::Value>,
    /// Output type description.
    pub output_description: Option<String>,
    /// Whether the capability is currently available.
    pub available: bool,
}

/// Beliefs: Agent's knowledge store.
///
/// A key-value store representing the agent's current beliefs/knowledge.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Beliefs {
    /// The belief store.
    store: HashMap<String, Value>,
}

impl Beliefs {
    /// Create empty beliefs.
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }

    /// Get a belief by key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.store.get(key)
    }

    /// Set a belief.
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        self.store.insert(key.into(), value);
    }

    /// Remove a belief.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.store.remove(key)
    }

    /// Check if a belief exists.
    pub fn contains(&self, key: &str) -> bool {
        self.store.contains_key(key)
    }

    /// Get all beliefs.
    pub fn all(&self) -> &HashMap<String, Value> {
        &self.store
    }

    /// Get the number of beliefs.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Check if beliefs are empty.
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

/// Goals: Agent's objectives queue.
///
/// A priority queue of goals that drive agent behavior.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Goals {
    /// The goals list (kept sorted by priority).
    goals: Vec<Goal>,
}

impl Goals {
    /// Create empty goals.
    pub fn new() -> Self {
        Self { goals: Vec::new() }
    }

    /// Add a goal.
    pub fn push(&mut self, goal: Goal) {
        self.goals.push(goal);
        // Sort by priority (descending)
        self.goals.sort_by_key(|b| std::cmp::Reverse(b.priority));
    }

    /// Get the highest priority active goal.
    pub fn peek(&self) -> Option<&Goal> {
        self.goals
            .iter()
            .find(|g| g.status == GoalStatus::Active || g.status == GoalStatus::Pending)
    }

    /// Get a goal by ID.
    pub fn get(&self, id: GoalId) -> Option<&Goal> {
        self.goals.iter().find(|g| g.id == id)
    }

    /// Get a mutable goal by ID.
    pub fn get_mut(&mut self, id: GoalId) -> Option<&mut Goal> {
        self.goals.iter_mut().find(|g| g.id == id)
    }

    /// Update goal status.
    pub fn set_status(&mut self, id: GoalId, status: GoalStatus) -> bool {
        if let Some(goal) = self.get_mut(id) {
            goal.status = status;
            true
        } else {
            false
        }
    }

    /// Get all goals.
    pub fn all(&self) -> &[Goal] {
        &self.goals
    }

    /// Get active goals.
    pub fn active(&self) -> impl Iterator<Item = &Goal> {
        self.goals.iter().filter(|g| g.status == GoalStatus::Active)
    }

    /// Get the number of goals.
    pub fn len(&self) -> usize {
        self.goals.len()
    }

    /// Check if goals are empty.
    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
    }
}

/// Capabilities: Agent's available tools.
///
/// A registry of capabilities (tools) the agent can invoke.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    /// The capabilities registry.
    registry: HashMap<String, Capability>,
}

impl Capabilities {
    /// Create empty capabilities.
    pub fn new() -> Self {
        Self {
            registry: HashMap::new(),
        }
    }

    /// Register a capability.
    pub fn register(&mut self, capability: Capability) {
        self.registry.insert(capability.name.clone(), capability);
    }

    /// Get a capability by name.
    pub fn get(&self, name: &str) -> Option<&Capability> {
        self.registry.get(name)
    }

    /// Check if a capability is registered.
    pub fn has(&self, name: &str) -> bool {
        self.registry.contains_key(name)
    }

    /// Check if a capability is available.
    pub fn is_available(&self, name: &str) -> bool {
        self.get(name).is_some_and(|c| c.available)
    }

    /// Get all capabilities.
    pub fn all(&self) -> &HashMap<String, Capability> {
        &self.registry
    }

    /// Get available capabilities.
    pub fn available(&self) -> impl Iterator<Item = &Capability> {
        self.registry.values().filter(|c| c.available)
    }

    /// Get the number of capabilities.
    pub fn len(&self) -> usize {
        self.registry.len()
    }

    /// Check if capabilities are empty.
    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }
}

/// The Agent Abstract Machine.
///
/// Represents the complete state of an agent as (Beliefs, Goals, Capabilities).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AAM {
    /// Agent's beliefs (knowledge store).
    pub beliefs: Beliefs,
    /// Agent's goals (objectives queue).
    pub goals: Goals,
    /// Agent's capabilities (available tools).
    pub capabilities: Capabilities,
}

impl AAM {
    /// Create a new empty AAM.
    pub fn new() -> Self {
        Self {
            beliefs: Beliefs::new(),
            goals: Goals::new(),
            capabilities: Capabilities::new(),
        }
    }

    /// Create AAM with initial beliefs.
    pub fn with_beliefs(beliefs: Beliefs) -> Self {
        Self {
            beliefs,
            goals: Goals::new(),
            capabilities: Capabilities::new(),
        }
    }
}
