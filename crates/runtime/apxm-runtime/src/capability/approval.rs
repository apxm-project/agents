//! Permission/approval flow for capability invocation.
//!
//! Provides [`ApprovalStore`] for caching user decisions.

use super::interceptor::InterceptDecision;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::time::Instant;

/// Scope that determines how long a cached approval decision persists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalScope {
    /// Approval is consumed after a single invocation.
    Once,
    /// Approval lasts for the current session (until [`ApprovalStore::clear`] is called).
    Session,
    /// Approval never expires.
    Always,
}

/// A cached approval record.
#[derive(Debug, Clone)]
pub struct ApprovalRecord {
    /// The capability name this decision applies to.
    pub capability: String,
    /// The cached intercept decision.
    pub decision: InterceptDecision,
    /// Scope governing lifetime of this record.
    pub scope: ApprovalScope,
    /// When this record was created.
    pub created_at: Instant,
}

/// Stores and retrieves cached approval decisions.
///
/// Thread-safe via internal [`RwLock`].
pub struct ApprovalStore {
    decisions: RwLock<HashMap<String, ApprovalRecord>>,
}

impl ApprovalStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            decisions: RwLock::new(HashMap::new()),
        }
    }

    /// Check if we have a cached decision for `capability`.
    ///
    /// Returns `None` if no cached decision exists. If a [`ApprovalScope::Once`]
    /// decision is found it is consumed (removed) and returned.
    pub fn check(&self, capability: &str) -> Option<InterceptDecision> {
        // Fast path: read-only check for Session/Always scopes.
        {
            let guard = self.decisions.read();
            if let Some(record) = guard.get(capability) {
                if record.scope != ApprovalScope::Once {
                    return Some(record.decision.clone());
                }
            }
        }

        // Slow path: write lock to atomically consume Once decisions.
        let mut guard = self.decisions.write();
        if let Some(record) = guard.get(capability) {
            if record.scope == ApprovalScope::Once {
                let decision = record.decision.clone();
                guard.remove(capability);
                return Some(decision);
            }
        }
        None
    }

    /// Record (cache) a decision for `capability`.
    pub fn record(&self, capability: String, decision: InterceptDecision, scope: ApprovalScope) {
        let record = ApprovalRecord {
            capability: capability.clone(),
            decision,
            scope,
            created_at: Instant::now(),
        };
        self.decisions.write().insert(capability, record);
    }

    /// Clear all cached decisions (e.g. when a session ends).
    pub fn clear(&self) {
        self.decisions.write().clear();
    }

    /// Return the number of cached decisions.
    pub fn len(&self) -> usize {
        self.decisions.read().len()
    }

    /// Return `true` if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.decisions.read().is_empty()
    }
}

impl Default for ApprovalStore {
    fn default() -> Self {
        Self::new()
    }
}

