//! Goal-state helpers for the Agent Abstract Machine.

use apxm_core::types::goal::{Goal, GoalId, GoalStatus};
use priority_queue::PriorityQueue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Priority queue used for active goals.
pub type GoalQueue = PriorityQueue<GoalId, u32>;

/// Detailed goal storage keyed by goal id.
pub type GoalDetailMap = HashMap<GoalId, Goal>;

/// Goal-level changes recorded for one transition.
#[derive(Debug, Clone)]
pub enum GoalChange {
    Added(Goal),
    Removed(GoalId),
    StatusChanged {
        id: GoalId,
        from: GoalStatus,
        to: GoalStatus,
    },
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
        self.children.get(parent_id).map(|v| v.as_slice()).unwrap_or(&[])
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

/// Snapshot goal queue/details for the selected goal descriptions.
pub fn filtered_state(keys: &[String], goals: &GoalDetailMap) -> (GoalQueue, GoalDetailMap) {
    let mut queue = GoalQueue::new();
    let mut details = GoalDetailMap::new();
    for goal in goals.values() {
        if keys.contains(&goal.description) {
            queue.push(goal.id, goal.priority);
            details.insert(goal.id, goal.clone());
        }
    }
    (queue, details)
}
