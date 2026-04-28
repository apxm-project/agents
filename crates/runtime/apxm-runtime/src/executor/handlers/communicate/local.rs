//! Local in-process COMMUNICATE dispatch via FlowRegistry sub-DAG execution
//! and inline-spawned agent fallback.

use super::super::{
    ExecutionContext, Node, Result, Value, execute_llm_request, get_string_attribute,
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

    let response = execute_llm_request(ctx, node.id, "COMMUNICATE", &request).await?;
    Ok(Some(Value::String(response.content)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::CommunicateProtocol;

    fn make_node_with_attrs(attrs: Vec<(&str, Value)>) -> Node {
        let mut node = Node::new(1, AISOperationType::Communicate);
        for (k, v) in attrs {
            node.attributes.insert(k.to_string(), v);
        }
        node
    }

    #[test]
    fn resolve_message_uses_template_with_input_names() {
        let node = make_node_with_attrs(vec![
            (
                graph_attrs::MESSAGE,
                Value::String("Hello {name}".to_string()),
            ),
            (
                graph_attrs::INPUT_NAMES,
                Value::Array(vec![Value::String("name".to_string())]),
            ),
        ]);

        let result = resolve_message(
            &node,
            CommunicateProtocol::Local,
            &[Value::String("World".to_string())],
        )
        .unwrap();

        assert_eq!(result, Value::String("Hello World".to_string()));
    }

    #[test]
    fn resolve_message_acp_prefers_string_input_over_first_input() {
        let node = make_node_with_attrs(vec![]);

        let result = resolve_message(
            &node,
            CommunicateProtocol::Acp,
            &[Value::Null, Value::String("real prompt".to_string())],
        )
        .unwrap();

        assert_eq!(result, Value::String("real prompt".to_string()));
    }

    #[test]
    fn communication_llm_operation_defaults_to_ask_when_unset() {
        let node = make_node_with_attrs(vec![]);
        assert_eq!(
            communication_llm_operation(&node).unwrap(),
            AISOperationType::Ask
        );
    }

    #[test]
    fn communication_llm_operation_rejects_non_llm_op() {
        let node = make_node_with_attrs(vec![(
            graph_attrs::LLM_OPERATION,
            Value::String(AISOperationType::Plan.to_string()),
        )]);
        assert!(communication_llm_operation(&node).is_err());
    }
}
