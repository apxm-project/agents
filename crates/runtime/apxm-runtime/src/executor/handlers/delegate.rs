//! DELEGATE operation - Delegate a task to a sub-agent
//!
//! Creates a sub-task and assigns it to a target agent. The target agent
//! executes its flow and returns the result. Returns an object with
//! task_handle and result fields.
//!
//! ## Attributes
//! - `task_spec`     (required): description of the task to delegate
//! - `target_agent`  (required): name of the agent to delegate to

use super::{
    ExecutionContext, Node, Result, Value, execute_llm_request_for_node, get_string_attribute,
    read_stm_with_scope_fallback,
};
use apxm_backends::LLMRequest;
use crate::aam::{ScopeSpec, TransitionLabel};
use crate::executor::ExecutorEngine;
use crate::flow_names::DELEGATE_FLOWS as DELEGATE_FLOW_NAMES;
use crate::metadata_keys as metadata;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let task_spec = get_string_attribute(node, graph_attrs::TASK_SPEC)?;
    let target_agent = get_string_attribute(node, graph_attrs::TARGET_AGENT)?;

    tracing::info!(
        execution_id = %ctx.execution_id,
        target_agent = %target_agent,
        task_spec = %task_spec,
        "Executing DELEGATE operation"
    );

    // Record delegation in AAM
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::DELEGATE_PREFIX, target_agent),
        Value::String(task_spec.clone()),
        TransitionLabel::Custom(format!("delegate:{}", target_agent)),
    );

    // Look up the target agent's flow in the FlowRegistry
    let sub_dag = {
        let mut found = None;
        for flow_name in DELEGATE_FLOW_NAMES {
            if let Some(dag) = ctx.flow_registry.get_flow(&target_agent, flow_name) {
                found = Some(dag);
                break;
            }
        }
        match found {
            Some(dag) => Some(dag),
            None => None,
        }
    };

    // Inline fallback (mirrors HANDOFF): when the target has no sibling
    // `delegate`/`main` flow in the artifact, look up `agent_info:<target>` in
    // STM (written by SPAWN_AGENT) and dispatch the task as a one-shot LLM ASK
    // against that agent's config. This makes `delegate(researcher)` work for an
    // inline-spawned sub-agent even before a multi-func flow is emitted (FR; US5).
    let Some(sub_dag) = sub_dag else {
        if let Some(response) =
            delegate_inline_agent(ctx, node, &target_agent, &task_spec, inputs.first()).await?
        {
            ctx.aam.set_belief(
                format!("{}{}", belief_keys::DELEGATE_PREFIX, target_agent),
                Value::Null,
                TransitionLabel::Custom(format!("delegate_completed:{}", target_agent)),
            );
            let task_handle = format!("delegate_{}_{}", target_agent, ctx.execution_id);
            let mut result_obj = HashMap::new();
            result_obj.insert(
                response_keys::TASK_HANDLE.to_string(),
                Value::String(task_handle),
            );
            result_obj.insert(response_keys::RESULT.to_string(), response);
            let result = Value::Object(result_obj);
            if let Some(emitter) = &ctx.event_emitter {
                emitter.emit_node_output_with_name(node.id, node.metadata.name.as_deref(), &result);
            }
            return Ok(result);
        }
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "Agent '{}' not found: no 'delegate'/'main' flow and no inline \
                 SPAWN_AGENT agent_info in STM",
                target_agent
            ),
        });
    };

    // Create a child context for sub-flow execution
    let child_ctx = ctx
        .child_with_scope(ScopeSpec::snapshot_all())
        .with_metadata(
            metadata::PARENT_EXECUTION_ID.to_string(),
            ctx.execution_id.clone(),
        )
        .with_metadata(metadata::DELEGATE_TASK_SPEC.to_string(), task_spec.clone())
        .with_metadata(metadata::DELEGATE_TARGET.to_string(), target_agent.clone());

    // Inject the task spec and any input into STM
    let _ = child_ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            child_ctx.scope_id(),
            belief_keys::DELEGATE_TASK_SPEC.to_string(),
            Value::String(task_spec.clone()),
        )
        .await;

    if let Some(input) = inputs.first() {
        let _ = child_ctx
            .memory
            .write_scoped(
                crate::memory::MemorySpace::Stm,
                child_ctx.scope_id(),
                belief_keys::DELEGATE_INPUT.to_string(),
                input.clone(),
            )
            .await;
    }

    // Execute the sub-flow DAG
    let engine = ExecutorEngine::new(child_ctx);
    let dag_to_execute = (*sub_dag).clone();

    let result = engine.execute_dag(dag_to_execute).await.map_err(|e| {
        tracing::error!(
            target_agent = %target_agent,
            error = %e,
            "DELEGATE sub-flow execution failed"
        );
        RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Delegation to agent '{}' failed: {}", target_agent, e),
        }
    })?;

    // Extract the response from the sub-flow's exit nodes
    let response = result
        .results
        .values()
        .find(|v| !matches!(v, Value::Null))
        .cloned()
        .unwrap_or(Value::Null);

    // Clear pending delegation belief
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::DELEGATE_PREFIX, target_agent),
        Value::Null,
        TransitionLabel::Custom(format!("delegate_completed:{}", target_agent)),
    );

    // Return result with task handle metadata
    let task_handle = format!("delegate_{}_{}", target_agent, ctx.execution_id);
    let mut result_obj = HashMap::new();
    result_obj.insert(
        response_keys::TASK_HANDLE.to_string(),
        Value::String(task_handle),
    );
    result_obj.insert(response_keys::RESULT.to_string(), response);

    tracing::info!(
        execution_id = %ctx.execution_id,
        target_agent = %target_agent,
        "DELEGATE completed successfully"
    );

    let result = Value::Object(result_obj);

    // Emit node output for session recording
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_node_output_with_name(node.id, node.metadata.name.as_deref(), &result);
    }

    Ok(result)
}

/// Inline fallback dispatch when the target sub-agent has no registered flow.
///
/// Mirrors `handoff_inline_agent`: reads `agent_info:<target>` from STM (written
/// by SPAWN_AGENT) and, if it carries a `system_prompt`/`backend`/`model`,
/// dispatches the task spec as a one-shot LLM ASK against that agent's config.
/// Returns `Ok(None)` when no usable inline agent info is present so the caller
/// can surface the original "not found" error.
async fn delegate_inline_agent(
    ctx: &ExecutionContext,
    node: &Node,
    target: &str,
    task_spec: &str,
    input: Option<&Value>,
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
    let backend = agent_info
        .get(response_keys::BACKEND)
        .and_then(|v| v.as_string())
        .cloned();
    let model = agent_info
        .get(response_keys::MODEL)
        .and_then(|v| v.as_string())
        .cloned();

    // Metadata-only entries (e.g. ACP subprocess agents) shouldn't be auto-run.
    if system_prompt.is_none() && backend.is_none() && model.is_none() {
        return Ok(None);
    }

    let prompt = match input.and_then(|v| v.as_string()) {
        Some(extra) if !extra.is_empty() => format!("{task_spec}\n\n{extra}"),
        _ => task_spec.to_string(),
    };

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
        target = %target,
        "DELEGATE dispatching to inline-spawned agent via LLM"
    );

    let response = execute_llm_request_for_node(ctx, node, "DELEGATE", &request).await?;
    Ok(Some(Value::String(response.content)))
}

