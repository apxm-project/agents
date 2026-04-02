//! Canonical Agent Abstract Machine contracts shared across APXM crates.

use crate::types::goal::GoalId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Projected AAM state for transmission to an ACP agent.
///
/// This is the bridge type — a purpose-built projection of (B, G, C)
/// that can be rendered into formats ACP agents understand.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AamContext {
    pub beliefs: HashMap<String, serde_json::Value>,
    pub goals: Vec<GoalProjection>,
    pub capabilities: Vec<CapabilityProjection>,
    pub system_prompt: Option<String>,
}

/// A goal projected for transmission to an ACP agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalProjection {
    pub description: String,
    pub priority: u32,
}

/// A capability projected for transmission to an ACP agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityProjection {
    pub name: String,
    pub description: String,
}

/// Capability metadata tracked in the AAM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRecord {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
    pub cost_estimate: f64,
}

/// Policy for when a parent goal should be auto-completed based on children.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletionPolicy {
    /// Parent completes when ALL children are completed.
    #[default]
    AllChildren,
    /// Parent completes when ANY child is completed.
    AnyChild,
    /// Parent never auto-completes; must be set manually.
    Manual,
}

/// Tracks parent-child relationships between goals.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoalTree {
    /// Maps parent -> children
    children: HashMap<GoalId, Vec<GoalId>>,
    /// Completion policy per goal (only meaningful for parent goals)
    policies: HashMap<GoalId, CompletionPolicy>,
}

impl GoalTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a child goal under a parent.
    pub fn add_child(&mut self, parent_id: GoalId, child_id: GoalId) {
        self.children.entry(parent_id).or_default().push(child_id);
    }

    /// Get children of a goal.
    pub fn children_of(&self, parent_id: &GoalId) -> &[GoalId] {
        self.children
            .get(parent_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Set completion policy for a goal.
    pub fn set_policy(&mut self, goal_id: GoalId, policy: CompletionPolicy) {
        self.policies.insert(goal_id, policy);
    }

    /// Get completion policy for a goal (defaults to AllChildren).
    pub fn policy(&self, goal_id: &GoalId) -> CompletionPolicy {
        self.policies.get(goal_id).copied().unwrap_or_default()
    }

    /// Remove a goal from the tree (both as parent and child).
    pub fn remove(&mut self, goal_id: &GoalId) {
        self.children.remove(goal_id);
        self.policies.remove(goal_id);
        for children in self.children.values_mut() {
            children.retain(|id| id != goal_id);
        }
    }
}

/// Controls which parts of an AAM dimension a child scope inherits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScopePolicy {
    /// Share all entries from the parent (child writes are visible to parent).
    Inherit,
    /// Start with empty state.
    Isolate,
    /// Child gets a point-in-time copy; writes do not affect the parent.
    Snapshot,
    /// Inherit only the listed keys (snapshot semantics for the selected keys).
    Filter(Vec<String>),
}

/// Per-dimension scope specification for hierarchical AAM execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeSpec {
    pub beliefs: ScopePolicy,
    pub capabilities: ScopePolicy,
    pub goals: ScopePolicy,
}

impl ScopeSpec {
    /// Create a child scope that snapshots every AAM dimension.
    pub fn snapshot_all() -> Self {
        Self {
            beliefs: ScopePolicy::Snapshot,
            capabilities: ScopePolicy::Snapshot,
            goals: ScopePolicy::Snapshot,
        }
    }
}

impl Default for ScopeSpec {
    fn default() -> Self {
        Self {
            beliefs: ScopePolicy::Inherit,
            capabilities: ScopePolicy::Inherit,
            goals: ScopePolicy::Inherit,
        }
    }
}
