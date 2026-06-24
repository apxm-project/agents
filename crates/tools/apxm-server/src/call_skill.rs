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
//! 3. Dispatches the child's entry DAG against the same [`Runtime`] the
//!    parent is executing on, propagating `parent_execution_id`,
//!    `parent_scope_id`, and the nested `call_skill_depth` counter.
//! 4. Returns the resolved `(skill_id, version, artifact_hash)` triple plus
//!    the child execution id and its final node-output map so the runtime
//!    handler can namespace outputs under the parent's `CALL_SKILL` node id
//!    and record provenance.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};

use apxm_artifact::Artifact;
use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use apxm_runtime::{
    CallSkillRequest, CallSkillResult, Runtime, SkillResolver, metadata_keys as metadata,
};
use apxm_skill::{CapabilityPolicy, SkillManifest};
use async_trait::async_trait;

use crate::skills::{ExecutableSkill, SkillLibrary, SkillLookupError};

/// Capability prefix every `CALL_SKILL` failure surfaces under. The suffix
/// after the colon distinguishes the precise gate that rejected the call.
const CAPABILITY_TAG: &str = "call_skill";

/// [`SkillResolver`] backed by [`SkillLibrary`] and the host [`Runtime`].
///
/// The runtime is held by [`Weak`] so the resolver does not extend its
/// lifetime; the install sequence wires the weak reference once the
/// runtime is wrapped in [`Arc`].
pub(crate) struct SkillLibrarySkillResolver {
    library: SkillLibrary,
    runtime: OnceLock<Weak<Runtime>>,
}

impl SkillLibrarySkillResolver {
    pub(crate) fn new(library: SkillLibrary) -> Self {
        Self {
            library,
            runtime: OnceLock::new(),
        }
    }

    /// Attach the host runtime via a weak reference. Must be called once
    /// after the runtime is wrapped in [`Arc`] but before any `CALL_SKILL`
    /// is dispatched. Subsequent calls are no-ops.
    pub(crate) fn attach_runtime(&self, runtime: &Arc<Runtime>) {
        // First-wins by design — the runtime is configured once at startup.
        let _ = self.runtime.set(Arc::downgrade(runtime));
    }

    fn upgrade_runtime(&self, skill_id: &str) -> Result<Arc<Runtime>, RuntimeError> {
        let weak = self.runtime.get().ok_or_else(|| RuntimeError::Capability {
            capability: format!("{CAPABILITY_TAG}:resolver_unattached:{skill_id}"),
            message: format!(
                "SkillLibrarySkillResolver was not attached to a Runtime before \
                 dispatching CALL_SKILL '{skill_id}'"
            ),
        })?;
        weak.upgrade().ok_or_else(|| RuntimeError::Capability {
            capability: format!("{CAPABILITY_TAG}:runtime_dropped:{skill_id}"),
            message: format!(
                "host Runtime has been dropped; cannot dispatch CALL_SKILL '{skill_id}'"
            ),
        })
    }
}

#[async_trait]
impl SkillResolver for SkillLibrarySkillResolver {
    async fn call_skill(&self, request: CallSkillRequest) -> Result<CallSkillResult, RuntimeError> {
        // Step 1: resolve the manifest identity through the library.
        let requested = match request.requested_version.as_deref() {
            Some(version) => format!("{}@{}", request.skill_id, version),
            None => request.skill_id.clone(),
        };
        let executable = self
            .library
            .find_executable(&requested)
            .map_err(|error| skill_lookup_error(&request.skill_id, &requested, error))?;
        let manifest =
            executable
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
        let resolved_skill_id = manifest.skill_id.clone();
        let resolved_version = manifest.version.clone();

        // Step 1b: visible-set gate — target must be in the caller's visible set
        // (shared tier or explicitly imported).
        let lib_id = executable.record.pack.as_ref().map(|p| p.pack_id.clone());
        let shared = manifest.shared
            || self
                .library
                .roots()
                .first()
                .is_some_and(|root| executable.record.package_dir.starts_with(root));
        if !skill_visible(
            request.parent_visible_skills.as_deref(),
            &resolved_skill_id,
            lib_id.as_deref(),
            shared,
        ) {
            return Err(RuntimeError::Capability {
                capability: format!("{CAPABILITY_TAG}:not_visible:{}", request.skill_id),
                message: format!(
                    "skill '{}' is not in the caller's visible set; import its \
                     library or mark it shared",
                    request.skill_id
                ),
            });
        }

        // Step 2: capability admission — the child must not widen the parent's
        // grant. Enforces `child_policy ⊆ parent_policy` via
        // `CapabilityPolicy::admits`, returning the child's policy so it can be
        // propagated to any grandchildren.
        let child_policy = admit_child_policy(&request, manifest)?;

        // Step 3: load the artifact bytes and parse.
        let runtime = self.upgrade_runtime(&request.skill_id)?;
        let artifact = load_child_artifact(&executable)?;

        // Step 4: dispatch the child entry DAG. Args are coerced into
        // strings to match the runtime entry-point ABI; non-string values
        // fail typed rather than silently lossy.
        let args = coerce_args_to_strings(&request)?;
        let parent_metadata = build_child_metadata(&request, &child_policy);
        let child_session_id = derive_child_session_id(&request);

        let child_result = runtime
            .execute_artifact_as_child(
                artifact,
                args,
                Some(child_session_id.clone()),
                None,
                request.parent_session_dir.clone(),
                parent_metadata,
            )
            .await
            .map_err(|error| RuntimeError::Capability {
                capability: format!("{CAPABILITY_TAG}:child_failed:{resolved_skill_id}"),
                message: format!(
                    "child execution of '{resolved_skill_id}@{resolved_version}' failed: {error}"
                ),
            })?;

        let (child_outputs, return_value) = project_child_result(&child_result);

        Ok(CallSkillResult {
            resolved_skill_id,
            resolved_version,
            resolved_artifact_hash: artifact_hash,
            child_execution_id: child_session_id,
            child_session_dir: None,
            child_outputs,
            return_value,
        })
    }
}

/// Read, hash-verify, and parse the child's `.apxmobj`.
fn load_child_artifact(executable: &ExecutableSkill) -> Result<Artifact, RuntimeError> {
    let bytes =
        std::fs::read(&executable.artifact_path).map_err(|error| RuntimeError::Capability {
            capability: format!(
                "{CAPABILITY_TAG}:artifact_read_failed:{}",
                executable
                    .record
                    .skill_id
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_string())
            ),
            message: format!(
                "failed to read child artifact at {}: {error}",
                executable.artifact_path.display()
            ),
        })?;

    let declared_hash = executable
        .record
        .hashes
        .artifact_hash
        .as_deref()
        .ok_or_else(|| RuntimeError::Capability {
            capability: format!(
                "{CAPABILITY_TAG}:missing_artifact_hash:{}",
                executable
                    .record
                    .skill_id
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_string())
            ),
            message: "child artifact has no declared hash to verify against".to_string(),
        })?;
    let actual_hash = format!("blake3:{}", blake3::hash(&bytes).to_hex());
    if !hashes_match(declared_hash, &actual_hash) {
        return Err(RuntimeError::Capability {
            capability: format!(
                "{CAPABILITY_TAG}:artifact_hash_mismatch:{}",
                executable
                    .record
                    .skill_id
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_string())
            ),
            message: format!(
                "child artifact hash mismatch: manifest={declared_hash} actual={actual_hash}"
            ),
        });
    }

    Artifact::from_bytes(&bytes).map_err(|error| RuntimeError::Capability {
        capability: format!(
            "{CAPABILITY_TAG}:artifact_parse_failed:{}",
            executable
                .record
                .skill_id
                .clone()
                .unwrap_or_else(|| "<unknown>".to_string())
        ),
        message: format!("failed to parse child artifact: {error}"),
    })
}

fn hashes_match(declared: &str, actual: &str) -> bool {
    declared.eq_ignore_ascii_case(actual)
}

/// Coerce the parent-supplied [`Value`] args into strings. The runtime
/// entry-flow signature is `Vec<String>`; the parent's positional
/// argument values are typically strings already, but anything else
/// (number, bool, null) is rendered via its JSON form.
fn coerce_args_to_strings(request: &CallSkillRequest) -> Result<Vec<String>, RuntimeError> {
    let mut out = Vec::with_capacity(request.args.len());
    for (index, value) in request.args.iter().enumerate() {
        match value {
            Value::String(s) => out.push(s.clone()),
            Value::Number(_) | Value::Bool(_) | Value::Null => out.push(value.to_string()),
            Value::Array(_) | Value::Object(_) | Value::Token(_) => {
                return Err(RuntimeError::Capability {
                    capability: format!(
                        "{CAPABILITY_TAG}:arg_type_unsupported:{}",
                        request.skill_id
                    ),
                    message: format!(
                        "CALL_SKILL arg #{index} for '{}' is not coerceable into the child's \
                         entry-flow ABI; only strings, numbers, bools, and null are accepted",
                        request.skill_id
                    ),
                });
            }
        }
    }
    Ok(out)
}

/// Build the metadata map the runtime layers onto the child's context.
/// This is the propagation point for the depth counter and the
/// parent-link breadcrumbs the nested-provenance schema relies on.
fn build_child_metadata(
    request: &CallSkillRequest,
    child_policy: &CapabilityPolicy,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert(
        metadata::CALL_SKILL_DEPTH.to_string(),
        request.depth.to_string(),
    );
    map.insert(
        metadata::PARENT_EXECUTION_ID.to_string(),
        request.parent_execution_id.clone(),
    );
    map.insert(
        metadata::PARENT_SCOPE_ID.to_string(),
        request.parent_scope_id.clone(),
    );
    // Propagate the admitted child policy as the grandchildren's parent grant,
    // so a nested CALL_SKILL chain keeps enforcing `descendant ⊆ ancestor`.
    map.insert(
        metadata::SIDE_EFFECT_POLICY.to_string(),
        child_policy.name(),
    );
    // Propagate the visible set unchanged so a nested CALL_SKILL chain stays
    // bounded by the original imports (a child cannot widen what it can see).
    if let Some(visible) = &request.parent_visible_skills {
        map.insert(metadata::VISIBLE_SKILLS.to_string(), visible.clone());
    }
    map
}

/// Returns true if the target skill is in the caller's visible set.
/// `None` is treated the same as an empty import set: only shared skills are visible.
fn skill_visible(
    parent_visible: Option<&str>,
    skill_id: &str,
    library: Option<&str>,
    shared: bool,
) -> bool {
    let csv = parent_visible.unwrap_or_default();
    let imports = csv
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let visible = apxm_skill::discovery::VisibleSet::from_imports(imports);
    visible.sees(&apxm_skill::discovery::SkillCard {
        skill_id: skill_id.to_string(),
        library: library.map(|s| s.to_string()),
        description: String::new(),
        when_to_use: String::new(),
        tags: Vec::new(),
        shared,
    })
}

/// Derive a deterministic child session id from the parent's invocation
/// site so repeated dispatches under the same `(parent_execution, node)`
/// reuse the same lane.
fn derive_child_session_id(request: &CallSkillRequest) -> String {
    format!(
        "{}::call_skill::{}",
        request.parent_execution_id, request.spawn_node_id
    )
}

/// Project the child's [`RuntimeExecutionResult`] into the resolver's
/// [`CallSkillResult`] shape: namespace outputs by stringified child
/// node id and pick the entry/exit token's value as the return value.
fn project_child_result(
    child: &apxm_runtime::RuntimeExecutionResult,
) -> (HashMap<String, Value>, Value) {
    let child_outputs = child
        .all_outputs
        .as_ref()
        .map(|map| {
            map.iter()
                .map(|(node_id, value)| (node_id.to_string(), value.clone()))
                .collect::<HashMap<String, Value>>()
        })
        .unwrap_or_default();

    // The runtime's `results` map is keyed by output token id. For the
    // common single-exit entry flow that is also the child's return
    // value. Pick deterministically (smallest token id) so the choice is
    // reproducible across executions.
    let return_value = child
        .results
        .iter()
        .min_by_key(|(token_id, _)| *token_id)
        .map_or(Value::Null, |(_, value)| value.clone());

    (child_outputs, return_value)
}

/// Translate a [`SkillLookupError`] into a typed runtime error preserving
/// the spec-defined failure-mode tags (`not_found` vs `ambiguous`).
fn skill_lookup_error(skill_id: &str, requested: &str, error: SkillLookupError) -> RuntimeError {
    match error {
        SkillLookupError::NotFound(_) => RuntimeError::Capability {
            capability: format!("{CAPABILITY_TAG}:not_found:{skill_id}"),
            message: format!("skill not found: {requested}"),
        },
        SkillLookupError::Ambiguous(_) => RuntimeError::Capability {
            capability: format!("{CAPABILITY_TAG}:ambiguous:{skill_id}"),
            message: format!("skill id has multiple versions; request {requested}@<version>"),
        },
    }
}

/// Enforce the no-widen rule `child_policy ⊆ parent_policy`.
///
/// The parent's effective grant arrives as `request.parent_side_effect_policy`
/// (seeded at the top-level execution and propagated through
/// [`build_child_metadata`]); `None` is treated as `read_only`. The child's
/// declared policy is parsed from `manifest.side_effect_policy` (also defaulting
/// to `read_only`). A child is admitted iff the parent policy admits it. On
/// success the child's policy is returned so it becomes the grandchildren's
/// parent grant.
fn admit_child_policy(
    request: &CallSkillRequest,
    manifest: &SkillManifest,
) -> Result<CapabilityPolicy, RuntimeError> {
    let parent_policy =
        CapabilityPolicy::from_manifest_value(request.parent_side_effect_policy.as_deref())
            .ok_or_else(|| RuntimeError::Capability {
                capability: format!("{CAPABILITY_TAG}:bad_parent_policy:{}", manifest.skill_id),
                message: format!(
                    "parent side_effect_policy '{}' is not a recognized capability policy",
                    request.parent_side_effect_policy.as_deref().unwrap_or("")
                ),
            })?;

    let child_policy = CapabilityPolicy::from_manifest_value(
        manifest.side_effect_policy.as_deref(),
    )
    .ok_or_else(|| RuntimeError::Capability {
        capability: format!("{CAPABILITY_TAG}:bad_child_policy:{}", manifest.skill_id),
        message: format!(
            "child skill '{}' declares side_effect_policy '{}' which is not a \
                     recognized capability policy",
            manifest.skill_id,
            manifest.side_effect_policy.as_deref().unwrap_or("")
        ),
    })?;

    if !parent_policy.admits(&child_policy) {
        return Err(RuntimeError::Capability {
            capability: format!("{CAPABILITY_TAG}:capability_widen:{}", manifest.skill_id),
            message: format!(
                "child skill '{}' requests policy '{}' which widens beyond the parent grant \
                 '{}'; refusing widen (child must be a subset of parent)",
                manifest.skill_id,
                child_policy.name(),
                parent_policy.name()
            ),
        });
    }

    Ok(child_policy)
}

pub(crate) fn install_unattached(
    runtime: &mut Runtime,
    library: SkillLibrary,
) -> Arc<SkillLibrarySkillResolver> {
    let resolver = Arc::new(SkillLibrarySkillResolver::new(library));
    {
        runtime.set_skill_resolver(Arc::clone(&resolver) as Arc<dyn SkillResolver>);
    }
    resolver
}
