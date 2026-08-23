//! Backend registry for sandbox selection.
//!
//! The [`SandboxRegistry`] holds registered backends and selects the best
//! one for a given set of requirements. Host applications register their
//! backends at startup; the APXM runtime queries the registry when it
//! needs to create a sandbox session.
//!
//! **APXM never auto-detects backends.** Detection logic (checking for
//! `bwrap`, `docker`, `wasmtime`, etc.) belongs in the host application.

use super::backend::{SandboxBackend, ValidationResult};
use super::error::SandboxError;
use super::manifest::{SandboxRequirements, SecurityManifest};
use super::types::{ExecRequest, IsolationLevel, SandboxCapabilities};
use std::sync::Arc;

const REJECTED_UNAVAILABLE_SUFFIX: &str = " unavailable";
const REJECTED_UNSUPPORTED_SEPARATOR: &str = " unsupported: ";
const NO_AVAILABLE_BACKEND_PREFIX: &str = "no available backend provides at least ";
const NO_AVAILABLE_BACKEND_MIDDLE: &str = " isolation (registered: ";
const NO_COMPATIBLE_BACKEND_PREFIX: &str = "no compatible backend for request requiring at least ";
const NO_COMPATIBLE_BACKEND_MIDDLE: &str = " isolation (registered: ";
const NO_COMPATIBLE_BACKEND_REJECTED_SEPARATOR: &str = "; rejected: ";
const NO_COMPATIBLE_BACKEND_SUFFIX: &str = ")";
const INVALID_MANIFEST_PREFIX: &str = "invalid security manifest: ";
const MANIFEST_REJECTED_SEPARATOR: &str = "; rejected: ";

/// Selected backend together with its validation outcome for a request.
pub struct SandboxSelection {
    pub backend: Arc<dyn SandboxBackend>,
    pub validation: ValidationResult,
}

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
                let registered = self
                    .backends
                    .iter()
                    .map(|b| {
                        let c = b.capabilities();
                        format!("{}({})", c.name, c.isolation_level)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                SandboxError::RequirementsNotMet(format!(
                    "{NO_AVAILABLE_BACKEND_PREFIX}{min_isolation}{NO_AVAILABLE_BACKEND_MIDDLE}{registered}{NO_COMPATIBLE_BACKEND_SUFFIX}"
                ))
            })
    }

    /// Select the best backend for a concrete execution request.
    ///
    /// Selection prefers:
    /// 1. backends whose isolation level meets or exceeds `min_isolation`
    /// 2. fully validated backends over degraded ones
    /// 3. the closest matching isolation level among qualifying backends
    pub fn select_for_request(
        &self,
        request: &ExecRequest,
    ) -> Result<SandboxSelection, SandboxError> {
        let min_isolation = request.min_isolation;

        #[derive(Clone)]
        struct Candidate {
            index: usize,
            backend: Arc<dyn SandboxBackend>,
            capabilities: SandboxCapabilities,
            validation: ValidationResult,
        }

        let mut validated = Vec::new();
        let mut rejected = Vec::new();

        for (index, backend) in self.backends.iter().enumerate() {
            if !backend.is_available() {
                let caps = backend.capabilities();
                rejected.push(format!(
                    "{name}{REJECTED_UNAVAILABLE_SUFFIX}",
                    name = caps.name
                ));
                continue;
            }

            let capabilities = backend.capabilities();
            let validation = backend.validate(request);
            match &validation {
                ValidationResult::Unsupported { reason } => {
                    rejected.push(format!(
                        "{name}{REJECTED_UNSUPPORTED_SEPARATOR}{reason}",
                        name = capabilities.name
                    ));
                }
                ValidationResult::Ok | ValidationResult::Degraded { .. } => {
                    validated.push(Candidate {
                        index,
                        backend: Arc::clone(backend),
                        capabilities,
                        validation,
                    });
                }
            }
        }

        let choose_candidate =
            |mut candidates: Vec<Candidate>, prefer_lowest_isolation: bool| -> Option<Candidate> {
                candidates.sort_by(|left, right| {
                    let left_validation_rank = match left.validation {
                        ValidationResult::Ok => 0_u8,
                        ValidationResult::Degraded { .. } => 1,
                        ValidationResult::Unsupported { .. } => 2,
                    };
                    let right_validation_rank = match right.validation {
                        ValidationResult::Ok => 0_u8,
                        ValidationResult::Degraded { .. } => 1,
                        ValidationResult::Unsupported { .. } => 2,
                    };

                    left_validation_rank
                        .cmp(&right_validation_rank)
                        .then_with(|| {
                            if prefer_lowest_isolation {
                                left.capabilities
                                    .isolation_level
                                    .cmp(&right.capabilities.isolation_level)
                            } else {
                                right
                                    .capabilities
                                    .isolation_level
                                    .cmp(&left.capabilities.isolation_level)
                            }
                        })
                        .then_with(|| left.index.cmp(&right.index))
                });
                candidates.into_iter().next()
            };

        let mut meets_min = Vec::new();

        for candidate in validated {
            if candidate.capabilities.isolation_level >= min_isolation {
                meets_min.push(candidate);
            }
        }

        let selected = choose_candidate(meets_min, true).ok_or_else(|| {
                let registered = self
                    .backends
                    .iter()
                    .map(|b| {
                        let c = b.capabilities();
                        format!("{}({})", c.name, c.isolation_level)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let rejected_text = if rejected.is_empty() {
                    "none".to_string()
                } else {
                    rejected.join("; ")
                };
            SandboxError::RequirementsNotMet(format!(
                "{NO_COMPATIBLE_BACKEND_PREFIX}{min_isolation}{NO_COMPATIBLE_BACKEND_MIDDLE}{registered}{NO_COMPATIBLE_BACKEND_REJECTED_SEPARATOR}{rejected_text}{NO_COMPATIBLE_BACKEND_SUFFIX}",
                min_isolation = request.min_isolation,
            ))
        })?;

        Ok(SandboxSelection {
            backend: selected.backend,
            validation: selected.validation,
        })
    }

    /// Select the best backend for a compiler-generated [`SecurityManifest`].
    pub fn select_for_manifest(
        &self,
        manifest: &SecurityManifest,
    ) -> Result<Arc<dyn SandboxBackend>, SandboxError> {
        Ok(self.select_for_manifest_with_validation(manifest)?.backend)
    }

    /// Select a backend only after validating the complete typed manifest
    /// requirements and the backend's full enforcement surface.
    pub fn select_for_manifest_with_validation(
        &self,
        manifest: &SecurityManifest,
    ) -> Result<SandboxSelection, SandboxError> {
        let requirements = manifest.requirements().map_err(|error| {
            SandboxError::RequirementsNotMet(format!("{INVALID_MANIFEST_PREFIX}{error}"))
        })?;
        self.select_for_requirements(&requirements)
    }

    fn select_for_requirements(
        &self,
        requirements: &SandboxRequirements,
    ) -> Result<SandboxSelection, SandboxError> {
        let mut rejected = Vec::new();

        for backend in &self.backends {
            let capabilities = backend.capabilities();
            if !backend.is_available() {
                rejected.push(format!(
                    "{}{}",
                    capabilities.name, REJECTED_UNAVAILABLE_SUFFIX
                ));
                continue;
            }

            let missing = requirements.missing_backend_capabilities(&capabilities);
            if !missing.is_empty() {
                rejected.push(format!(
                    "{} unsupported: missing {}",
                    capabilities.name,
                    missing.join(", ")
                ));
                continue;
            }

            match backend.validate_manifest(requirements) {
                ValidationResult::Ok => {
                    return Ok(SandboxSelection {
                        backend: Arc::clone(backend),
                        validation: ValidationResult::Ok,
                    });
                }
                ValidationResult::Degraded { warnings } => rejected.push(format!(
                    "{} degraded: {}",
                    capabilities.name,
                    warnings.join(", ")
                )),
                ValidationResult::Unsupported { reason } => rejected.push(format!(
                    "{}{}{}",
                    capabilities.name, REJECTED_UNSUPPORTED_SEPARATOR, reason
                )),
            }
        }

        let registered = self
            .backends
            .iter()
            .map(|backend| {
                let capabilities = backend.capabilities();
                format!("{}({})", capabilities.name, capabilities.isolation_level)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let rejected_text = if rejected.is_empty() {
            "none".to_string()
        } else {
            rejected.join(MANIFEST_REJECTED_SEPARATOR)
        };
        Err(SandboxError::RequirementsNotMet(format!(
            "{NO_COMPATIBLE_BACKEND_PREFIX}{}{NO_COMPATIBLE_BACKEND_MIDDLE}{registered}{NO_COMPATIBLE_BACKEND_REJECTED_SEPARATOR}{rejected_text}{NO_COMPATIBLE_BACKEND_SUFFIX}",
            requirements.min_isolation
        )))
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

impl std::fmt::Debug for SandboxSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxSelection")
            .field("backend", &self.backend.capabilities())
            .field("validation", &self.validation)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::backend::DefaultBackend;
    use crate::sandbox::manifest::tier;
    use crate::sandbox::{ExecResult, NodeSandboxReq};
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    fn capabilities(name: &str) -> SandboxCapabilities {
        SandboxCapabilities {
            isolation_level: IsolationLevel::OsLevel,
            supports_filesystem_restriction: true,
            supports_network_restriction: true,
            supports_syscall_filtering: true,
            supports_process_restriction: true,
            supports_resource_limits: true,
            name: name.to_string(),
            version: "test".to_string(),
        }
    }

    fn io_manifest() -> SecurityManifest {
        SecurityManifest {
            max_tier: tier::IO,
            min_isolation: IsolationLevel::OsLevel,
            node_requirements: vec![NodeSandboxReq {
                node_id: 1,
                op: AISOperationType::CapabilityInvoke,
                tier: tier::IO,
                min_isolation: IsolationLevel::OsLevel,
                capabilities_used: vec!["demo".to_string()],
            }],
            needs_network: true,
            needs_filesystem_write: true,
            needs_process_spawn: true,
            tool_capabilities_used: vec!["demo".to_string()],
        }
    }

    #[test]
    fn manifest_selection_validates_the_complete_typed_requirements() {
        let backend = DefaultBackend::new(capabilities("complete"), |_request| async {
            Ok(ExecResult {
                success: true,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                duration: std::time::Duration::ZERO,
                timed_out: false,
            })
        });
        let mut registry = SandboxRegistry::new();
        registry.register(Arc::new(backend));

        let selection = registry
            .select_for_manifest_with_validation(&io_manifest())
            .expect("complete backend satisfies complete manifest");
        assert!(matches!(selection.validation, ValidationResult::Ok));
    }

    #[test]
    fn manifest_selection_fails_closed_for_incomplete_os_enforcement() {
        let mut incomplete = capabilities("incomplete");
        incomplete.supports_resource_limits = false;
        let backend = DefaultBackend::new(incomplete, |_request| async {
            Ok(ExecResult {
                success: true,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                duration: std::time::Duration::ZERO,
                timed_out: false,
            })
        });
        let mut registry = SandboxRegistry::new();
        registry.register(Arc::new(backend));

        let error = match registry.select_for_manifest(&io_manifest()) {
            Ok(_) => panic!("resource enforcement cannot be silently degraded"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("resource-limits"));
    }
}
