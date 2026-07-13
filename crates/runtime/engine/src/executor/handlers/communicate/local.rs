//! Local in-process COMMUNICATE dispatch via FlowRegistry sub-DAG execution
//! and inline-spawned agent fallback.

use super::super::{
    ExecutionContext, Node, Result, Value, get_string_attribute,
    llm::execute_contextual_node_request,
    target_resolution::resolve_target_or_passthrough,
    template::{input_names_from_node, render_named},
};
use crate::aam::{ScopeSpec, TransitionLabel};
use crate::executor::ExecutorEngine;
use crate::flow_names::COMMUNICATE_FLOWS as COMMUNICATE_FLOW_NAMES;
use crate::metadata_keys as metadata;
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use apxm_core::error::RuntimeError;
use apxm_core::types::AISOperationType;

pub(super) fn message_from_attributes(node: &Node) -> Option<Value> {
    get_string_attribute(node, graph_attrs::MESSAGE)
        .ok()
        .map(Value::String)
        .or_else(|| {
            get_string_attribute(node, graph_attrs::PROMPT)
                .ok()
                .map(Value::String)
        })
        .or_else(|| {
            get_string_attribute(node, graph_attrs::TEMPLATE_STR)
                .ok()
                .map(Value::String)
        })
}

pub(super) fn resolve_message(
    node: &Node,
    protocol: apxm_core::types::CommunicateProtocol,
    inputs: &[Value],
) -> Result<Value> {
    let input_names = input_names_from_node(node);
    let attr_message = message_from_attributes(node);
    let message_input_count = input_names.len();
    let message_inputs = if message_input_count == 0 {
        inputs
    } else {
        &inputs[..message_input_count.min(inputs.len())]
    };

    if let Some(Value::String(template)) = attr_message.as_ref() {
        if !input_names.is_empty() {
            return Ok(Value::String(render_named(
                template,
                message_inputs,
                &input_names,
            )?));
        }
        return Ok(Value::String(template.clone()));
    }

    let fallback = if protocol == apxm_core::types::CommunicateProtocol::Acp {
        message_inputs
            .iter()
            .find(|v| matches!(v, Value::String(_)))
            .cloned()
            .or_else(|| message_inputs.first().cloned())
    } else {
        message_inputs.first().cloned()
    };

    Ok(fallback.or(attr_message).unwrap_or(Value::Null))
}

pub(super) fn communication_llm_operation(node: &Node) -> Result<AISOperationType> {
    let Some(raw_value) = node.attributes.get(graph_attrs::LLM_OPERATION) else {
        return Ok(AISOperationType::Ask);
    };
    let value = raw_value
        .as_string()
        .ok_or_else(|| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "COMMUNICATE '{}' must be a string",
                graph_attrs::LLM_OPERATION
            ),
        })?;
    let operation = value
        .parse::<AISOperationType>()
        .map_err(|_| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "COMMUNICATE has invalid '{}' value '{}'",
                graph_attrs::LLM_OPERATION,
                value
            ),
        })?;
    if !matches!(
        operation,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
    ) {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "COMMUNICATE '{}' must be {}, {}, or {}",
                graph_attrs::LLM_OPERATION,
                AISOperationType::Ask,
                AISOperationType::Think,
                AISOperationType::Reason
            ),
        });
    }
    Ok(operation)
}

pub(super) async fn execute_local(
    ctx: &ExecutionContext,
    node: &Node,
    recipient: &str,
    message: Value,
) -> Result<Value> {
    if recipient.is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "COMMUNICATE requires 'recipient' attribute for local protocol".to_string(),
        });
    }

    // `topic:`/`capability:` targets resolve to a concrete member
    // through the routing pipeline before the exact lookup below; anything
    // else (including bare exact ids) passes through unchanged.
    let resolved_recipient = resolve_target_or_passthrough(&ctx.flow_registry, recipient)?;
    let recipient = resolved_recipient.as_str();

    tracing::info!(
        execution_id = %ctx.execution_id,
        recipient = %recipient,
        "Executing COMMUNICATE operation"
    );

    let label = TransitionLabel::Custom(format!("communicate:{}", recipient));
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::PENDING_COMMUNICATE_PREFIX, recipient),
        Value::Object(
            vec![
                (
                    graph_attrs::RECIPIENT.to_string(),
                    Value::String(recipient.to_string()),
                ),
                (graph_attrs::MESSAGE.to_string(), message.clone()),
            ]
            .into_iter()
            .collect(),
        ),
        label,
    );

    let sub_dag = {
        let mut found = None;
        for flow_name in COMMUNICATE_FLOW_NAMES {
            if let Some(dag) = ctx.flow_registry.get_flow(recipient, flow_name) {
                found = Some(dag);
                break;
            }
        }
        match found {
            Some(dag) => dag,
            None => {
                if let Some(response) =
                    communicate_inline_agent(ctx, node, recipient, &message).await?
                {
                    return Ok(response);
                }

                let available = ctx.flow_registry.flows_for_agent(recipient);
                let hint = if available.is_empty() {
                    let all_flows = ctx.flow_registry.list_flows();
                    if all_flows.is_empty() {
                        format!(
                            "COMMUNICATE target '{}' not found. No agents registered and no \
                             inline SPAWN_AGENT info in STM.",
                            recipient
                        )
                    } else {
                        format!(
                            "Agent '{}' not found. Registered agents: {}",
                            recipient,
                            all_flows
                                .iter()
                                .map(|(a, f)| format!("{}.{}", a, f))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                } else {
                    format!(
                        "Agent '{}' has no 'communicate' or 'main' flow. Available flows: {}",
                        recipient,
                        available.join(", ")
                    )
                };

                return Err(RuntimeError::Operation {
                    op_type: node.op_type,
                    message: hint,
                });
            }
        }
    };

    tracing::debug!(
        recipient = %recipient,
        nodes = sub_dag.nodes.len(),
        "Found flow for recipient agent, executing sub-DAG"
    );

    let child_ctx = ctx
        .child_with_scope(ScopeSpec::snapshot_all())
        .with_metadata(
            metadata::PARENT_EXECUTION_ID.to_string(),
            ctx.execution_id.clone(),
        )
        .with_metadata(
            metadata::COMMUNICATE_SENDER.to_string(),
            ctx.execution_id.clone(),
        )
        .with_metadata(
            metadata::COMMUNICATE_RECIPIENT.to_string(),
            recipient.to_string(),
        );

    let _ = child_ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            child_ctx.scope_id(),
            belief_keys::COMMUNICATE_MESSAGE.to_string(),
            message.clone(),
        )
        .await;

    let engine = ExecutorEngine::new(child_ctx);
    let dag_to_execute = (*sub_dag).clone();

    let result = engine.execute_dag(dag_to_execute).await.map_err(|e| {
        tracing::error!(
            recipient = %recipient,
            error = %e,
            "Inter-agent communication failed"
        );
        RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Communication with agent '{}' failed: {}", recipient, e),
        }
    })?;

    ctx.aam.set_belief(
        format!("{}{}", belief_keys::PENDING_COMMUNICATE_PREFIX, recipient),
        Value::Null,
        TransitionLabel::Custom(format!("communicate_completed:{}", recipient)),
    );

    let response = result
        .results
        .values()
        .find(|v| !matches!(v, Value::Null))
        .cloned()
        .unwrap_or(Value::Null);

    tracing::info!(
        recipient = %recipient,
        duration_ms = result.stats.duration_ms,
        executed_nodes = result.stats.executed_nodes,
        "COMMUNICATE completed successfully"
    );

    Ok(response)
}

/// Fallback dispatch when the recipient agent has no registered flow.
///
/// Looks up `agent_info:<recipient>` in STM (written by SPAWN_AGENT) and, if
/// it has a `system_prompt` and/or `model`, dispatches the message as a
/// one-shot LLM call against that agent's config. Returns `Ok(None)` if no
/// inline agent info is present so the caller can emit the original error.
async fn communicate_inline_agent(
    ctx: &ExecutionContext,
    node: &Node,
    recipient: &str,
    message: &Value,
) -> Result<Option<Value>> {
    let key = format!("{}{}", belief_keys::AGENT_INFO_PREFIX, recipient);
    let agent_info = match super::super::read_stm_with_scope_fallback(ctx, &key).await {
        Some(Value::Object(obj)) => obj,
        _ => return Ok(None),
    };

    let system_prompt = agent_info
        .get(response_keys::SYSTEM_PROMPT)
        .and_then(|v| v.as_string())
        .cloned();
    let backend = agent_info
        .get(response_keys::BACKEND)
        .and_then(|v| v.as_string())
        .cloned();
    let model = agent_info
        .get(response_keys::MODEL)
        .and_then(|v| v.as_string())
        .cloned();

    if system_prompt.is_none() && backend.is_none() && model.is_none() {
        return Ok(None);
    }

    let prompt = message.as_string().cloned().unwrap_or_default();

    let mut request =
        LLMRequest::new(prompt).with_operation_type(communication_llm_operation(node)?);
    if let Some(sp) = system_prompt {
        request = request.with_system_prompt(sp);
    }
    if let Some(b) = backend {
        request = request.with_backend(b);
    }
    if let Some(m) = model {
        request = request.with_model(m);
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        recipient = %recipient,
        "COMMUNICATE dispatching to inline-spawned agent via LLM"
    );

    let response = execute_contextual_node_request(ctx, node, "COMMUNICATE", &request).await?;
    Ok(Some(Value::String(response.content)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::types::{
        Agent, AgentFlow, AgentMetadata, CapabilityDeclaration, DagMetadata, ExecutionDag,
    };
    use std::sync::Arc;

    async fn test_context() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam)
    }

    /// A trivial single-NOP-node flow so a registered agent has a runnable
    /// `communicate`/`main` flow without exercising the scheduler beyond a
    /// NOP.
    fn trivial_dag(flow_name: &str) -> ExecutionDag {
        let mut node = Node::new(1, AISOperationType::Nop);
        node.add_output_token(1);
        ExecutionDag {
            nodes: vec![node],
            edges: Vec::new(),
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: DagMetadata {
                name: Some(flow_name.to_string()),
                is_entry: true,
                parameters: Vec::new(),
            },
        }
    }

    fn agent_with(name: &str, capabilities: &[&str], discoverable: &[&str]) -> Agent {
        let execution_dag = trivial_dag(crate::flow_names::COMMUNICATE);
        let flow = AgentFlow {
            name: crate::flow_names::COMMUNICATE.to_string(),
            is_entry: true,
            parameters: Vec::new(),
            task_dag: None,
            execution_dag,
        };
        Agent {
            name: name.to_string(),
            metadata: AgentMetadata {
                memories: Vec::new(),
                capabilities: capabilities
                    .iter()
                    .map(|c| CapabilityDeclaration {
                        name: c.to_string(),
                        description: None,
                    })
                    .collect(),
                tools: Vec::new(),
                discoverable: discoverable.iter().map(|d| d.to_string()).collect(),
                context: None,
            },
            flows: [(flow.name.clone(), flow)].into_iter().collect(),
        }
    }

    fn node_with(recipient: &str) -> Node {
        let mut node = Node::new(2, AISOperationType::Communicate);
        node.set_attribute(
            graph_attrs::RECIPIENT.to_string(),
            Value::String(recipient.to_string()),
        );
        node
    }

    /// Regression: an exact-id recipient with no registered flow and
    /// no inline STM agent info fails with exactly the same "not found" hint
    /// as before target-grammar resolution was added.
    #[tokio::test]
    async fn exact_id_recipient_unchanged_not_found_error() {
        let ctx = test_context().await;
        let node = node_with("support");

        let err = execute_local(&ctx, &node, "support", Value::String("hi".to_string()))
            .await
            .expect_err("no flow, no STM info");
        match err {
            RuntimeError::Operation { message, .. } => {
                assert!(
                    message.contains("COMMUNICATE target 'support' not found")
                        || message.contains("has no 'communicate' or 'main' flow"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Operation error, got {other:?}"),
        }
    }

    /// `topic:<subject>` resolves to a concrete registered member via
    /// the routing pipeline before the exact flow lookup runs.
    #[tokio::test]
    async fn topic_recipient_resolves_and_executes() {
        let ctx = test_context().await;
        ctx.flow_registry
            .register_agent(agent_with("billing", &[], &["receivables"]));
        let node = node_with("topic:receivables");

        let result = execute_local(
            &ctx,
            &node,
            "topic:receivables",
            Value::String("please reconcile".to_string()),
        )
        .await
        .expect("resolves and executes");
        // The trivial NOP sub-flow returns Null; success (not an error) is
        // the signal that resolution found "billing" and dispatched to it.
        assert_eq!(result, Value::Null);
    }

    /// `capability:<cap-id>` resolves to the declaring member.
    #[tokio::test]
    async fn capability_recipient_resolves_and_executes() {
        let ctx = test_context().await;
        ctx.flow_registry
            .register_agent(agent_with("writer", &["draft_report"], &[]));
        let node = node_with("capability:draft_report");

        let result = execute_local(
            &ctx,
            &node,
            "capability:draft_report",
            Value::String("draft it".to_string()),
        )
        .await
        .expect("resolves and executes");
        assert_eq!(result, Value::Null);
    }

    /// zero candidates for a routed target never falls back to a
    /// broadcast — it's a typed no-route error (platform.md rule 5).
    #[tokio::test]
    async fn topic_recipient_with_no_candidate_returns_no_route_error() {
        let ctx = test_context().await;
        let node = node_with("topic:receivables");

        let err = execute_local(
            &ctx,
            &node,
            "topic:receivables",
            Value::String("hi".to_string()),
        )
        .await
        .expect_err("no agent declares this topic");
        assert!(matches!(err, RuntimeError::NoRouteFound { .. }));
    }
}
