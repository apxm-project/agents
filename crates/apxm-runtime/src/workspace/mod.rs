//! Workspace control plane for multi-agent coordination.
//!
//! Provides a [`ScopeRegistry`] that tracks active scopes and their AAM
//! instances, and a [`WorkspaceManager`] that coordinates multi-agent
//! execution across scopes.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::aam::{Aam, ScopeSpec};

// ---------------------------------------------------------------------------
// ScopeEntry
// ---------------------------------------------------------------------------

/// A single registered scope with its associated AAM and specification.
#[derive(Clone)]
pub struct ScopeEntry {
    /// Unique identifier for this scope.
    pub scope_id: String,
    /// Optional parent scope (root scopes have `None`).
    pub parent_id: Option<String>,
    /// The AAM instance governing this scope.
    pub aam: Aam,
    /// The scope specification used when this scope was created.
    pub spec: ScopeSpec,
}

impl std::fmt::Debug for ScopeEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopeEntry")
            .field("scope_id", &self.scope_id)
            .field("parent_id", &self.parent_id)
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// ScopeRegistry
// ---------------------------------------------------------------------------

/// Registry tracking active scopes and their AAM instances.
///
/// Thread-safe via interior `RwLock`. All public methods acquire the lock
/// internally so callers never need to worry about synchronization.
pub struct ScopeRegistry {
    scopes: RwLock<HashMap<String, ScopeEntry>>,
}

impl std::fmt::Debug for ScopeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let scopes = self.scopes.read();
        f.debug_struct("ScopeRegistry")
            .field("scope_count", &scopes.len())
            .finish()
    }
}

impl Default for ScopeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ScopeRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            scopes: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new scope. Overwrites any existing entry with the same `id`.
    pub fn register(&self, id: String, parent: Option<String>, aam: Aam, spec: ScopeSpec) {
        let entry = ScopeEntry {
            scope_id: id.clone(),
            parent_id: parent,
            aam,
            spec,
        };
        self.scopes.write().insert(id, entry);
    }

    /// Look up a scope by id, returning a clone of its entry.
    pub fn get(&self, id: &str) -> Option<ScopeEntry> {
        self.scopes.read().get(id).cloned()
    }

    /// Return the ids of all scopes whose `parent_id` matches `parent_id`.
    pub fn children_of(&self, parent_id: &str) -> Vec<String> {
        self.scopes
            .read()
            .values()
            .filter(|e| e.parent_id.as_deref() == Some(parent_id))
            .map(|e| e.scope_id.clone())
            .collect()
    }

    /// Remove and return the scope with the given id, if it exists.
    pub fn remove(&self, id: &str) -> Option<ScopeEntry> {
        self.scopes.write().remove(id)
    }

    /// Return the number of registered scopes.
    pub fn len(&self) -> usize {
        self.scopes.read().len()
    }

    /// Returns `true` if the registry contains no scopes.
    pub fn is_empty(&self) -> bool {
        self.scopes.read().is_empty()
    }

    /// Return the ids of all registered scopes.
    pub fn scope_ids(&self) -> Vec<String> {
        self.scopes.read().keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// WorkspaceManager
// ---------------------------------------------------------------------------

/// Workspace manager coordinating multi-agent execution.
///
/// Currently provides access to the [`ScopeRegistry`]. Future work will
/// add Materializer, StateProjector, and PolicyEngine integration.
#[derive(Debug)]
pub struct WorkspaceManager {
    /// The scope registry tracking all active scopes.
    pub scope_registry: Arc<ScopeRegistry>,
}

impl Default for WorkspaceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceManager {
    /// Create a new workspace manager with an empty scope registry.
    pub fn new() -> Self {
        Self {
            scope_registry: Arc::new(ScopeRegistry::new()),
        }
    }

    /// Create a workspace manager wrapping an existing scope registry.
    pub fn with_registry(registry: Arc<ScopeRegistry>) -> Self {
        Self {
            scope_registry: registry,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::{Aam, ScopePolicy, ScopeSpec};

    fn default_spec() -> ScopeSpec {
        ScopeSpec {
            beliefs: ScopePolicy::Inherit,
            capabilities: ScopePolicy::Inherit,
            goals: ScopePolicy::Inherit,
        }
    }

    #[test]
    fn test_register_and_get() {
        let registry = ScopeRegistry::new();
        let aam = Aam::new();
        registry.register("scope-1".into(), None, aam, default_spec());

        let entry = registry.get("scope-1").expect("scope should exist");
        assert_eq!(entry.scope_id, "scope-1");
        assert!(entry.parent_id.is_none());
    }

    #[test]
    fn test_get_missing_returns_none() {
        let registry = ScopeRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_children_of() {
        let registry = ScopeRegistry::new();
        registry.register("root".into(), None, Aam::new(), default_spec());
        registry.register(
            "child-a".into(),
            Some("root".into()),
            Aam::new(),
            default_spec(),
        );
        registry.register(
            "child-b".into(),
            Some("root".into()),
            Aam::new(),
            default_spec(),
        );
        registry.register("orphan".into(), None, Aam::new(), default_spec());

        let mut children = registry.children_of("root");
        children.sort();
        assert_eq!(children, vec!["child-a", "child-b"]);

        assert!(registry.children_of("orphan").is_empty());
    }

    #[test]
    fn test_remove() {
        let registry = ScopeRegistry::new();
        registry.register("scope-1".into(), None, Aam::new(), default_spec());
        assert_eq!(registry.len(), 1);

        let removed = registry.remove("scope-1");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().scope_id, "scope-1");
        assert_eq!(registry.len(), 0);

        // Removing again returns None
        assert!(registry.remove("scope-1").is_none());
    }

    #[test]
    fn test_overwrite_existing_scope() {
        let registry = ScopeRegistry::new();
        registry.register("s".into(), None, Aam::new(), default_spec());
        registry.register(
            "s".into(),
            Some("parent".into()),
            Aam::new(),
            default_spec(),
        );

        let entry = registry.get("s").unwrap();
        assert_eq!(entry.parent_id.as_deref(), Some("parent"));
        // Still only one entry
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_len_and_is_empty() {
        let registry = ScopeRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        registry.register("a".into(), None, Aam::new(), default_spec());
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_scope_ids() {
        let registry = ScopeRegistry::new();
        registry.register("alpha".into(), None, Aam::new(), default_spec());
        registry.register("beta".into(), None, Aam::new(), default_spec());

        let mut ids = registry.scope_ids();
        ids.sort();
        assert_eq!(ids, vec!["alpha", "beta"]);
    }

    #[test]
    fn test_workspace_manager_new() {
        let mgr = WorkspaceManager::new();
        assert!(mgr.scope_registry.is_empty());
    }

    #[test]
    fn test_workspace_manager_with_registry() {
        let registry = Arc::new(ScopeRegistry::new());
        registry.register("s1".into(), None, Aam::new(), default_spec());

        let mgr = WorkspaceManager::with_registry(registry.clone());
        assert_eq!(mgr.scope_registry.len(), 1);
        assert!(mgr.scope_registry.get("s1").is_some());
    }
}
