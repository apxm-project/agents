//! Permission/approval flow for capability invocation.
//!
//! Provides [`ApprovalStore`] for caching user decisions and
//! [`ApprovalChannel`] for interactive approval requests.

use super::interceptor::InterceptDecision;
use async_trait::async_trait;
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
    pub fn record(
        &self,
        capability: String,
        decision: InterceptDecision,
        scope: ApprovalScope,
    ) {
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

/// Async channel for requesting user approval before executing a capability.
///
/// Implementations may display a TUI prompt, send an HTTP callback, or
/// delegate to any other approval mechanism.
#[async_trait]
pub trait ApprovalChannel: Send + Sync {
    /// Request approval for invoking `capability` with the given `args`.
    ///
    /// `context` provides human-readable information about why the
    /// capability is being invoked. The implementation should return an
    /// [`InterceptDecision`] and an [`ApprovalScope`] indicating how
    /// long the decision should be cached.
    async fn request_approval(
        &self,
        capability: &str,
        args: &serde_json::Value,
        context: &str,
    ) -> (InterceptDecision, ApprovalScope);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_record_and_check_session() {
        let store = ApprovalStore::new();
        assert!(store.is_empty());

        store.record(
            "dangerous_tool".to_string(),
            InterceptDecision::Allow,
            ApprovalScope::Session,
        );
        assert_eq!(store.len(), 1);

        // Session-scoped decisions persist across multiple checks.
        let d1 = store.check("dangerous_tool");
        assert!(matches!(d1, Some(InterceptDecision::Allow)));
        let d2 = store.check("dangerous_tool");
        assert!(matches!(d2, Some(InterceptDecision::Allow)));

        // Still present.
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_store_once_consumed() {
        let store = ApprovalStore::new();
        store.record(
            "one_shot".to_string(),
            InterceptDecision::Allow,
            ApprovalScope::Once,
        );

        // First check consumes the record.
        let d = store.check("one_shot");
        assert!(matches!(d, Some(InterceptDecision::Allow)));
        assert!(store.is_empty());

        // Second check returns None.
        let d = store.check("one_shot");
        assert!(d.is_none());
    }

    #[test]
    fn test_store_always_persists() {
        let store = ApprovalStore::new();
        store.record(
            "safe_tool".to_string(),
            InterceptDecision::Allow,
            ApprovalScope::Always,
        );

        for _ in 0..5 {
            let d = store.check("safe_tool");
            assert!(matches!(d, Some(InterceptDecision::Allow)));
        }
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_store_deny_cached() {
        let store = ApprovalStore::new();
        store.record(
            "blocked".to_string(),
            InterceptDecision::Deny {
                reason: "user said no".to_string(),
            },
            ApprovalScope::Session,
        );

        let d = store.check("blocked");
        match d {
            Some(InterceptDecision::Deny { reason }) => {
                assert_eq!(reason, "user said no");
            }
            other => panic!("expected Deny, got {:?}", other),
        }
    }

    #[test]
    fn test_store_clear() {
        let store = ApprovalStore::new();
        store.record(
            "a".to_string(),
            InterceptDecision::Allow,
            ApprovalScope::Session,
        );
        store.record(
            "b".to_string(),
            InterceptDecision::Allow,
            ApprovalScope::Always,
        );
        assert_eq!(store.len(), 2);

        store.clear();
        assert!(store.is_empty());
        assert!(store.check("a").is_none());
        assert!(store.check("b").is_none());
    }

    #[test]
    fn test_store_overwrite() {
        let store = ApprovalStore::new();
        store.record(
            "tool".to_string(),
            InterceptDecision::Allow,
            ApprovalScope::Session,
        );
        // Overwrite with Deny.
        store.record(
            "tool".to_string(),
            InterceptDecision::Deny {
                reason: "changed mind".to_string(),
            },
            ApprovalScope::Session,
        );
        let d = store.check("tool");
        assert!(matches!(d, Some(InterceptDecision::Deny { .. })));
    }

    #[test]
    fn test_check_unknown_capability() {
        let store = ApprovalStore::new();
        assert!(store.check("nonexistent").is_none());
    }
}
