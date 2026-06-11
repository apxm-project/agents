//! WORKFLOW_SPAWN operation - execute a child AIR file, artifact, or workflow.

use std::collections::HashMap;
use std::path::Path;

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute, get_string_attribute,
};
use crate::executor::handlers::template::input_names_from_node;
use crate::metadata_keys as metadata;
use apxm_core::constants::{graph::attrs as graph_attrs, runtime::response_keys};
use apxm_core::error::RuntimeError;
use apxm_core::types::{WorkflowInvocation, WorkflowInvocationKind, WorkflowTarget};

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
        parent_execution_id: Some(ctx.execution_id.clone()),
        parent_session_dir: ctx.metadata.get(metadata::SESSION_DIR).cloned(),
        parent_scope_id: Some(ctx.scope_id().to_string()),
        spawn_node_id: Some(node.id),
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
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::types::{
        AISOperationType, Node, WORKFLOW_TARGET_KIND_AIR_PATH, WORKFLOW_TARGET_KIND_ARTIFACT_PATH,
        WORKFLOW_TARGET_KIND_WORKFLOW_PATH,
    };
    use std::sync::Arc;

    async fn test_ctx() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
    }

    fn workflow_spawn_node() -> Node {
        let mut node = Node::new(1, AISOperationType::WorkflowSpawn);
        node.set_attribute(
            graph_attrs::TARGET_KIND.to_string(),
            Value::String(WORKFLOW_TARGET_KIND_WORKFLOW_PATH.to_string()),
        );
        node.set_attribute(
            graph_attrs::TARGET.to_string(),
            Value::String("workflow/review.apxmw".to_string()),
        );
        node
    }

    #[test]
    fn parse_target_uses_shared_kind_contract() {
        assert!(matches!(
            parse_target(WORKFLOW_TARGET_KIND_AIR_PATH, "steps/review.air"),
            Ok(WorkflowTarget::AirPath { .. })
        ));
        assert!(matches!(
            parse_target(WORKFLOW_TARGET_KIND_ARTIFACT_PATH, "build/review.apxmobj"),
            Ok(WorkflowTarget::ArtifactPath { .. })
        ));
        assert!(matches!(
            parse_target(WORKFLOW_TARGET_KIND_WORKFLOW_PATH, "workflow/review.apxmw"),
            Ok(WorkflowTarget::WorkflowPath { .. })
        ));
    }

    #[test]
    fn parse_target_rejects_unknown_target_kind() {
        let error =
            parse_target("registered_flow", "review").expect_err("must reject unknown kind");
        let RuntimeError::Operation { message, .. } = error else {
            panic!("expected operation error");
        };
        assert!(message.contains(WORKFLOW_TARGET_KIND_AIR_PATH));
        assert!(message.contains(WORKFLOW_TARGET_KIND_ARTIFACT_PATH));
        assert!(message.contains(WORKFLOW_TARGET_KIND_WORKFLOW_PATH));
    }

    #[tokio::test]
    async fn explicit_session_root_is_rejected() {
        let ctx = test_ctx()
            .await
            .with_metadata(
                metadata::SESSION_ROOT.to_string(),
                "/tmp/parent-root".to_string(),
            )
            .with_metadata(
                metadata::SESSION_DIR.to_string(),
                "/tmp/parent-root/run-1".to_string(),
            );
        let mut node = workflow_spawn_node();
        node.set_attribute(
            graph_attrs::SESSION_ROOT.to_string(),
            Value::String("/tmp/explicit-root".to_string()),
        );

        let error = resolve_session_root(&ctx, &node).expect_err("session root must be rejected");
        let RuntimeError::Operation { message, .. } = error else {
            panic!("expected operation error");
        };
        assert!(message.contains("server-controlled"));
    }

    #[tokio::test]
    async fn session_root_falls_back_to_parent_root_then_parent_session_dir_parent() {
        let ctx_with_root = test_ctx().await.with_metadata(
            metadata::SESSION_ROOT.to_string(),
            "/tmp/parent-root".to_string(),
        );
        assert_eq!(
            resolve_session_root(&ctx_with_root, &workflow_spawn_node()).expect("session root"),
            Some("/tmp/parent-root".to_string())
        );

        let ctx_from_dir = test_ctx().await.with_metadata(
            metadata::SESSION_DIR.to_string(),
            "/tmp/custom-sessions/execution-1".to_string(),
        );
        assert_eq!(
            resolve_session_root(&ctx_from_dir, &workflow_spawn_node()).expect("session root"),
            Some("/tmp/custom-sessions".to_string())
        );
    }

    #[test]
    fn resolve_workflow_spawn_args_rejects_unknown_placeholders() {
        let mut node = workflow_spawn_node();
        node.set_attribute(
            graph_attrs::ARGS.to_string(),
            Value::Object(HashMap::from([(
                "topic".to_string(),
                Value::String("{missing}".to_string()),
            )])),
        );
        node.set_attribute(
            graph_attrs::INPUT_NAMES.to_string(),
            Value::Array(vec![Value::String("known".to_string())]),
        );

        let error = resolve_workflow_spawn_args(&node, &[Value::String("apxm".to_string())])
            .expect_err("must reject unresolved placeholders");
        let RuntimeError::Operation { message, .. } = error else {
            panic!("expected operation error");
        };
        assert!(message.contains("{missing}"));
    }
}
