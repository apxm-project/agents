//! Scope policy for hierarchical AAM execution.

use serde::{Deserialize, Serialize};

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

/// Per-dimension scope specification for `child_with_scope`.
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
