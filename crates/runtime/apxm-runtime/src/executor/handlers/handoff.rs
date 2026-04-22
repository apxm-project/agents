//! HANDOFF operation — transfer execution from one agent to another
//!
//! Reads source/target agent names from attributes. If `transfer_state` is true
//! (the default), copies context-stack frames from the source agent into the
//! target agent's sub-flow so it can continue with full conversational context.
//! Emits `HANDOFF_START` / `HANDOFF_END` events with span continuity.

use super::{
    ExecutionContext, Node, Result, Value, execute_llm_request, get_string_attribute,
    read_stm_with_scope_fallback,
};
use crate::aam::{ScopeSpec, TransitionLabel};
use crate::executor::ExecutorEngine;
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, metadata, response_keys};
use apxm_core::error::RuntimeError;

/// Well-known flow names tried when looking up the target agent.
const HANDOFF_FLOW_NAMES: &[&str] = &["communicate", "main"];

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let source = get_string_attribute(node, graph_attrs::HANDOFF_FROM)?;
    let target = get_string_attribute(node, graph_attrs::HANDOFF_TO)?;

    // transfer_state defaults to true when not specified
    let transfer_state = node
        .attributes
        .get(graph_attrs::TRANSFER_STATE)
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let payload = inputs
        .first()
        .cloned()
        .or_else(|| {
            node.attributes
                .get("payload")
                .and_then(|v| v.as_string())
                .map(|s| Value::String(s.to_string()))
        })
        .unwrap_or(Value::Null);

    tracing::info!(
        execution_id = %ctx.execution_id,
        source = %source,
        target = %target,
        transfer_state = transfer_state,
        "Executing HANDOFF operation"
    );

    // Emit HANDOFF_START event
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_operation_start(node.id, "HANDOFF_START");
    }

    // Record the handoff in AAM beliefs
    let label = TransitionLabel::Custom(format!("handoff:{}→{}", source, target));
    ctx.aam.set_belief(
        format!("handoff:{}→{}", source, target),
        Value::Object(
            vec![
                (
                    graph_attrs::HANDOFF_FROM.to_string(),
                    Value::String(source.clone()),
                ),
                (
                    graph_attrs::HANDOFF_TO.to_string(),
                    Value::String(target.clone()),
                ),
                (
                    graph_attrs::TRANSFER_STATE.to_string(),
                    Value::Bool(transfer_state),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        label,
    );

    // Resolve the target agent. Prefer a registered flow; otherwise fall back
    // to an inline-spawned agent (SPAWN_AGENT in the current flow stamps
    // instructions+model into STM agent_info).
    let sub_dag = {
        let mut found = None;
        for flow_name in HANDOFF_FLOW_NAMES {
            if let Some(dag) = ctx.flow_registry.get_flow(&target, flow_name) {
                found = Some(dag);
                break;
            }
        }
        match found {
            Some(dag) => Some(dag),
            None => {
                if let Some(response) =
                    handoff_inline_agent(ctx, node, &source, &target, &payload).await?
                {
                    finalize_handoff(ctx, node, &source, &target, response.clone(), 0);
                    return Ok(response);
                }
                let available = ctx.flow_registry.flows_for_agent(&target);
                let hint = if available.is_empty() {
                    let all_flows = ctx.flow_registry.list_flows();
                    if all_flows.is_empty() {
                        format!(
                            "HANDOFF target '{}' not found. No agents registered and no inline \
                             SPAWN_AGENT info in STM.",
                            target
                        )
                    } else {
                        format!(
                            "HANDOFF target '{}' not found. Registered agents: {}",
                            target,
                            all_flows
                                .iter()
                                .map(|(a, f)| format!("{}.{}", a, f))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                } else {
                    format!(
                        "HANDOFF target '{}' has no 'communicate' or 'main' flow. Available: {}",
                        target,
                        available.join(", ")
                    )
                };
                return Err(RuntimeError::Operation {
                    op_type: node.op_type,
                    message: hint,
                });
            }
        }
    }
    .expect("sub_dag is Some at this point");

    // Build a child context for the target agent
    let scope_spec = if transfer_state {
        ScopeSpec::snapshot_all()
    } else {
        ScopeSpec::default()
    };

    let child_ctx = ctx
        .child_with_scope(scope_spec)
        .with_metadata(
            metadata::PARENT_EXECUTION_ID.to_string(),
            ctx.execution_id.clone(),
        )
        .with_metadata("handoff_from".to_string(), source.clone())
        .with_metadata("handoff_to".to_string(), target.clone());

    // Inject the payload into STM so the target flow can access it
    let _ = child_ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            child_ctx.scope_id(),
            belief_keys::COMMUNICATE_MESSAGE.to_string(),
            payload.clone(),
        )
        .await;

    // If transfer_state is true and we have a context stack, assemble context
    // from the source agent and write it into the child's STM so the target
    // agent can see the source's conversational history.
    if transfer_state {
        if let Some(ref stack) = ctx.context_stack {
            let assembly = stack.assemble(
                node.id,
                "default",
                apxm_core::constants::runtime::context_stack::DEFAULT_PROMPT_BUDGET_TOKENS,
            );
            if !assembly.frames.is_empty() {
                let context_text = format!("{}", assembly);
                let _ = child_ctx
                    .memory
                    .write_scoped(
                        crate::memory::MemorySpace::Stm,
                        child_ctx.scope_id(),
                        "handoff_context".to_string(),
                        Value::String(context_text),
                    )
                    .await;
            }
        }
    }

    // Execute the target agent's flow
    let engine = ExecutorEngine::new(child_ctx);
    let dag_to_execute = (*sub_dag).clone();

    let result = engine.execute_dag(dag_to_execute).await.map_err(|e| {
        tracing::error!(
            source = %source,
            target = %target,
            error = %e,
            "HANDOFF execution failed"
        );
        RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "Handoff from '{}' to '{}' failed: {}",
                source, target, e
            ),
        }
    })?;

    // Extract the response from the target flow's exit nodes
    let response = result
        .results
        .values()
        .find(|v| !matches!(v, Value::Null))
        .cloned()
        .unwrap_or(Value::Null);

    finalize_handoff(
        ctx,
        node,
        &source,
        &target,
        response.clone(),
        result.stats.duration_ms,
    );
    Ok(response)
}

/// Emit HANDOFF_END, clear the in-flight belief, log completion. Shared by the
/// flow-DAG path and the inline-agent fallback so both produce identical traces.
fn finalize_handoff(
    ctx: &ExecutionContext,
    node: &Node,
    source: &str,
    target: &str,
    _response: Value,
    duration_ms: u128,
) {
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_operation_end(
            node.id,
            "HANDOFF_END",
            std::time::Duration::ZERO, // actual duration tracked by dispatcher
            true,
            None,
            None,
        );
    }

    ctx.aam.set_belief(
        format!("handoff:{}→{}", source, target),
        Value::Null,
        TransitionLabel::Custom(format!("handoff_completed:{}→{}", source, target)),
    );

    tracing::info!(
        execution_id = %ctx.execution_id,
        source = %source,
        target = %target,
        duration_ms = duration_ms,
        "HANDOFF completed successfully"
    );
}

/// Fallback dispatch when the target agent has no registered flow.
///
/// Looks up `agent_info:<target>` in STM (written by SPAWN_AGENT) and, if it
/// has a `system_prompt` and/or `model`, dispatches the payload as a one-shot
/// LLM ASK against that agent's config. Returns `Ok(None)` if no inline agent
/// info is present so the caller can emit the original "not found" error.
async fn handoff_inline_agent(
    ctx: &ExecutionContext,
    node: &Node,
    source: &str,
    target: &str,
    payload: &Value,
) -> Result<Option<Value>> {
    let key = format!("{}{}", belief_keys::AGENT_INFO_PREFIX, target);
    let agent_info = match read_stm_with_scope_fallback(ctx, &key).await {
        Some(Value::Object(obj)) => obj,
        _ => return Ok(None),
    };

    let system_prompt = agent_info
        .get(response_keys::SYSTEM_PROMPT)
        .and_then(|v| v.as_string())
        .cloned();
    let model = agent_info
        .get(response_keys::MODEL)
        .and_then(|v| v.as_string())
        .cloned();

    // Require at least one of system_prompt/model — pure metadata-only entries
    // (e.g. ACP subprocess agents) shouldn't be auto-dispatched as LLMs.
    if system_prompt.is_none() && model.is_none() {
        return Ok(None);
    }

    let prompt = payload
        .as_string()
        .cloned()
        .unwrap_or_default();

    let mut request = LLMRequest::new(prompt).with_operation_type(node.op_type);
    if let Some(sp) = system_prompt {
        request = request.with_system_prompt(sp);
    }
    if let Some(m) = model {
        request = request.with_model(m);
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        source = %source,
        target = %target,
        "HANDOFF dispatching to inline-spawned agent via LLM"
    );

    let response = execute_llm_request(ctx, node.id, "HANDOFF", &request).await?;
    Ok(Some(Value::String(response.content)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::capability::flow_registry::FlowRegistry;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::types::{
        execution::{ExecutionDag, NodeMetadata},
        operations::AISOperationType,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_echo_dag() -> ExecutionDag {
        let mut const_node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        const_node.attributes.insert(
            graph_attrs::VALUE.to_string(),
            Value::String("handoff response".to_string()),
        );
        ExecutionDag {
            nodes: vec![const_node],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn test_handoff_basic() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        flow_registry.register_flow("target_agent", "communicate", create_echo_dag());

        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new());
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Handoff,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::HANDOFF_FROM.to_string(),
            Value::String("source_agent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::HANDOFF_TO.to_string(),
            Value::String("target_agent".to_string()),
        );

        let result = execute(&ctx, &node, vec![Value::String("hello from source".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("handoff response".to_string()));
    }

    #[tokio::test]
    async fn test_handoff_target_not_found() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new());

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Handoff,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::HANDOFF_FROM.to_string(),
            Value::String("source".to_string()),
        );
        node.attributes.insert(
            graph_attrs::HANDOFF_TO.to_string(),
            Value::String("nonexistent".to_string()),
        );

        let err = execute(&ctx, &node, vec![]).await.unwrap_err();
        assert!(err.to_string().contains("HANDOFF target"));
    }

    #[tokio::test]
    async fn test_handoff_no_transfer_state() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        flow_registry.register_flow("target_agent", "communicate", create_echo_dag());

        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new());
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Handoff,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::HANDOFF_FROM.to_string(),
            Value::String("source_agent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::HANDOFF_TO.to_string(),
            Value::String("target_agent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::TRANSFER_STATE.to_string(),
            Value::Bool(false),
        );

        let result = execute(&ctx, &node, vec![Value::String("msg".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("handoff response".to_string()));
    }
}
