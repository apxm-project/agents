//! Driver/CLI `CALL_SKILL` admission: named, pre-run rejection.
//!
//! The driver/CLI runtime has no `SkillLibrary`-backed skill catalog (see
//! `workspace/agents/crates/orchestration/driver/src/skill_resolver.rs` for
//! the *unrelated* node-workspace `driver::SkillResolver` — do not conflate
//! the two). Giving the driver a full skill-library resolver is out of scope
//! (the shared capability setup); instead this module makes the existing fail-closed behavior
//! deterministic, named, and as early as possible:
//!
//! 1. [`UnsupportedCallSkillResolver`] is installed on every driver/CLI
//!    `Runtime` so a `CALL_SKILL` node that somehow reaches dispatch still
//!    fails with the same fail-closed shape as
//!    `apxm_runtime::NoOpSkillResolver`, but tagged
//!    [`apxm_runtime::executor::skill_resolver::CALL_SKILL_UNSUPPORTED_HERE_TAG`]
//!    so the failure is distinguishable from a merely-misconfigured resolver.
//! 2. [`reject_unsupported_call_skill`] statically scans every DAG about to
//!    be executed for a `CALL_SKILL` node *before* any node dispatches, so a
//!    multi-node artifact fails at admission time with zero partial-execution
//!    side effects instead of failing mid-run on whichever node happens to
//!    reach the `CALL_SKILL` op first.

use apxm_core::error::RuntimeError;
use apxm_core::types::AISOperationType;
use apxm_core::types::execution::ExecutionDag;
use apxm_runtime::executor::skill_resolver::CALL_SKILL_UNSUPPORTED_HERE_TAG;
use apxm_runtime::{CallSkillRequest, CallSkillResult, SkillResolver};
use async_trait::async_trait;

use crate::error::DriverError;

/// Installed as every driver/CLI `Runtime`'s `SkillResolver`. Never resolves
/// a skill; exists solely to give a `CALL_SKILL` that reaches dispatch (e.g.
/// via a path that bypasses [`reject_unsupported_call_skill`]) a named,
/// unambiguous failure instead of the generic `call_skill_no_resolver`
/// message a bare `NoOpSkillResolver` would produce.
pub struct UnsupportedCallSkillResolver;

#[async_trait]
impl SkillResolver for UnsupportedCallSkillResolver {
    async fn call_skill(&self, request: CallSkillRequest) -> Result<CallSkillResult, RuntimeError> {
        Err(RuntimeError::Capability {
            capability: format!("call_skill:{}", request.skill_id),
            message: format!(
                "{CALL_SKILL_UNSUPPORTED_HERE_TAG}: CALL_SKILL is not supported by the driver/CLI \
                 runtime; this artifact must run under apxm-server, which resolves skills via a \
                 SkillLibrary-backed SkillResolver (cannot resolve '{}')",
                request.skill_id
            ),
        })
    }
}

/// Scan every DAG in `dags` for a `CALL_SKILL` node and, if found, fail
/// closed before any node has dispatched. Called at the top of every
/// `RuntimeExecutor` execute entry point so the rejection is an admission-time
/// failure (no partial-execution side effects) rather than a mid-run one.
pub fn reject_unsupported_call_skill(dags: &[ExecutionDag]) -> Result<(), DriverError> {
    for dag in dags {
        for node in &dag.nodes {
            if node.op_type == AISOperationType::CallSkill {
                return Err(DriverError::Driver(format!(
                    "{CALL_SKILL_UNSUPPORTED_HERE_TAG}: CALL_SKILL (node {}) is not supported by \
                     the driver/CLI runtime; this artifact must run under apxm-server, which \
                     resolves skills via a SkillLibrary-backed SkillResolver",
                    node.id
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::execution::Node;

    fn dag_with(op_type: AISOperationType) -> ExecutionDag {
        let mut dag = ExecutionDag::new();
        dag.nodes.push(Node::new(1, op_type));
        dag
    }

    #[test]
    fn admits_dags_without_call_skill() {
        let dags = vec![
            dag_with(AISOperationType::Nop),
            dag_with(AISOperationType::Think),
        ];
        assert!(reject_unsupported_call_skill(&dags).is_ok());
    }

    #[test]
    fn rejects_call_skill_before_any_dispatch() {
        let dags = vec![
            dag_with(AISOperationType::Nop),
            dag_with(AISOperationType::CallSkill),
        ];
        let error = reject_unsupported_call_skill(&dags).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(CALL_SKILL_UNSUPPORTED_HERE_TAG),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn unsupported_resolver_fails_named_and_fail_closed() {
        let resolver = UnsupportedCallSkillResolver;
        let request = CallSkillRequest {
            skill_id: "demo".to_string(),
            requested_version: None,
            args: Vec::new(),
            named_args: Default::default(),
            parent_execution_id: "exec-1".to_string(),
            parent_scope_id: "scope-1".to_string(),
            parent_session_dir: None,
            spawn_node_id: 1,
            depth: 0,
            parent_side_effect_policy: None,
            parent_visible_skills: None,
        };
        let error = resolver.call_skill(request).await.unwrap_err();
        match error {
            RuntimeError::Capability { message, .. } => {
                assert!(message.contains(CALL_SKILL_UNSUPPORTED_HERE_TAG));
            }
            other => panic!("expected Capability error, got {other:?}"),
        }
    }
}
