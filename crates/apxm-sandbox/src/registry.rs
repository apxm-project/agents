//! Backend registry for sandbox selection.
//!
//! The [`SandboxRegistry`] holds registered backends and selects the best
//! one for a given set of requirements. Host applications register their
//! backends at startup; the APXM runtime queries the registry when it
//! needs to create a sandbox session.
//!
//! **APXM never auto-detects backends.** Detection logic (checking for
//! `bwrap`, `docker`, `wasmtime`, etc.) belongs in the host application.

use crate::backend::SandboxBackend;
use crate::error::SandboxError;
use crate::manifest::SecurityManifest;
use crate::types::{IsolationLevel, SandboxCapabilities};
use std::sync::Arc;

/// Registry that holds available sandbox backends.
///
/// The host application registers backends at startup. The APXM runtime
/// calls [`select`](SandboxRegistry::select) or
/// [`select_for_manifest`](SandboxRegistry::select_for_manifest) to find
/// the best backend for a given execution.
pub struct SandboxRegistry {
    backends: Vec<Arc<dyn SandboxBackend>>,
}

impl SandboxRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
        }
    }

    /// Register a backend. Order matters for tie-breaking:
    /// if two backends offer the same isolation level, the one registered
    /// first is preferred.
    pub fn register(&mut self, backend: Arc<dyn SandboxBackend>) {
        self.backends.push(backend);
    }

    /// Get all registered backends and their capabilities.
    pub fn list(&self) -> Vec<SandboxCapabilities> {
        self.backends.iter().map(|b| b.capabilities()).collect()
    }

    /// How many backends are registered.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// Select the best available backend that meets the minimum isolation level.
    ///
    /// "Best" means: available, meets minimum isolation, and lowest isolation
    /// level among qualifying backends (to avoid overkill).
    pub fn select(
        &self,
        min_isolation: IsolationLevel,
    ) -> Result<Arc<dyn SandboxBackend>, SandboxError> {
        let mut candidates: Vec<_> = self
            .backends
            .iter()
            .filter(|b| b.is_available())
            .filter(|b| b.capabilities().isolation_level >= min_isolation)
            .collect();

        // Sort by isolation level (prefer closest match, not overkill)
        candidates.sort_by_key(|b| b.capabilities().isolation_level);

        candidates
            .first()
            .map(|b| Arc::clone(b))
            .ok_or_else(|| {
                SandboxError::RequirementsNotMet(format!(
                    "no available backend provides at least {min_isolation} isolation \
                     (registered: {})",
                    self.backends
                        .iter()
                        .map(|b| {
                            let c = b.capabilities();
                            format!("{}({})", c.name, c.isolation_level)
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
    }

    /// Select the best backend for a compiler-generated [`SecurityManifest`].
    pub fn select_for_manifest(
        &self,
        manifest: &SecurityManifest,
    ) -> Result<Arc<dyn SandboxBackend>, SandboxError> {
        self.select(manifest.min_isolation)
    }

    /// Get the default backend (first registered, regardless of isolation level).
    ///
    /// Returns `None` if the registry is empty.
    pub fn default_backend(&self) -> Option<Arc<dyn SandboxBackend>> {
        self.backends.first().map(Arc::clone)
    }
}

impl Default for SandboxRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SandboxRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxRegistry")
            .field("backend_count", &self.backends.len())
            .field(
                "backends",
                &self
                    .backends
                    .iter()
                    .map(|b| {
                        let c = b.capabilities();
                        format!("{}({})", c.name, c.isolation_level)
                    })
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::DefaultBackend;
    use crate::types::{ExecResult, SandboxCapabilities};
    use std::time::Duration;

    fn make_backend(name: &str, level: IsolationLevel) -> Arc<dyn SandboxBackend> {
        Arc::new(DefaultBackend::new(
            SandboxCapabilities {
                isolation_level: level,
                supports_filesystem_restriction: false,
                supports_network_restriction: false,
                supports_syscall_filtering: false,
                supports_resource_limits: false,
                name: name.to_string(),
                version: "test".to_string(),
            },
            |_req| async {
                Ok(ExecResult {
                    success: true,
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                    duration: Duration::ZERO,
                    timed_out: false,
                })
            },
        ))
    }

    #[test]
    fn test_empty_registry() {
        let reg = SandboxRegistry::new();
        assert!(reg.is_empty());
        assert!(reg.select(IsolationLevel::None).is_err());
    }

    #[test]
    fn test_register_and_list() {
        let mut reg = SandboxRegistry::new();
        reg.register(make_backend("passthrough", IsolationLevel::None));
        reg.register(make_backend("bwrap", IsolationLevel::Container));

        assert_eq!(reg.len(), 2);
        let caps = reg.list();
        assert_eq!(caps[0].name, "passthrough");
        assert_eq!(caps[1].name, "bwrap");
    }

    #[test]
    fn test_select_minimum() {
        let mut reg = SandboxRegistry::new();
        reg.register(make_backend("passthrough", IsolationLevel::None));
        reg.register(make_backend("bwrap", IsolationLevel::Container));

        // Requesting None should pick passthrough (lowest matching)
        let b = reg.select(IsolationLevel::None).unwrap();
        assert_eq!(b.capabilities().name, "passthrough");

        // Requesting Container should pick bwrap
        let b = reg.select(IsolationLevel::Container).unwrap();
        assert_eq!(b.capabilities().name, "bwrap");

        // Requesting Hypervisor should fail (nothing that high)
        assert!(reg.select(IsolationLevel::Hypervisor).is_err());
    }

    #[test]
    fn test_select_prefers_lowest_qualifying() {
        let mut reg = SandboxRegistry::new();
        reg.register(make_backend("remote", IsolationLevel::Remote));
        reg.register(make_backend("os-level", IsolationLevel::OsLevel));
        reg.register(make_backend("container", IsolationLevel::Container));

        // Requesting OsLevel should pick os-level, not container or remote
        let b = reg.select(IsolationLevel::OsLevel).unwrap();
        assert_eq!(b.capabilities().name, "os-level");
    }

    #[test]
    fn test_select_for_manifest() {
        let mut reg = SandboxRegistry::new();
        reg.register(make_backend("passthrough", IsolationLevel::None));
        reg.register(make_backend("bwrap", IsolationLevel::Container));

        let manifest = SecurityManifest {
            min_isolation: IsolationLevel::Container,
            ..SecurityManifest::default()
        };

        let b = reg.select_for_manifest(&manifest).unwrap();
        assert_eq!(b.capabilities().name, "bwrap");
    }

    #[test]
    fn test_default_backend() {
        let mut reg = SandboxRegistry::new();
        assert!(reg.default_backend().is_none());

        reg.register(make_backend("first", IsolationLevel::None));
        reg.register(make_backend("second", IsolationLevel::Container));

        assert_eq!(reg.default_backend().unwrap().capabilities().name, "first");
    }
}
