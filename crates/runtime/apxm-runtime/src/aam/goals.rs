//! Goal-state helpers for the Agent Abstract Machine.

use apxm_core::types::goal::{Goal, GoalId, GoalStatus};
pub use apxm_core::{CompletionPolicy, GoalTree};
use priority_queue::PriorityQueue;
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
    Updated {
        before: Goal,
        after: Goal,
    },
    StatusChanged {
        id: GoalId,
        from: GoalStatus,
        to: GoalStatus,
    },
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
