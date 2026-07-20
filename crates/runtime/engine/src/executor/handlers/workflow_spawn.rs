//! WORKFLOW_SPAWN operation - execute a child AIR file, artifact, or workflow.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute, get_string_attribute,
};
use crate::context_envelope::{SealedContextRuntimeTransport, resolve as resolve_sealed_context};
use crate::executor::handlers::template::input_names_from_node;
use crate::metadata_keys as metadata;
use apxm_core::constants::{graph::attrs as graph_attrs, runtime::response_keys};
use apxm_core::error::RuntimeError;
use apxm_core::types::context_contracts::{
    CHILD_EXECUTION_ENVELOPE_SCHEMA_VERSION, ChildExecutionEnvelope, ContextFrameDisposition,
    ContextLayerKind, SELECTED_SKILL_DELEGATION_SCOPE,
};
use apxm_core::types::{
    ChildExecutionAdmission, RuntimeCapabilityGrant, WorkflowInvocation, WorkflowInvocationKind,
    WorkflowTarget,
};

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let target_kind = get_string_attribute(node, graph_attrs::TARGET_KIND)?;
    let target = get_string_attribute(node, graph_attrs::TARGET)?;

    if let Some(await_result) = node
        .attributes
        .get(graph_attrs::AWAIT_RESULT)
        .and_then(Value::as_bool)
        && !await_result
    {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "WORKFLOW_SPAWN requires await_result=true".to_string(),
        });
    }

    let target_spec = parse_target(&target_kind, &target)?;
    let resolved_args = resolve_workflow_spawn_args(node, &inputs)?;
    let parent_execution_id = ctx
        .metadata
        .get(metadata::EXECUTION_ID)
        .cloned()
        .unwrap_or_else(|| ctx.execution_id.clone());
    let invocation = WorkflowInvocation {
        kind: WorkflowInvocationKind::WorkflowSpawn,
        target: target_spec,
        args: resolved_args
            .into_iter()
            .map(|(name, value)| {
                let arg_name = name.clone();
                value
                    .to_json()
                    .map(|json| (name, json))
                    .map_err(|e| RuntimeError::Operation {
                        op_type: node.op_type,
                        message: format!(
                            "Failed to serialize WORKFLOW_SPAWN arg '{arg_name}' to JSON: {e}"
                        ),
                    })
            })
            .collect::<Result<HashMap<_, _>>>()?,
        await_result: true,
        session_root: resolve_session_root(ctx, node)?,
        session_dir: None,
        parent_execution_id: Some(parent_execution_id),
        parent_session_dir: ctx.metadata.get(metadata::SESSION_DIR).cloned(),
        parent_scope_id: Some(ctx.scope_id().to_string()),
        spawn_node_id: Some(node.id),
        child_execution_admission: child_execution_admission(ctx)?,
    };

    let result = ctx
        .workflow_spawner
        .spawn_workflow(
            invocation,
            ctx.event_emitter.clone(),
            Some(ctx.cancellation_token.child()),
        )
        .await?;

    let mut payload = HashMap::from([
        (response_keys::RESULT.to_string(), result.value),
        (
            graph_attrs::TARGET_KIND.to_string(),
            Value::String(target_kind),
        ),
        (graph_attrs::TARGET.to_string(), Value::String(target)),
    ]);

    if let Some(session_dir) = result.session_dir {
        payload.insert(
            metadata::SESSION_DIR.to_string(),
            Value::String(session_dir),
        );
    }

    Ok(Value::Object(payload))
}

fn child_execution_admission(ctx: &ExecutionContext) -> Result<ChildExecutionAdmission> {
    let envelope = ctx.metadata.get(metadata::CHILD_EXECUTION_ENVELOPE_V1);
    let transport = ctx
        .metadata
        .get(metadata::CHILD_SEALED_CONTEXT_TRANSPORT_V1);
    match (envelope, transport) {
        (None, None) => Ok(ChildExecutionAdmission::Isolated),
        (Some(_), None) | (None, Some(_)) => Err(child_admission_error(
            "child execution admission must include both the envelope and sealed context transport",
        )),
        (Some(envelope), Some(transport)) => {
            let envelope: ChildExecutionEnvelope =
                serde_json::from_str(envelope).map_err(|error| {
                    child_admission_error(&format!(
                        "child execution envelope is malformed: {error}"
                    ))
                })?;
            let sealed_context: SealedContextRuntimeTransport = serde_json::from_str(transport)
                .map_err(|error| {
                    child_admission_error(&format!(
                        "child sealed context transport is malformed: {error}"
                    ))
                })?;
            validate_child_execution_envelope(ctx, &envelope, &sealed_context)?;
            let runtime_capability_grants = delegated_runtime_capability_grants(ctx, &envelope)?;
            Ok(ChildExecutionAdmission::Delegated {
                envelope: Box::new(envelope),
                sealed_context_transport: transport.clone(),
                runtime_capability_grants,
            })
        }
    }
}

fn validate_child_execution_envelope(
    ctx: &ExecutionContext,
    envelope: &ChildExecutionEnvelope,
    sealed_context: &SealedContextRuntimeTransport,
) -> Result<()> {
    let parent_execution_id = ctx
        .metadata
        .get(metadata::EXECUTION_ID)
        .map(String::as_str)
        .unwrap_or(ctx.execution_id.as_str());
    if envelope.schema_version != CHILD_EXECUTION_ENVELOPE_SCHEMA_VERSION
        || envelope.lineage.parent_execution_id != parent_execution_id
        || envelope.lineage.parent_session_id != ctx.session_id
        || envelope.lineage.child_execution_id.trim().is_empty()
        || envelope.lineage.delegation_ref.trim().is_empty()
        || envelope.program_package.package_id.trim().is_empty()
        || envelope.program_package.entry_flow.trim().is_empty()
        || envelope.context_ref != sealed_context.pin_id
        || envelope.context_digest != sealed_context.sealed_identity_digest
        || envelope.context_policy_ref != sealed_context.model_context.policy_ref
        || envelope.memory_policy_ref.trim().is_empty()
        || envelope.effect_policy_ref.trim().is_empty()
        || envelope.idempotency_key.trim().is_empty()
        || envelope.trace.trace_id.trim().is_empty()
        || envelope.trace.span_id.trim().is_empty()
    {
        return Err(child_admission_error(
            "child execution envelope identity does not match its parent or sealed context",
        ));
    }
    resolve_sealed_context(sealed_context).map_err(|error| {
        child_admission_error(&format!(
            "child sealed context transport is invalid: {error}"
        ))
    })?;
    validate_delegated_selected_skills(envelope, sealed_context)
}

fn validate_delegated_selected_skills(
    envelope: &ChildExecutionEnvelope,
    sealed_context: &SealedContextRuntimeTransport,
) -> Result<()> {
    let selected_frames: Vec<_> = sealed_context
        .model_context
        .frames
        .iter()
        .filter(|frame| frame.layer == ContextLayerKind::SelectedSkillInstructions)
        .collect();
    let selected_layer = sealed_context
        .model_context
        .layers
        .iter()
        .find(|layer| layer.kind == ContextLayerKind::SelectedSkillInstructions)
        .ok_or_else(|| child_admission_error("child sealed context has no selected-skill layer"))?;

    if envelope.selected_skill_delegations.is_empty() {
        return if selected_frames.is_empty() {
            Ok(())
        } else {
            Err(child_admission_error(
                "child sealed context contains selected skills without envelope delegation",
            ))
        };
    }

    let mut delegated_ids = BTreeSet::new();
    let mut delegated_digests = BTreeMap::<String, usize>::new();
    let mut evidence_refs = BTreeSet::new();
    for delegation in &envelope.selected_skill_delegations {
        if delegation.scope != SELECTED_SKILL_DELEGATION_SCOPE
            || delegation.skill_id.trim().is_empty()
            || delegation.root_selection_evidence_ref.trim().is_empty()
            || delegation.manifest_digest != delegation.instruction_digest
            || !delegated_ids.insert(&delegation.skill_id)
        {
            return Err(child_admission_error(
                "child selected-skill delegation is not an exact descendant-scoped root selection",
            ));
        }
        *delegated_digests
            .entry(delegation.instruction_digest.clone())
            .or_default() += 1;
        evidence_refs.insert(&delegation.root_selection_evidence_ref);
    }
    if evidence_refs.len() != 1
        || selected_layer.provenance_ref
            != **evidence_refs
                .first()
                .expect("non-empty delegated evidence set")
        || selected_frames.len() != envelope.selected_skill_delegations.len()
    {
        return Err(child_admission_error(
            "child selected-skill evidence does not match the root selection",
        ));
    }

    let content_digests = sealed_context
        .instruction_contents
        .iter()
        .map(|content| (&content.content_ref, &content.instruction_digest))
        .collect::<HashMap<_, _>>();
    let mut frame_digests = BTreeMap::<String, usize>::new();
    for frame in selected_frames {
        if frame.disposition != ContextFrameDisposition::Prompt
            || content_digests.get(&frame.content_ref) != Some(&&frame.digest)
        {
            return Err(child_admission_error(
                "child selected-skill frame lacks the exact sealed instruction digest",
            ));
        }
        *frame_digests.entry(frame.digest.clone()).or_default() += 1;
    }
    if frame_digests != delegated_digests {
        return Err(child_admission_error(
            "child selected-skill pins do not match the sealed instruction digests",
        ));
    }
    Ok(())
}

fn delegated_runtime_capability_grants(
    ctx: &ExecutionContext,
    envelope: &ChildExecutionEnvelope,
) -> Result<String> {
    let expected: BTreeMap<_, _> = envelope
        .capability_grants
        .iter()
        .map(|grant| (grant.grant_id.as_str(), grant))
        .collect();
    if expected.len() != envelope.capability_grants.len() {
        return Err(child_admission_error(
            "child execution envelope contains duplicate capability grants",
        ));
    }
    if expected.is_empty() {
        return Ok("[]".to_string());
    }
    let parent_grants = ctx
        .metadata
        .get(metadata::CAPABILITY_GRANTS)
        .ok_or_else(|| {
            child_admission_error("child delegated grants are absent from the parent")
        })?;
    let parent_grants: Vec<RuntimeCapabilityGrant> =
        serde_json::from_str(parent_grants).map_err(|error| {
            child_admission_error(&format!(
                "parent runtime capability grants are malformed: {error}"
            ))
        })?;
    let mut delegated = Vec::with_capacity(expected.len());
    for parent_grant in parent_grants {
        let Some(reference) = expected.get(parent_grant.grant_id.as_str()) else {
            continue;
        };
        let operations = parent_grant
            .operations
            .iter()
            .map(|operation| {
                serde_json::to_value(operation)
                    .ok()
                    .and_then(|operation| operation.as_str().map(str::to_string))
            })
            .collect::<Option<BTreeSet<_>>>();
        let expected_operations = reference
            .operations
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if parent_grant.capability_binding != reference.capability_id
            || operations.as_ref() != Some(&expected_operations)
            || !runtime_grant_resources_match(&parent_grant, reference.resources.as_slice())
        {
            return Err(child_admission_error(
                "child capability grant differs from its host-admitted parent reference",
            ));
        }
        delegated.push(parent_grant);
    }
    if delegated.len() != expected.len() {
        return Err(child_admission_error(
            "child capability grant is not present in the host-admitted parent authority",
        ));
    }
    serde_json::to_string(&delegated).map_err(|error| {
        child_admission_error(&format!(
            "failed to serialize delegated capability grants: {error}"
        ))
    })
}

fn runtime_grant_resources_match(
    runtime_grant: &RuntimeCapabilityGrant,
    expected: &[String],
) -> bool {
    match (runtime_grant.resource.uri.as_deref(), expected) {
        (None, []) => true,
        (Some(resource), resources) => resources.len() == 1 && resources[0] == resource,
        _ => false,
    }
}

fn child_admission_error(message: &str) -> RuntimeError {
    RuntimeError::Operation {
        op_type: apxm_core::types::AISOperationType::WorkflowSpawn,
        message: message.to_string(),
    }
}

fn parse_target(target_kind: &str, target: &str) -> Result<WorkflowTarget> {
    WorkflowTarget::from_path_target_kind(target_kind, target.to_string()).map_err(|message| {
        RuntimeError::Operation {
            op_type: apxm_core::types::AISOperationType::WorkflowSpawn,
            message,
        }
    })
}

fn resolve_session_root(ctx: &ExecutionContext, node: &Node) -> Result<Option<String>> {
    if let Some(explicit) = get_optional_string_attribute(node, graph_attrs::SESSION_ROOT)? {
        let trimmed = explicit.trim();
        if trimmed.is_empty() {
            return Err(RuntimeError::Operation {
                op_type: node.op_type,
                message: "WORKFLOW_SPAWN session_root must not be empty".to_string(),
            });
        }
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message:
                "WORKFLOW_SPAWN session_root is server-controlled and may not be supplied by a graph"
                    .to_string(),
        });
    }

    if let Some(root) = ctx.metadata.get(metadata::SESSION_ROOT) {
        return Ok(Some(root.clone()));
    }

    if let Some(session_dir) = ctx.metadata.get(metadata::SESSION_DIR)
        && let Some(parent) = Path::new(session_dir).parent()
    {
        return Ok(Some(parent.to_string_lossy().to_string()));
    }

    Ok(None)
}

fn resolve_workflow_spawn_args(node: &Node, inputs: &[Value]) -> Result<HashMap<String, Value>> {
    let input_names = input_names_from_node(node);
    if !input_names.is_empty() && input_names.len() != inputs.len() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "WORKFLOW_SPAWN input_names length ({}) does not match inputs length ({})",
                input_names.len(),
                inputs.len()
            ),
        });
    }

    let input_bindings: HashMap<String, Value> = input_names
        .iter()
        .cloned()
        .zip(inputs.iter().cloned())
        .collect();

    let mut named_args = HashMap::new();
    if let Some(raw_args) = node.attributes.get(graph_attrs::ARGS) {
        let raw_args = raw_args
            .as_object()
            .ok_or_else(|| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("Attribute {} must be an object", graph_attrs::ARGS),
            })?;
        for (name, raw_value) in raw_args {
            named_args.insert(
                name.clone(),
                resolve_argument_value(raw_value, &input_bindings, node)?,
            );
        }
    }

    for (name, value) in input_bindings {
        named_args.entry(name).or_insert(value);
    }

    Ok(named_args)
}

fn resolve_argument_value(
    raw: &Value,
    input_bindings: &HashMap<String, Value>,
    node: &Node,
) -> Result<Value> {
    match raw {
        Value::String(text) => {
            if let Some(binding) = parse_exact_placeholder(text) {
                return input_bindings.get(binding).cloned().ok_or_else(|| {
                    RuntimeError::Operation {
                        op_type: node.op_type,
                        message: format!(
                            "WORKFLOW_SPAWN argument placeholder '{{{binding}}}' is not present in input_names"
                        ),
                    }
                });
            }
            Ok(raw.clone())
        }
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|item| resolve_argument_value(item, input_bindings, node))
                .collect::<Result<Vec<_>>>()?,
        )),
        Value::Object(map) => Ok(Value::Object(
            map.iter()
                .map(|(key, value)| {
                    resolve_argument_value(value, input_bindings, node)
                        .map(|resolved| (key.clone(), resolved))
                })
                .collect::<Result<HashMap<_, _>>>()?,
        )),
        _ => Ok(raw.clone()),
    }
}

fn parse_exact_placeholder(value: &str) -> Option<&str> {
    let inner = if let Some(stripped) = value.strip_prefix("{{").and_then(|v| v.strip_suffix("}}"))
    {
        stripped
    } else {
        value.strip_prefix('{')?.strip_suffix('}')?
    };
    if inner.is_empty()
        || !inner
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    Some(inner)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use apxm_backends::LLMRegistry;

    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use crate::metadata_keys as metadata;

    async fn test_execution_context() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam)
    }

    fn selected_skill_transport() -> SealedContextRuntimeTransport {
        serde_json::from_value(serde_json::json!({
            "schema_version": "apxm.sealed-context-runtime-transport.v1",
            "pin_id": "context-child",
            "sealed_identity_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "policy": {
                "schema_version": "apxm.context-assembly-policy.v1",
                "policy_id": "policy-child",
                "policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "ordered_layers": [],
                "mandatory_instruction_kinds": [],
                "trusted_instruction_sources": [],
                "data_budgets": {"session_state":0,"memory":0,"beliefs":0,"transcript":0,"retrieval":0,"upstream_outputs":0,"tool_results":0},
                "compaction": {"enabled":false,"deterministic":true,"preserve_provenance":true,"reinject_exact_instructions":true},
                "hooks": {"ordinary_hook_trust":"data","trusted_instruction_hook_policy_ref":"policy://hooks"}
            },
            "model_context": {
                "schema_version": "apxm.model-context-envelope.v1",
                "context_id": "context-child",
                "invocation_id": "child-invocation",
                "policy_ref": "sealed-context-policy:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "layers": [{"kind":"selected_skill_instructions","position":3,"provenance_ref":"selection://root/turn/3","trust":"selected_skill_instruction","redaction":"sensitive","token_budget":1,"token_cost":1,"frame_ids":["skill-frame"]}],
                "frames": [{"frame_id":"skill-frame","layer":"selected_skill_instructions","role":"developer","source_ref":"skill://security-review","digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","trust":"selected_skill_instruction","redaction":"sensitive","scope":"descendants","truncation":"forbidden","token_cost":1,"content_ref":"content://security-review","disposition":"prompt"}],
                "tool_schemas": [],
                "model_requirements": {"tools":false,"structured_output":false,"thinking":false,"vision":false,"locality":"any","minimum_context_tokens":0},
                "sealed_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "instruction_contents": [{"content_ref":"content://security-review","instruction_digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","content":"review"}]
        }))
        .expect("selected-skill transport")
    }

    fn selected_skill_envelope(scope: &str, manifest_digest: &str) -> ChildExecutionEnvelope {
        serde_json::from_value(serde_json::json!({
            "schema_version": "apxm.child-execution-envelope.v1",
            "lineage": {"parent_execution_id":"parent","child_execution_id":"child","parent_session_id":null,"child_session_id":null,"delegation_ref":"delegation://parent/child"},
            "program_package": {"package_id":"pkg-child","digest":"sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","entry_flow":"child"},
            "capability_grants": [],
            "credential_refs": [],
            "selected_skill_delegations": [{"skill_id":"security-review","root_selection_evidence_ref":"selection://root/turn/3","scope":scope,"manifest_digest":manifest_digest,"instruction_digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}],
            "context_ref":"context-child",
            "context_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "context_policy_ref":"sealed-context-policy:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "memory_policy_ref":"policy://memory/child",
            "budgets":{"input_tokens":1,"output_tokens":1,"tool_calls":0,"memory_bytes":0,"concurrency":1,"effects":0,"wall_clock_ms":1},
            "cancellation":{"token_ref":"cancel-child"},
            "effect_policy_ref":"policy://effects/read-only",
            "idempotency_key":"idempotency-child",
            "trace":{"trace_id":"trace-child","span_id":"span-child"}
        }))
        .expect("child execution envelope")
    }

    #[tokio::test]
    async fn child_without_typed_envelope_isolated_from_parent_authority() {
        let mut ctx = test_execution_context().await;
        ctx.metadata.insert(
            metadata::CAPABILITY_GRANTS.to_string(),
            "[{\"grant_id\":\"parent-grant\"}]".to_string(),
        );
        ctx.metadata.insert(
            metadata::SEALED_CONTEXT_TRANSPORT_V1.to_string(),
            "parent-context".to_string(),
        );

        assert_eq!(
            child_execution_admission(&ctx).expect("isolated child admission"),
            ChildExecutionAdmission::Isolated
        );
    }

    #[tokio::test]
    async fn partial_child_admission_is_rejected_before_spawn() {
        let mut ctx = test_execution_context().await;
        ctx.metadata.insert(
            metadata::CHILD_EXECUTION_ENVELOPE_V1.to_string(),
            "{}".to_string(),
        );

        let error = child_execution_admission(&ctx).expect_err("partial envelope must fail");
        assert!(
            error
                .to_string()
                .contains("envelope and sealed context transport")
        );
    }

    #[test]
    fn selected_skill_delegation_rejects_scope_escalation() {
        let envelope = selected_skill_envelope(
            "session",
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        );
        let error = validate_delegated_selected_skills(&envelope, &selected_skill_transport())
            .expect_err("only descendant scope can cross a child boundary");
        assert!(
            error
                .to_string()
                .contains("descendant-scoped root selection")
        );
    }

    #[test]
    fn selected_skill_delegation_rejects_digest_mismatch() {
        let envelope = selected_skill_envelope(
            SELECTED_SKILL_DELEGATION_SCOPE,
            "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        );
        let error = validate_delegated_selected_skills(&envelope, &selected_skill_transport())
            .expect_err("the child must use the exact root pin digest");
        assert!(
            error
                .to_string()
                .contains("descendant-scoped root selection")
        );
    }

    #[test]
    fn selected_skill_delegation_accepts_exact_root_descendant_pin() {
        let envelope = selected_skill_envelope(
            SELECTED_SKILL_DELEGATION_SCOPE,
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        );
        validate_delegated_selected_skills(&envelope, &selected_skill_transport())
            .expect("exact root evidence, descendant scope, and pinned digest are admitted");
    }

    #[tokio::test]
    async fn child_envelope_rejects_a_different_parent_execution() {
        let ctx = test_execution_context().await;
        let envelope = selected_skill_envelope(
            SELECTED_SKILL_DELEGATION_SCOPE,
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        );
        let error = validate_child_execution_envelope(&ctx, &envelope, &selected_skill_transport())
            .expect_err("a child envelope cannot be replayed under another parent");
        assert!(
            error
                .to_string()
                .contains("identity does not match its parent")
        );
    }
}
