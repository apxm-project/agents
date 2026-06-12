//! Agent Abstract Machine (AAM) state management.
//!
//! This module provides the first pass of the AAM used by the runtime. It tracks
//! beliefs (key/value map), goals (priority queue), registered capabilities, and
//! episodic state transitions. The interface is intentionally conservative so we
//! can evolve it alongside the rest of the runtime.

mod beliefs;
mod capabilities;
pub mod effects;
mod goals;
mod scope;
pub mod session;

use apxm_core::error::RuntimeError;
pub use apxm_core::types::goal::{Goal, GoalId, GoalStatus};
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
pub use beliefs::{BeliefChangeSet, BeliefMap};
pub use capabilities::{CapabilityChange, CapabilityMap, CapabilityRecord};
use chrono::{DateTime, Utc};
pub use goals::{CompletionPolicy, GoalChange, GoalDetailMap, GoalQueue, GoalTree};
use parking_lot::RwLock;
pub use scope::{ScopePolicy, ScopeSpec};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Prefix for staged belief keys (used by QMEM).
pub const STAGED_BELIEF_PREFIX: &str = apxm_core::constants::runtime::belief_keys::STAGED_PREFIX;

/// Shared handle to the Agent Abstract Machine state.
#[derive(Clone, Default)]
pub struct Aam {
    inner: Arc<RwLock<AamState>>,
}

impl std::fmt::Debug for Aam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aam").finish_non_exhaustive()
    }
}

impl Aam {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(AamState::new())),
        }
    }

    /// Apply a mutation to the AAM state.
    pub fn apply_transition<F>(&self, label: TransitionLabel, f: F) -> TransitionRecord
    where
        F: FnOnce(&mut AamState) -> TransitionDelta,
    {
        let mut state = self.inner.write();
        state.apply_transition(label, f)
    }

    /// Read-only snapshot of beliefs.
    pub fn beliefs(&self) -> HashMap<String, Value> {
        self.inner.read().beliefs.clone()
    }

    pub fn get_belief(&self, key: &str) -> Option<Value> {
        self.inner.read().beliefs.get(key).cloned()
    }

    pub fn set_belief(
        &self,
        key: String,
        value: Value,
        label: TransitionLabel,
    ) -> TransitionRecord {
        self.apply_transition(label, move |state| {
            let mut delta = TransitionDelta::default();
            let before = state.beliefs.insert(key.clone(), value.clone());
            delta.belief_changes.insert(key, (before, Some(value)));
            delta
        })
    }

    pub fn add_goal(&self, goal: Goal, label: TransitionLabel) -> TransitionRecord {
        self.apply_transition(label, move |state| state.add_goal(goal))
    }

    pub fn register_capability(
        &self,
        name: String,
        metadata: CapabilityRecord,
        label: TransitionLabel,
    ) -> TransitionRecord {
        self.apply_transition(label, move |state| {
            state.capabilities.insert(name.clone(), metadata.clone());
            let mut delta = TransitionDelta::default();
            delta
                .capability_changes
                .push(CapabilityChange::Registered { name, metadata });
            delta
        })
    }

    pub fn recent_transitions(&self, last_n: usize) -> Vec<TransitionRecord> {
        let state = self.inner.read();
        let start = state.transitions.len().saturating_sub(last_n);
        state.transitions[start..].to_vec()
    }

    pub fn checkpoint(&self) -> AamCheckpoint {
        self.inner.read().snapshot()
    }

    pub fn restore(&self, checkpoint: &AamCheckpoint) {
        self.inner.write().restore(checkpoint);
    }

    pub fn enter_operation(&self, op_id: u64) {
        let mut state = self.inner.write();
        state.call_stack.push(CallFrame {
            op_id,
            entered_at: Utc::now(),
        });
    }

    pub fn exit_operation(&self) -> Option<u64> {
        self.inner.write().call_stack.pop().map(|frame| frame.op_id)
    }

    pub fn register_exception_handler(&self, op_id: u64, handler_id: u64) {
        self.inner
            .write()
            .exception_handlers
            .insert(op_id, handler_id);
    }

    pub fn current_exception_handler(&self) -> Option<u64> {
        let state = self.inner.read();
        state
            .call_stack
            .last()
            .and_then(|frame| state.exception_handlers.get(&frame.op_id))
            .copied()
    }

    pub fn goals(&self) -> Vec<Goal> {
        self.inner.read().goal_details.values().cloned().collect()
    }

    pub fn top_goal(&self) -> Option<Goal> {
        let state = self.inner.read();
        state
            .goals
            .peek()
            .and_then(|(id, _)| state.goal_details.get(id).cloned())
    }

    pub fn update_goal_status(
        &self,
        goal_id: GoalId,
        new_status: GoalStatus,
        label: TransitionLabel,
    ) -> Option<TransitionRecord> {
        let mut state = self.inner.write();
        if !state.goal_details.contains_key(&goal_id) {
            return None;
        }
        Some(state.apply_transition(label, move |state| {
            state.update_goal_status(goal_id, new_status)
        }))
    }

    pub fn update_goal(
        &self,
        goal_id: GoalId,
        description: Option<String>,
        priority: Option<u32>,
        status: Option<GoalStatus>,
        label: TransitionLabel,
    ) -> Option<TransitionRecord> {
        let mut state = self.inner.write();
        if !state.goal_details.contains_key(&goal_id) {
            return None;
        }
        Some(state.apply_transition(label, move |state| {
            state.update_goal(goal_id, description, priority, status)
        }))
    }

    pub fn remove_goal(&self, goal_id: GoalId, label: TransitionLabel) -> Option<TransitionRecord> {
        let mut state = self.inner.write();
        if !state.goal_details.contains_key(&goal_id) {
            return None;
        }
        Some(state.apply_transition(label, move |state| state.remove_goal(goal_id)))
    }

    /// Add a child goal under a parent, registering the relationship in the goal tree.
    pub fn add_child_goal(
        &self,
        parent_id: GoalId,
        child: Goal,
        label: TransitionLabel,
    ) -> TransitionRecord {
        let child_id = child.id;
        self.apply_transition(label, move |state| {
            let delta = state.add_goal(child);
            state.goal_tree.add_child(parent_id, child_id);
            delta
        })
    }

    /// Set the completion policy for a goal.
    pub fn set_completion_policy(&self, goal_id: GoalId, policy: CompletionPolicy) {
        self.inner.write().goal_tree.set_policy(goal_id, policy);
    }

    /// The explicitly-set completion policy for a goal, if any.
    pub fn completion_policy(&self, goal_id: &GoalId) -> Option<CompletionPolicy> {
        self.inner.read().goal_tree.policy_opt(goal_id)
    }

    /// Check if a parent goal should auto-complete based on its children's statuses,
    /// and if so, mark it as Completed. Returns the transition record if status changed.
    pub fn propagate_completion(
        &self,
        goal_id: GoalId,
        label: TransitionLabel,
    ) -> Option<TransitionRecord> {
        let mut state = self.inner.write();
        let policy = state.goal_tree.policy(&goal_id);
        let children = state.goal_tree.children_of(&goal_id);

        if children.is_empty() {
            return None;
        }

        let should_complete = match policy {
            CompletionPolicy::AllChildren => children.iter().all(|child_id| {
                state
                    .goal_details
                    .get(child_id)
                    .map(|g| g.status == GoalStatus::Completed)
                    .unwrap_or(false) // missing children should NOT count as completed
            }),
            CompletionPolicy::AnyChild => children.iter().any(|child_id| {
                state
                    .goal_details
                    .get(child_id)
                    .map(|g| g.status == GoalStatus::Completed)
                    .unwrap_or(false)
            }),
            CompletionPolicy::Manual => false,
        };

        if should_complete {
            if !state.goal_details.contains_key(&goal_id) {
                return None;
            }
            Some(state.apply_transition(label, move |state| {
                state.update_goal_status(goal_id, GoalStatus::Completed)
            }))
        } else {
            None
        }
    }

    /// Get the children of a goal.
    pub fn children_of(&self, goal_id: &GoalId) -> Vec<GoalId> {
        self.inner.read().goal_tree.children_of(goal_id).to_vec()
    }

    /// Return the priority of the highest-priority *active* goal, if any.
    ///
    /// This bridges the AAM goal system to the scheduler: callers can project
    /// this value onto node scheduling priorities so that nodes associated with
    /// high-priority goals are executed first.
    pub fn active_goal_priority(&self) -> Option<u32> {
        let state = self.inner.read();
        state
            .goal_details
            .values()
            .filter(|g| g.status == GoalStatus::Active)
            .map(|g| g.priority)
            .max()
    }

    /// Look up the priority of a specific goal by its description key.
    ///
    /// This is used by the scheduler to resolve the `goal_id` attribute
    /// (which stores the goal description) to its AAM priority.
    pub fn goal_priority_by_description(&self, description: &str) -> Option<u32> {
        let state = self.inner.read();
        state
            .goal_details
            .values()
            .find(|g| g.description == description)
            .map(|g| g.priority)
    }

    pub fn has_capability(&self, name: &str) -> bool {
        self.inner.read().capabilities.contains_key(name)
    }

    pub fn capabilities(&self) -> HashMap<String, CapabilityRecord> {
        self.inner.read().capabilities.clone()
    }

    /// Create a child AAM governed by the given [`ScopeSpec`].
    ///
    /// * **Inherit** on *all* dimensions → the child shares the parent's
    ///   `Arc<RwLock<AamState>>` (writes in either direction are visible).
    /// * Any non-Inherit dimension → a new `AamState` is created.
    ///   - `Isolate` → that dimension starts empty.
    ///   - `Snapshot` → that dimension gets a point-in-time copy.
    ///   - `Filter(keys)` → that dimension gets only the listed keys (snapshot
    ///     semantics for the selected subset).
    pub fn child_scope(&self, spec: &ScopeSpec) -> Aam {
        // Fast path: if everything is Inherit, share the same Arc.
        if matches!(
            (&spec.beliefs, &spec.capabilities, &spec.goals),
            (
                ScopePolicy::Inherit,
                ScopePolicy::Inherit,
                ScopePolicy::Inherit
            )
        ) {
            return Aam {
                inner: Arc::clone(&self.inner),
            };
        }

        let parent = self.inner.read();

        let beliefs = match &spec.beliefs {
            ScopePolicy::Inherit | ScopePolicy::Snapshot => parent.beliefs.clone(),
            ScopePolicy::Isolate => BeliefMap::new(),
            ScopePolicy::Filter(keys) => beliefs::snapshot_subset(&parent.beliefs, keys),
        };

        let capabilities = match &spec.capabilities {
            ScopePolicy::Inherit | ScopePolicy::Snapshot => parent.capabilities.clone(),
            ScopePolicy::Isolate => CapabilityMap::new(),
            ScopePolicy::Filter(keys) => capabilities::snapshot_subset(&parent.capabilities, keys),
        };

        let (goals, goal_details) = match &spec.goals {
            ScopePolicy::Inherit | ScopePolicy::Snapshot => {
                (parent.goals.clone(), parent.goal_details.clone())
            }
            ScopePolicy::Isolate => (GoalQueue::new(), GoalDetailMap::new()),
            ScopePolicy::Filter(keys) => goals::filtered_state(keys, &parent.goal_details),
        };

        let new_state = AamState {
            beliefs,
            goals,
            goal_details,
            capabilities,
            // Child starts with an empty episodic log, call stack, and exception table.
            transitions: Vec::new(),
            call_stack: Vec::new(),
            exception_handlers: HashMap::new(),
            goal_tree: parent.goal_tree.clone(),
        };

        Aam {
            inner: Arc::new(RwLock::new(new_state)),
        }
    }
}

/// Internal AAM state.
pub struct AamState {
    pub beliefs: BeliefMap,
    pub goals: GoalQueue,
    pub goal_details: GoalDetailMap,
    pub capabilities: CapabilityMap,
    pub transitions: Vec<TransitionRecord>,
    pub call_stack: Vec<CallFrame>,
    pub exception_handlers: HashMap<u64, u64>,
    pub goal_tree: GoalTree,
}

impl Default for AamState {
    fn default() -> Self {
        Self::new()
    }
}

impl AamState {
    pub fn new() -> Self {
        Self {
            beliefs: BeliefMap::new(),
            goals: GoalQueue::new(),
            goal_details: GoalDetailMap::new(),
            capabilities: CapabilityMap::new(),
            transitions: Vec::new(),
            call_stack: Vec::new(),
            exception_handlers: HashMap::new(),
            goal_tree: GoalTree::new(),
        }
    }

    fn apply_transition<F>(&mut self, label: TransitionLabel, f: F) -> TransitionRecord
    where
        F: FnOnce(&mut Self) -> TransitionDelta,
    {
        let before = self.beliefs.clone();
        let delta = f(self);
        let belief_changes = beliefs::diff_beliefs(&before, &self.beliefs, delta.belief_changes);

        let record = TransitionRecord {
            timestamp: Utc::now(),
            label,
            belief_changes,
            goal_changes: delta.goal_changes,
            capability_changes: delta.capability_changes,
        };
        self.transitions.push(record.clone());
        record
    }

    fn add_goal(&mut self, goal: Goal) -> TransitionDelta {
        let priority = goal.priority;
        let id = goal.id;
        self.goals.push(id, priority);
        self.goal_details.insert(id, goal.clone());

        let mut delta = TransitionDelta::default();
        delta.goal_changes.push(GoalChange::Added(goal));
        delta
    }

    fn update_goal_status(&mut self, goal_id: GoalId, new_status: GoalStatus) -> TransitionDelta {
        let mut delta = TransitionDelta::default();
        if let Some(goal) = self.goal_details.get_mut(&goal_id) {
            let old = goal.status;
            goal.status = new_status;
            delta.goal_changes.push(GoalChange::StatusChanged {
                id: goal_id,
                from: old,
                to: new_status,
            });
        }
        delta
    }

    fn update_goal(
        &mut self,
        goal_id: GoalId,
        description: Option<String>,
        priority: Option<u32>,
        status: Option<GoalStatus>,
    ) -> TransitionDelta {
        let mut delta = TransitionDelta::default();
        if let Some(goal) = self.goal_details.get_mut(&goal_id) {
            let before = goal.clone();
            if let Some(description) = description {
                goal.description = description;
            }
            if let Some(priority) = priority {
                goal.priority = priority;
                self.goals.remove(&goal_id);
                self.goals.push(goal_id, priority);
            }
            if let Some(status) = status {
                let old = goal.status;
                goal.status = status;
                if old != status {
                    delta.goal_changes.push(GoalChange::StatusChanged {
                        id: goal_id,
                        from: old,
                        to: status,
                    });
                }
            }
            if before != *goal {
                delta.goal_changes.push(GoalChange::Updated {
                    before,
                    after: goal.clone(),
                });
            }
        }
        delta
    }

    fn remove_goal(&mut self, goal_id: GoalId) -> TransitionDelta {
        let mut delta = TransitionDelta::default();
        self.goals.remove(&goal_id);
        self.goal_tree.remove(&goal_id);
        if self.goal_details.remove(&goal_id).is_some() {
            delta.goal_changes.push(GoalChange::Removed(goal_id));
        }
        delta
    }

    pub fn snapshot(&self) -> AamCheckpoint {
        AamCheckpoint {
            beliefs: self.beliefs.clone(),
            goals: self.goal_details.values().cloned().collect(),
            capabilities: self.capabilities.clone(),
            goal_tree: self.goal_tree.clone(),
            timestamp: Utc::now(),
        }
    }

    fn restore(&mut self, checkpoint: &AamCheckpoint) {
        self.beliefs = checkpoint.beliefs.clone();
        self.capabilities = checkpoint.capabilities.clone();
        self.goal_tree = checkpoint.goal_tree.clone();
        self.goals.clear();
        self.goal_details.clear();
        for goal in &checkpoint.goals {
            self.goals.push(goal.id, goal.priority);
            self.goal_details.insert(goal.id, goal.clone());
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransitionRecord {
    pub timestamp: DateTime<Utc>,
    pub label: TransitionLabel,
    pub belief_changes: BeliefChangeSet,
    pub goal_changes: Vec<GoalChange>,
    pub capability_changes: Vec<CapabilityChange>,
}

#[derive(Debug, Clone, Default)]
pub struct TransitionDelta {
    pub belief_changes: BeliefChangeSet,
    pub goal_changes: Vec<GoalChange>,
    pub capability_changes: Vec<CapabilityChange>,
}

/// Labels for transitions recorded in episodic memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionLabel {
    Operation {
        op_id: u64,
        op_type: Option<AISOperationType>,
    },
    Custom(String),
}

impl TransitionLabel {
    pub fn custom(label: impl Into<String>) -> Self {
        TransitionLabel::Custom(label.into())
    }

    pub fn operation(op_id: u64, op_type: AISOperationType) -> Self {
        TransitionLabel::Operation {
            op_id,
            op_type: Some(op_type),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CallFrame {
    pub op_id: u64,
    pub entered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AamCheckpoint {
    pub beliefs: HashMap<String, Value>,
    pub goals: Vec<Goal>,
    #[serde(default)]
    pub capabilities: HashMap<String, CapabilityRecord>,
    #[serde(default)]
    pub goal_tree: GoalTree,
    pub timestamp: DateTime<Utc>,
}

impl AamCheckpoint {
    /// Save checkpoint to a file (JSON serialized).
    pub fn save_to_file(&self, path: &Path) -> Result<(), RuntimeError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| RuntimeError::Serialization(e.to_string()))?;
        std::fs::write(path, json)
            .map_err(|e| RuntimeError::State(format!("Failed to write checkpoint: {}", e)))?;
        Ok(())
    }

    /// Load checkpoint from a file.
    pub fn load_from_file(path: &Path) -> Result<Self, RuntimeError> {
        let json = std::fs::read_to_string(path)
            .map_err(|e| RuntimeError::State(format!("Failed to read checkpoint: {}", e)))?;
        serde_json::from_str(&json).map_err(|e| RuntimeError::Serialization(e.to_string()))
    }
}

