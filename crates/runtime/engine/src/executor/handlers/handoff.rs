//! HANDOFF operation — transfer execution from one agent to another
//!
//! Reads source/target agent names from attributes. Handoffs are isolated by
//! default. `transfer_state=true` is an explicit, typed opt-in for the limited
//! AAM snapshot and rendered context-frame payload transfer; credentials,
//! grants, Agent Skill selections, budgets, prompts, and parent metadata never
//! cross the child boundary. Emits typed lifecycle events with span continuity.

use super::{
    ExecutionContext, Node, Result, Value, get_string_attribute,
    llm::{
        context_planning_runtime_error, context_profile_for_node, execute_contextual_node_request,
    },
    read_stm_with_scope_fallback,
};
use crate::aam::TransitionLabel;
use crate::executor::{ExecutorEngine, HandoffTransfer};
use crate::flow_names::HANDOFF_FLOWS as HANDOFF_FLOW_NAMES;
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use apxm_core::error::RuntimeError;
use apxm_core::types::operations::AISOperationType;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let source = get_string_attribute(node, graph_attrs::HANDOFF_FROM)?;
    let target = get_string_attribute(node, graph_attrs::HANDOFF_TO)?;

    // State transfer is fail-closed. Only an explicit typed `true` opts in.
    let transfer = if node
        .attributes
        .get(graph_attrs::TRANSFER_STATE)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        HandoffTransfer::explicit_state_transfer()
    } else {
        HandoffTransfer::default()
    };

    let payload = inputs
        .first()
        .cloned()
        .or_else(|| {
            node.attributes
                .get(graph_attrs::PAYLOAD)
                .and_then(|v| v.as_string())
                .map(|s| Value::String(s.to_string()))
        })
        .unwrap_or(Value::Null);

    tracing::info!(
        execution_id = %ctx.execution_id,
        source = %source,
        target = %target,
        transfer_state = transfer.transfers_state(),
        "Executing HANDOFF operation"
    );

    // Emit HANDOFF_START event
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_operation_start(node.id, AISOperationType::Handoff);
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
                    Value::Bool(transfer.transfers_state()),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        label,
    );

    // Resolve a target-owned flow or an explicit inline-agent configuration.
    // The latter is target lookup data, not inherited child state.
    let sub_dag = HANDOFF_FLOW_NAMES
        .iter()
        .find_map(|flow_name| ctx.flow_registry.get_flow(&target, flow_name));
    let inline_agent = if sub_dag.is_none() {
        resolve_inline_agent_config(ctx, &target).await
    } else {
        None
    };

    if sub_dag.is_none() && inline_agent.is_none() {
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
                        .map(|(agent, flow)| format!("{agent}.{flow}"))
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

    let child_ctx = ctx.handoff_child(transfer);

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

    // Context frames cross only through the explicit typed transfer. The child
    // never receives the parent ContextStack itself, so it cannot demand-page
    // additional prompt, memory, or selected-skill state later.
    if transfer.transfers_context_frames() {
        if let Some(ref stack) = ctx.context_stack {
            let profile = context_profile_for_node(node)
                .map_err(|error| context_planning_runtime_error(node, error))?;
            let assembly = stack
                .assemble(node.id, profile)
                .map_err(|error| context_planning_runtime_error(node, error))?;
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

    if let Some(inline_agent) = inline_agent {
        let response =
            handoff_inline_agent(&child_ctx, node, &source, &target, &payload, inline_agent)
                .await?;
        finalize_handoff(ctx, node, &source, &target, response.clone(), 0);
        return Ok(response);
    }

    // Execute the target agent's registered flow in the isolated child.
    let engine = ExecutorEngine::new(child_ctx);
    let dag_to_execute = (*sub_dag.expect("registered flow was resolved")).clone();

    let result = engine.execute_dag(dag_to_execute).await.map_err(|e| {
        tracing::error!(
            source = %source,
            target = %target,
            error = %e,
            "HANDOFF execution failed"
        );
        RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Handoff from '{}' to '{}' failed: {}", source, target, e),
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
            AISOperationType::Handoff,
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

/// Target-owned configuration resolved from an inline SPAWN_AGENT record.
///
/// This is target lookup data, not parent state copied into the handoff child.
#[derive(Clone)]
struct InlineAgentConfig {
    system_prompt: Option<String>,
    backend: Option<String>,
    model: Option<String>,
}

/// Resolve the explicit configuration of an inline target agent. Returns
/// `None` when the target has no executable inline configuration.
async fn resolve_inline_agent_config(
    ctx: &ExecutionContext,
    target: &str,
) -> Option<InlineAgentConfig> {
    let key = format!("{}{}", belief_keys::AGENT_INFO_PREFIX, target);
    let Value::Object(agent_info) = read_stm_with_scope_fallback(ctx, &key).await? else {
        return None;
    };
    let config = InlineAgentConfig {
        system_prompt: agent_info
            .get(response_keys::SYSTEM_PROMPT)
            .and_then(|value| value.as_string())
            .cloned(),
        backend: agent_info
            .get(response_keys::BACKEND)
            .and_then(|value| value.as_string())
            .cloned(),
        model: agent_info
            .get(response_keys::MODEL)
            .and_then(|value| value.as_string())
            .cloned(),
    };

    (config.system_prompt.is_some() || config.backend.is_some() || config.model.is_some())
        .then_some(config)
}

/// Fallback dispatch when the target agent has no registered flow.
///
/// The request executes in the already-isolated HANDOFF child. Its only input
/// from the source is the explicit payload; the supplied configuration belongs
/// to the resolved target agent.
async fn handoff_inline_agent(
    ctx: &ExecutionContext,
    node: &Node,
    source: &str,
    target: &str,
    payload: &Value,
    config: InlineAgentConfig,
) -> Result<Value> {
    let InlineAgentConfig {
        system_prompt,
        backend,
        model,
    } = config;

    let prompt = payload.as_string().cloned().unwrap_or_default();

    let mut request = LLMRequest::new(prompt).with_operation_type(node.op_type);
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
        source = %source,
        target = %target,
        "HANDOFF dispatching to inline-spawned agent via LLM"
    );

    let response = execute_contextual_node_request(ctx, node, "HANDOFF", &request).await?;
    Ok(Value::String(response.content))
}
