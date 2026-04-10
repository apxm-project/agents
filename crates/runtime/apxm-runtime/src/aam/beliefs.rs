//! Belief-state helpers for the Agent Abstract Machine.
//!
//! Beliefs are still represented as a typed key/value map today, but keeping
//! the map aliases and helper functions here gives the AAM a clear component
//! boundary before we evolve beliefs into a richer scoped store.

use apxm_core::types::values::Value;
use std::collections::HashMap;

/// Concrete belief storage used by the runtime today.
pub type BeliefMap = HashMap<String, Value>;

/// Recorded belief-level changes for one transition.
pub type BeliefChangeSet = HashMap<String, (Option<Value>, Option<Value>)>;

/// Snapshot only a selected subset of belief keys.
pub fn snapshot_subset(beliefs: &BeliefMap, keys: &[String]) -> BeliefMap {
    keys.iter()
        .filter_map(|k| beliefs.get(k).map(|v| (k.clone(), v.clone())))
        .collect()
}

/// Compute the effective belief delta between two snapshots.
pub fn diff_beliefs(
    before_snapshot: &BeliefMap,
    after: &BeliefMap,
    mut explicit_changes: BeliefChangeSet,
) -> BeliefChangeSet {
    for key in before_snapshot.keys().chain(after.keys()) {
        if explicit_changes.contains_key(key) {
            continue;
        }
        let before = before_snapshot.get(key).cloned();
        let after = after.get(key).cloned();
        if before != after {
            explicit_changes.insert(key.clone(), (before, after));
        }
    }
    explicit_changes
}
