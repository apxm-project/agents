//! Session-scoped approval grant cache.
//!
//! Maps `(session_id, capability + args fingerprint)` to a cached allow decision
//! so "approve for session" suppresses repeat prompts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use apxm_core::types::values::Value;
use blake3::Hasher;

/// Stable digest of a capability invocation (tool name + canonical args).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GrantFingerprint(pub String);

impl GrantFingerprint {
    /// Build a fingerprint from `capability_id` and invocation args.
    pub fn from_invocation(capability_id: &str, args: &HashMap<String, Value>) -> Self {
        let mut keys: Vec<_> = args.keys().collect();
        keys.sort();
        let mut hasher = Hasher::new();
        hasher.update(capability_id.as_bytes());
        hasher.update(b"\0");
        for key in keys {
            hasher.update(key.as_bytes());
            hasher.update(b"=");
            if let Some(value) = args.get(key)
                && let Ok(json) = value.to_json()
                && let Ok(bytes) = serde_json::to_vec(&json)
            {
                hasher.update(&bytes);
            }
            hasher.update(b";");
        }
        Self(hasher.finalize().to_hex().to_string())
    }
}

/// Per-session set of approved invocation fingerprints.
#[derive(Debug, Default)]
struct SessionGrants {
    fingerprints: std::collections::HashSet<GrantFingerprint>,
}

/// Process-global session grant cache keyed by `session_id`.
#[derive(Debug, Default, Clone)]
pub struct SessionGrantCache {
    inner: Arc<Mutex<HashMap<String, SessionGrants>>>,
}

impl SessionGrantCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Shared singleton for server-side permission checks (mirrors session ledger registry).
    pub fn global() -> &'static Self {
        static CACHE: OnceLock<SessionGrantCache> = OnceLock::new();
        CACHE.get_or_init(SessionGrantCache::new)
    }

    /// Whether `session_id` has a cached allow for this invocation fingerprint.
    pub fn is_granted(&self, session_id: &str, fingerprint: &GrantFingerprint) -> bool {
        self.inner
            .lock()
            .expect("grant cache poisoned")
            .get(session_id)
            .is_some_and(|grants| grants.fingerprints.contains(fingerprint))
    }

    /// Record a session-scoped allow for `fingerprint`.
    pub fn record_session_grant(&self, session_id: &str, fingerprint: GrantFingerprint) {
        self.inner
            .lock()
            .expect("grant cache poisoned")
            .entry(session_id.to_string())
            .or_default()
            .fingerprints
            .insert(fingerprint);
    }

    /// Drop cached grants when a session ends.
    pub fn clear_session(&self, session_id: &str) {
        self.inner
            .lock()
            .expect("grant cache poisoned")
            .remove(session_id);
    }

    /// Number of cached fingerprints for a session (test/diagnostic).
    pub fn grant_count(&self, session_id: &str) -> usize {
        self.inner
            .lock()
            .expect("grant cache poisoned")
            .get(session_id)
            .map_or(0, |g| g.fingerprints.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::values::Value;

    #[test]
    fn fingerprint_is_stable_for_same_args() {
        let mut args = HashMap::new();
        args.insert("path".to_string(), Value::String("/tmp/a".to_string()));
        let a = GrantFingerprint::from_invocation("write_file", &args);
        let b = GrantFingerprint::from_invocation("write_file", &args);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_differs_for_different_args() {
        let mut a_args = HashMap::new();
        a_args.insert("path".to_string(), Value::String("/tmp/a".to_string()));
        let mut b_args = HashMap::new();
        b_args.insert("path".to_string(), Value::String("/tmp/b".to_string()));
        assert_ne!(
            GrantFingerprint::from_invocation("write_file", &a_args),
            GrantFingerprint::from_invocation("write_file", &b_args)
        );
    }

    #[test]
    fn session_grant_suppresses_repeat_fingerprint() {
        let cache = SessionGrantCache::new();
        let session = "sess-grant-cache";
        let fp = GrantFingerprint::from_invocation("write_file", &HashMap::new());
        assert!(!cache.is_granted(session, &fp));
        cache.record_session_grant(session, fp.clone());
        assert!(cache.is_granted(session, &fp));
        cache.clear_session(session);
        assert!(!cache.is_granted(session, &fp));
    }
}
