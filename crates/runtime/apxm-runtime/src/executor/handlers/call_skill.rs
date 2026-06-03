//! `CALL_SKILL` operation - bind and dispatch a child skill by manifest identity.
//!
//! `CALL_SKILL` is the classical-library `ld` analogue for APXM: a parent
//! skill references another skill by `skill_id` (or `skill_id@version`)
//! and the runtime resolves that identity through the host's
//! [`SkillResolver`] at execution time. Implementation contract lives in
//! `docs/design/ais-ops/call-skill.md`. The eight isolation gates this
//! handler must satisfy live in
//! `docs/design/ais-ops/call-skill-isolation-tests.md`.
//!
//! The seven steps from the design spec:
//! 1. **Parse** `skill_id` into `(id, Option<version>)`.
//! 2. **Validate** the id against the canonical pattern (no path
//!    separators, no `..`, no `~`, no control chars, no whitespace
//!    padding, non-empty).
//! 3. **Check** the nested-call depth against
//!    [`apxm_core::constants::call_skill::MAX_CALL_SKILL_DEPTH`].
//! 4. **Resolve** through the host's [`SkillResolver`] (the server's
//!    `SkillLibrary` is the canonical implementation).
//! 5. **Admit** the resolved skill against the parent's capability
//!    grant. This step is performed inside the resolver — the runtime
//!    cannot inspect skill manifests on its own.
//! 6. **Dispatch** the child's entry DAG and await its result.
//! 7. **Namespace** the child's outputs under the `CALL_SKILL` op's
//!    instance id so multiple linked calls in the same parent DAG
//!    cannot collide.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use crate::aam::TransitionLabel;
use crate::executor::handlers::template::input_names_from_node;
use crate::executor::skill_resolver::CallSkillRequest;
use crate::metadata_keys as metadata;
use apxm_core::constants::call_skill::MAX_CALL_SKILL_DEPTH;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Number;

/// Logical capability tag prefix used in `RuntimeError::Capability::capability`
/// for every `CALL_SKILL` failure mode. The suffix after the colon
/// encodes the specific gate (`invalid_id`, `depth_exceeded`,
/// `no_resolver`, plus resolver-supplied tags like `not_found`,
/// `version_not_found`, `capability_widen`, `child_failed`).
const CAPABILITY_TAG: &str = "call_skill";

/// Belief key prefix under which a `CALL_SKILL` invocation's provenance
/// (resolved triple + child outputs) is recorded in the parent AAM.
const CALL_SKILL_OUTPUT_PREFIX: &str = "_call_skill_outputs:";

pub fn execute<'a>(
    ctx: &'a ExecutionContext,
    node: &'a Node,
    inputs: Vec<Value>,
) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
    Box::pin(execute_impl(ctx, node, inputs))
}

async fn execute_impl(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Step 1 + 2: parse and validate the skill id.
    let raw_skill_id = get_string_attribute(node, graph_attrs::SKILL_ID)?;
    let (skill_id, requested_version) = parse_skill_id(&raw_skill_id);
    validate_skill_id(&skill_id, &raw_skill_id)?;
    if let Some(version) = requested_version.as_deref() {
        validate_skill_version(version, &raw_skill_id)?;
    }

    // Step 3: enforce nested call depth.
    let current_depth: usize = ctx
        .metadata
        .get(metadata::CALL_SKILL_DEPTH)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let next_depth = current_depth.saturating_add(1);
    if next_depth > MAX_CALL_SKILL_DEPTH {
        return Err(RuntimeError::Capability {
            capability: format!("{CAPABILITY_TAG}:depth_exceeded:{skill_id}"),
            message: format!(
                "CALL_SKILL depth {next_depth} exceeds MAX_CALL_SKILL_DEPTH={MAX_CALL_SKILL_DEPTH}"
            ),
        });
    }

    // Resolve positional + named args from inputs and the optional
    // `input_names` / `args` attributes.
    let (positional_args, named_args) = resolve_args(node, &inputs)?;

    // Step 4-6: hand off to the resolver. The resolver is responsible
    // for capability admission and for dispatching the child's entry DAG.
    let request = CallSkillRequest {
        skill_id: skill_id.clone(),
        requested_version: requested_version.clone(),
        args: positional_args,
        named_args,
        parent_execution_id: ctx.execution_id.clone(),
        parent_scope_id: ctx.scope_id().to_string(),
        parent_session_dir: ctx.metadata.get(metadata::SESSION_DIR).cloned(),
        spawn_node_id: node.id,
        depth: next_depth,
        parent_side_effect_policy: ctx.metadata.get(metadata::SIDE_EFFECT_POLICY).cloned(),
        parent_visible_skills: ctx.metadata.get(metadata::VISIBLE_SKILLS).cloned(),
    };

    tracing::info!(
        skill_id = %skill_id,
        requested_version = ?requested_version,
        depth = next_depth,
        spawn_node_id = node.id,
        "Dispatching CALL_SKILL"
    );

    let result = ctx.skill_resolver.call_skill(request).await?;

    // Step 7: record provenance evidence (resolved triple + child
    // execution id + namespaced outputs) on the parent AAM.
    record_call_skill_outputs(
        ctx,
        node.id,
        &skill_id,
        requested_version.as_deref(),
        &result,
    );

    Ok(result.return_value)
}

/// Split `id` or `id@version` into its components without trimming.
fn parse_skill_id(raw: &str) -> (String, Option<String>) {
    match raw.split_once('@') {
        Some((id, version)) => (id.to_string(), Some(version.to_string())),
        None => (raw.to_string(), None),
    }
}

/// Reject path-shaped or otherwise hostile skill ids before any
/// filesystem or resolver access. This is gate 1 from the test
/// contract.
fn validate_skill_id(id: &str, raw: &str) -> Result<()> {
    if id.is_empty() {
        return Err(invalid_skill_id(raw, "empty"));
    }
    if id.len() != id.trim().len() {
        return Err(invalid_skill_id(raw, "leading or trailing whitespace"));
    }
    for ch in id.chars() {
        if ch.is_control() {
            return Err(invalid_skill_id(raw, "control character"));
        }
        if ch.is_whitespace() {
            return Err(invalid_skill_id(raw, "embedded whitespace"));
        }
        if ch == '/' || ch == '\\' {
            return Err(invalid_skill_id(raw, "path separator"));
        }
        if ch == '~' {
            return Err(invalid_skill_id(raw, "home expansion attempt"));
        }
    }
    if id.contains("..") {
        return Err(invalid_skill_id(raw, "parent-relative traversal"));
    }
    Ok(())
}

fn validate_skill_version(version: &str, raw: &str) -> Result<()> {
    if version.is_empty() {
        return Err(invalid_skill_id(raw, "empty version after '@'"));
    }
    for ch in version.chars() {
        if ch.is_control() || ch.is_whitespace() {
            return Err(invalid_skill_id(raw, "control or whitespace in version"));
        }
        if ch == '/' || ch == '\\' || ch == '~' {
            return Err(invalid_skill_id(raw, "path-shaped version"));
        }
    }
    Ok(())
}

fn invalid_skill_id(raw: &str, reason: &str) -> RuntimeError {
    RuntimeError::Operation {
        op_type: AISOperationType::CallSkill,
        message: format!("invalid skill_id '{raw}': {reason}"),
    }
}

/// Resolve the positional + named argument vectors a `CALL_SKILL` op
/// forwards to its child. Mirrors `flow_call::resolve_flow_call_args`'s
/// shape but stays lean: the cross-skill ABI is positional-by-default
/// and named arguments are only consulted when the parent provided an
/// `args` map.
fn resolve_args(node: &Node, inputs: &[Value]) -> Result<(Vec<Value>, HashMap<String, Value>)> {
    let input_names = input_names_from_node(node);
    if !input_names.is_empty() && input_names.len() != inputs.len() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "CALL_SKILL input_names length ({}) does not match inputs length ({})",
                input_names.len(),
                inputs.len()
            ),
        });
    }

    let mut named_args: HashMap<String, Value> = input_names
        .iter()
        .cloned()
        .zip(inputs.iter().cloned())
        .collect();

    if let Some(raw_args) = node.attributes.get(graph_attrs::ARGS) {
        let raw_args = raw_args
            .as_object()
            .ok_or_else(|| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("Attribute {} must be an object", graph_attrs::ARGS),
            })?;
        for (name, value) in raw_args {
            named_args.insert(name.clone(), value.clone());
        }
    }

    Ok((inputs.to_vec(), named_args))
}

/// Record a `CALL_SKILL` invocation's provenance (resolved triple +
/// child execution id + child outputs) on the parent's AAM beliefs.
/// The outputs are namespaced under the parent's `CALL_SKILL` node id
/// so multiple linked calls in the same DAG cannot collide.
fn record_call_skill_outputs(
    ctx: &ExecutionContext,
    parent_node_id: u64,
    requested_skill_id: &str,
    requested_version: Option<&str>,
    result: &super::super::skill_resolver::CallSkillResult,
) {
    let resolved = Value::Object(
        vec![
            (
                "skill_id".to_string(),
                Value::String(result.resolved_skill_id.clone()),
            ),
            (
                "version".to_string(),
                Value::String(result.resolved_version.clone()),
            ),
            (
                "artifact_hash".to_string(),
                Value::String(result.resolved_artifact_hash.clone()),
            ),
        ]
        .into_iter()
        .collect(),
    );

    let outputs = Value::Object(
        result
            .child_outputs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );

    let evidence = Value::Object(
        vec![
            ("op".to_string(), Value::String("call_skill".to_string())),
            (
                "parent_node_id".to_string(),
                Value::Number(Number::Integer(parent_node_id as i64)),
            ),
            (
                "requested_skill_id".to_string(),
                Value::String(requested_skill_id.to_string()),
            ),
            (
                "requested_version".to_string(),
                requested_version
                    .map(|v| Value::String(v.to_string()))
                    .unwrap_or(Value::Null),
            ),
            ("resolved".to_string(), resolved),
            (
                "child_execution_id".to_string(),
                Value::String(result.child_execution_id.clone()),
            ),
            (
                "child_session_dir".to_string(),
                result
                    .child_session_dir
                    .as_ref()
                    .map(|s| Value::String(s.clone()))
                    .unwrap_or(Value::Null),
            ),
            ("outputs".to_string(), outputs),
        ]
        .into_iter()
        .collect(),
    );

    ctx.aam.set_belief(
        format!("{CALL_SKILL_OUTPUT_PREFIX}{requested_skill_id}:{parent_node_id}"),
        evidence,
        TransitionLabel::Custom(format!("call_skill_completed:{requested_skill_id}")),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::executor::skill_resolver::{CallSkillResult, NoOpSkillResolver, SkillResolver};
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::types::execution::{Node, NodeMetadata};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Arc;

    async fn make_ctx() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
    }

    fn call_skill_node(skill_id: &str) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::CallSkill,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::SKILL_ID.to_string(),
            Value::String(skill_id.to_string()),
        );
        node
    }

    #[test]
    fn parse_skill_id_handles_bare_and_versioned() {
        assert_eq!(
            parse_skill_id("apxm-orient"),
            ("apxm-orient".to_string(), None)
        );
        assert_eq!(
            parse_skill_id("apxm-orient@0.2.0"),
            ("apxm-orient".to_string(), Some("0.2.0".to_string()))
        );
    }

    #[test]
    fn validate_skill_id_rejects_path_shaped_ids() {
        for bad in [
            "../escape",
            "foo/bar",
            "foo\\bar",
            "~/skill",
            "",
            " skill",
            "skill ",
            "skill\twith-tab",
        ] {
            let (id, _) = parse_skill_id(bad);
            assert!(
                validate_skill_id(&id, bad).is_err(),
                "expected rejection of {bad:?}"
            );
        }
    }

    #[test]
    fn validate_skill_id_rejects_control_chars() {
        let bad = "skill\x00with-null";
        let (id, _) = parse_skill_id(bad);
        assert!(validate_skill_id(&id, bad).is_err());
    }

    #[test]
    fn validate_skill_id_accepts_canonical_ids() {
        for good in ["apxm-orient", "checkout-context-triage", "a", "skill_42"] {
            let (id, _) = parse_skill_id(good);
            assert!(
                validate_skill_id(&id, good).is_ok(),
                "expected acceptance of {good:?}"
            );
        }
    }

    #[tokio::test]
    async fn execute_rejects_path_shaped_skill_id() {
        let ctx = make_ctx().await;
        let node = call_skill_node("../escape");
        let result = execute(&ctx, &node, vec![]).await;
        let err = result.expect_err("expected invalid skill_id");
        assert!(err.to_string().contains("invalid skill_id"), "got: {err}");
    }

    #[tokio::test]
    async fn execute_rejects_when_no_resolver_configured() {
        let ctx = make_ctx().await;
        let node = call_skill_node("apxm-orient");
        let err = execute(&ctx, &node, vec![]).await.expect_err("no resolver");
        let message = err.to_string();
        assert!(
            message.contains("no SkillResolver configured") || message.contains("no_resolver"),
            "got: {message}"
        );
    }

    #[tokio::test]
    async fn execute_enforces_max_depth() {
        let ctx = make_ctx().await.with_metadata(
            metadata::CALL_SKILL_DEPTH.to_string(),
            MAX_CALL_SKILL_DEPTH.to_string(),
        );
        let node = call_skill_node("apxm-orient");
        let err = execute(&ctx, &node, vec![]).await.expect_err("depth");
        assert!(
            err.to_string().contains("MAX_CALL_SKILL_DEPTH"),
            "got: {err}"
        );
    }

    struct StubResolver {
        result: CallSkillResult,
    }

    #[async_trait]
    impl SkillResolver for StubResolver {
        async fn call_skill(
            &self,
            _request: CallSkillRequest,
        ) -> std::result::Result<CallSkillResult, RuntimeError> {
            Ok(self.result.clone())
        }
    }

    #[tokio::test]
    async fn execute_records_resolved_provenance_on_success() {
        let mut child_outputs = HashMap::new();
        child_outputs.insert(
            "result".to_string(),
            Value::String("child-output".to_string()),
        );
        let ctx = make_ctx().await.with_skill_resolver(Arc::new(StubResolver {
            result: CallSkillResult {
                resolved_skill_id: "apxm-orient".to_string(),
                resolved_version: "0.2.0".to_string(),
                resolved_artifact_hash: "blake3:abc123".to_string(),
                child_execution_id: "child-exec".to_string(),
                child_session_dir: None,
                child_outputs,
                return_value: Value::String("child-output".to_string()),
            },
        }));

        let node = call_skill_node("apxm-orient");
        let value = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(value, Value::String("child-output".to_string()));

        let key = format!("{CALL_SKILL_OUTPUT_PREFIX}apxm-orient:1");
        let evidence = ctx.aam.get_belief(&key).expect("provenance belief");
        let Value::Object(map) = evidence else {
            panic!("expected object evidence");
        };
        let Some(Value::Object(resolved)) = map.get("resolved") else {
            panic!("expected resolved triple");
        };
        assert_eq!(
            resolved.get("skill_id"),
            Some(&Value::String("apxm-orient".to_string()))
        );
        assert_eq!(
            resolved.get("version"),
            Some(&Value::String("0.2.0".to_string()))
        );
        assert_eq!(
            resolved.get("artifact_hash"),
            Some(&Value::String("blake3:abc123".to_string()))
        );

        // NoOpSkillResolver returns a "no_resolver" capability error,
        // which we exercise elsewhere; here we verified a configured
        // resolver gets the resolved triple recorded.
        let _ = NoOpSkillResolver; // keep the import alive even if unused above
    }
}
