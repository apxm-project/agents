//! Server-side bridge implementation of [`apxm_runtime::SkillResolver`]
//! for the `CALL_SKILL` op.
//!
//! The runtime emits a [`CallSkillRequest`] describing the child skill the
//! parent wants to link to (by manifest identity, not by raw path). The
//! server-side resolver:
//!
//! 1. Resolves `(skill_id, requested_version)` through [`SkillLibrary`].
//! 2. Loads and admits the resolved manifest against the parent's effective
//!    capability grant (no-widen invariant).
//! 3. Returns the resolved `(skill_id, version, artifact_hash)` triple so the
//!    runtime can record it in the parent's provenance.
//!
//! Step 4 (dispatching the child's entry DAG and surfacing its outputs)
//! lives behind a follow-up integration plug. Today this resolver explicitly
//! refuses to silently dispatch the child: the runtime gets back a typed
//! [`RuntimeError::Capability`] tagged `call_skill:child_dispatch_unwired`
//! with the resolved triple in the message so callers can distinguish "we
//! got as far as resolving and admitting your child, but the dispatcher
//! plug is not installed" from "your id was invalid" or "version not found".
//!
//! This honours the project's no-fallback contract: rather than silently
//! producing a `Value::Null` and claiming success, we hard-fail at exactly
//! the boundary the host has not yet supplied.

use std::collections::HashSet;
use std::sync::Arc;

use apxm_core::error::RuntimeError;
use apxm_runtime::{CallSkillRequest, CallSkillResult, SkillResolver};
use apxm_skill::SkillManifest;
use async_trait::async_trait;

use crate::skills::{SkillLibrary, SkillLookupError};

/// Capability prefix every `CALL_SKILL` failure surfaces under. The suffix
/// after the colon distinguishes the precise gate that rejected the call.
const CAPABILITY_TAG: &str = "call_skill";

/// [`SkillResolver`] backed by [`SkillLibrary`].
pub(crate) struct SkillLibrarySkillResolver {
    library: SkillLibrary,
}

impl SkillLibrarySkillResolver {
    pub(crate) fn new(library: SkillLibrary) -> Self {
        Self { library }
    }
}

#[async_trait]
impl SkillResolver for SkillLibrarySkillResolver {
    async fn call_skill(
        &self,
        request: CallSkillRequest,
    ) -> Result<CallSkillResult, RuntimeError> {
        // Step 1: resolve the manifest identity through the library.
        let requested = match request.requested_version.as_deref() {
            Some(version) => format!("{}@{}", request.skill_id, version),
            None => request.skill_id.clone(),
        };
        let executable = self
            .library
            .find_executable(&requested)
            .map_err(|error| skill_lookup_error(&request.skill_id, &requested, error))?;
        let manifest = executable
            .record
            .manifest
            .as_ref()
            .ok_or_else(|| RuntimeError::Capability {
                capability: format!("{CAPABILITY_TAG}:invalid_manifest:{}", request.skill_id),
                message: format!(
                    "resolved skill '{}' has no valid manifest",
                    request.skill_id
                ),
            })?;
        let artifact_hash = executable
            .record
            .hashes
            .artifact_hash
            .clone()
            .ok_or_else(|| RuntimeError::Capability {
                capability: format!("{CAPABILITY_TAG}:not_compiled:{}", request.skill_id),
                message: format!(
                    "resolved skill '{}' is not compiled (skill.apxmobj missing)",
                    request.skill_id
                ),
            })?;

        // Step 2: capability admission — child must not widen the parent's
        // grant. The parent's grant is not yet plumbed through the
        // CallSkillRequest, so today we enforce only the conservative
        // "no required_capabilities" subset rule when invoked from the
        // default context. The full subset check belongs to the
        // ExecutionContext refactor that ships with the dispatcher plug.
        admit_required_capabilities(manifest)?;

        // Step 4 (dispatch the child DAG and collect its outputs) is the
        // missing host plug. Refuse loudly instead of returning empty
        // outputs and claiming success.
        Err(RuntimeError::Capability {
            capability: format!(
                "{CAPABILITY_TAG}:child_dispatch_unwired:{}",
                manifest.skill_id
            ),
            message: format!(
                "resolved '{}@{}' (artifact_hash={}); child-skill dispatcher is not yet \
                 wired into the server runtime. Resolved manifest is admissible but no \
                 host plug is installed to execute the child entry DAG.",
                manifest.skill_id, manifest.version, artifact_hash
            ),
        })
    }
}

/// Translate a [`SkillLookupError`] into a typed runtime error preserving
/// the spec-defined failure-mode tags (`not_found` vs `ambiguous`).
fn skill_lookup_error(
    skill_id: &str,
    requested: &str,
    error: SkillLookupError,
) -> RuntimeError {
    match error {
        SkillLookupError::NotFound(_) => RuntimeError::Capability {
            capability: format!("{CAPABILITY_TAG}:not_found:{skill_id}"),
            message: format!("skill not found: {requested}"),
        },
        SkillLookupError::Ambiguous(_) => RuntimeError::Capability {
            capability: format!("{CAPABILITY_TAG}:ambiguous:{skill_id}"),
            message: format!(
                "skill id has multiple versions; request {requested}@<version>"
            ),
        },
    }
}

/// Enforce the conservative no-widen rule: until the parent's effective
/// capability grant is threaded through `CallSkillRequest`, refuse any
/// child that requests *any* capability. This is intentionally strict —
/// the right policy is "child.required_capabilities ⊆ parent.grant" but
/// the parent grant is not plumbed yet, so the only safe default is
/// "child must be capability-free".
fn admit_required_capabilities(manifest: &SkillManifest) -> Result<(), RuntimeError> {
    if manifest.required_capabilities.is_empty() {
        return Ok(());
    }
    let mut declared: HashSet<&str> = HashSet::new();
    for capability in &manifest.required_capabilities {
        declared.insert(capability.as_str());
    }
    let summary = declared.iter().copied().collect::<Vec<_>>().join(",");
    Err(RuntimeError::Capability {
        capability: format!(
            "{CAPABILITY_TAG}:capability_widen:{}",
            manifest.skill_id
        ),
        message: format!(
            "child skill '{}' declares required_capabilities=[{}] but parent capability \
             grant is not yet plumbed into CALL_SKILL; refusing widen by default",
            manifest.skill_id, summary
        ),
    })
}

/// Install `library` as the runtime's [`SkillResolver`] on the supplied
/// runtime. Hosts call this once during startup after the library has been
/// scanned.
pub(crate) fn install(runtime: &mut apxm_runtime::Runtime, library: SkillLibrary) {
    runtime.set_skill_resolver(Arc::new(SkillLibrarySkillResolver::new(library)));
}
