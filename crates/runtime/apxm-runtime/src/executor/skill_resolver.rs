//! Skill resolution bridge for the `CALL_SKILL` op.
//!
//! `apxm-runtime` does not depend on `apxm-server`, so cross-skill linking
//! is expressed as a trait the host implements. The server-side
//! `SkillLibrary` is the canonical implementation; in-process tests can
//! provide a fake.

use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use async_trait::async_trait;
use std::collections::HashMap;

/// Request the runtime hands to the host for a single `CALL_SKILL`
/// invocation.
#[derive(Debug, Clone)]
pub struct CallSkillRequest {
    /// Resolved skill identifier without any version suffix.
    pub skill_id: String,
    /// Optional explicit version. `None` resolves to `@latest`.
    pub requested_version: Option<String>,
    /// Positional arguments forwarded to the child's entry flow.
    pub args: Vec<Value>,
    /// Optional named arguments forwarded to the child's entry flow.
    pub named_args: HashMap<String, Value>,
    /// Parent execution id (for provenance).
    pub parent_execution_id: String,
    /// Parent scope id (propagated to the child execution).
    pub parent_scope_id: String,
    /// Parent session directory, when available.
    pub parent_session_dir: Option<String>,
    /// Node id of the `CALL_SKILL` op in the parent DAG.
    pub spawn_node_id: u64,
    /// Current `CALL_SKILL` nesting depth (parent included).
    pub depth: usize,
    /// The parent execution's effective side-effect policy (the wire form of
    /// [`apxm_skill::CapabilityPolicy`], e.g. `read_only`, `sandboxed`,
    /// `broader[...]`). Threaded so the admission layer can enforce the
    /// no-widen rule (`child ⊆ parent`) instead of refusing every capability.
    /// `None` is treated as `read_only` by the admission layer.
    pub parent_side_effect_policy: Option<String>,
}

/// Successful result of a `CALL_SKILL` invocation.
#[derive(Debug, Clone)]
pub struct CallSkillResult {
    /// Resolved skill id (mirrors the request).
    pub resolved_skill_id: String,
    /// Resolved version that the host actually loaded.
    pub resolved_version: String,
    /// `blake3:<hex>` content hash of the artifact that was executed.
    pub resolved_artifact_hash: String,
    /// Child execution id (assigned by the host).
    pub child_execution_id: String,
    /// Child session directory, when available.
    pub child_session_dir: Option<String>,
    /// Final node output map of the child execution, namespaced under
    /// the parent's `CALL_SKILL` node id by the caller.
    pub child_outputs: HashMap<String, Value>,
    /// Primary return value (the child's exit value).
    pub return_value: Value,
}

/// Bridge trait through which the runtime delegates cross-skill linking
/// to the host. The server's `SkillLibrary` is the canonical impl.
#[async_trait]
pub trait SkillResolver: Send + Sync {
    /// Resolve, admit, and execute a child skill by manifest identity.
    ///
    /// Implementations are expected to:
    /// 1. Resolve `(skill_id, requested_version)` through their manifest
    ///    catalog to a concrete `.apxmobj` + version + artifact hash.
    /// 2. Check that the child's `required_capabilities` are a subset of
    ///    the parent's effective grant (no-widen invariant).
    /// 3. Dispatch the child's entry DAG with `args` / `named_args`,
    ///    propagating `parent_scope_id` so child events carry the
    ///    parent's scope.
    /// 4. Return the resolved triple, child execution id, and the
    ///    child's final outputs.
    async fn call_skill(&self, request: CallSkillRequest) -> Result<CallSkillResult, RuntimeError>;
}

/// No-op bridge for runtimes that do not have a `SkillLibrary` attached.
///
/// `CALL_SKILL` against this resolver fails with a `Capability` error
/// tagged `call_skill_no_resolver` so callers can distinguish "no
/// library configured" from any other failure mode.
pub struct NoOpSkillResolver;

#[async_trait]
impl SkillResolver for NoOpSkillResolver {
    async fn call_skill(&self, request: CallSkillRequest) -> Result<CallSkillResult, RuntimeError> {
        Err(RuntimeError::Capability {
            capability: format!("call_skill:{}", request.skill_id),
            message: format!(
                "no SkillResolver configured on this ExecutionContext; cannot resolve '{}'",
                request.skill_id
            ),
        })
    }
}
