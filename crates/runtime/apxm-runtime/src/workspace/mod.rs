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
