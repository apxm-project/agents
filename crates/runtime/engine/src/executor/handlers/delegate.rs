//! DELEGATE operation - Delegate a task to a sub-agent
//!
//! Creates a sub-task and assigns it to a target agent. The target agent
//! executes its flow and returns the result. Returns an object with
//! task_handle and result fields.
//!
//! ## Attributes
//! - `task_spec`     (required): description of the task to delegate
//! - `target_agent`  (required): name of the agent to delegate to; accepts
//!   the platform target grammar (`docs/plans/platform.md` §5) —
//!   `topic:<subject>` and `capability:<cap-id>` are resolved to a concrete
//!   registered agent via `target_resolution` (RTG-8) before the exact-id
//!   lookup below runs; anything else is treated as an exact id, unchanged.

use super::{
    ExecutionContext, Node, Result, Value, execute_llm_request_for_node, get_string_attribute,
    read_stm_with_scope_fallback, target_resolution::resolve_target_or_passthrough,
};
use crate::aam::{ScopeSpec, TransitionLabel};
use crate::executor::ExecutorEngine;
use crate::flow_names::DELEGATE_FLOWS as DELEGATE_FLOW_NAMES;
use crate::metadata_keys as metadata;
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let task_spec = get_string_attribute(node, graph_attrs::TASK_SPEC)?;
    let raw_target = get_string_attribute(node, graph_attrs::TARGET_AGENT)?;
    // RTG-8: `topic:`/`capability:` targets resolve to a concrete member
    // through the routing pipeline before the exact lookup below; anything
    // else (including bare exact ids) passes through unchanged.
    let target_agent = resolve_target_or_passthrough(&ctx.flow_registry, &raw_target)?;

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
        found
    };

    // Inline fallback (mirrors HANDOFF): when the target has no sibling
    // `delegate`/`main` flow in the artifact, look up `agent_info:<target>` in
    // STM (written by SPAWN_AGENT) and dispatch the task as a one-shot LLM ASK
    // against that agent's config. This makes `delegate(researcher)` work for an
    // inline-spawned sub-agent even before a multi-func flow is emitted.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::types::{
        AISOperationType, Agent, AgentFlow, AgentMetadata, CapabilityDeclaration, DagMetadata,
        ExecutionDag,
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
    /// `delegate`/`main` flow without exercising the scheduler beyond a NOP.
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
        let execution_dag = trivial_dag(crate::flow_names::DELEGATE);
        let flow = AgentFlow {
            name: crate::flow_names::DELEGATE.to_string(),
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

    fn node_with(target_agent: &str) -> Node {
        let mut node = Node::new(2, AISOperationType::Delegate);
        node.set_attribute(
            graph_attrs::TASK_SPEC.to_string(),
            Value::String("do the thing".to_string()),
        );
        node.set_attribute(
            graph_attrs::TARGET_AGENT.to_string(),
            Value::String(target_agent.to_string()),
        );
        node
    }

    /// Regression (RTG-8): an exact-id target with no registered flow and no
    /// inline STM agent info fails with exactly the same "not found" message
    /// as before target-grammar resolution was added — the exact lookup
    /// itself is unchanged.
    #[tokio::test]
    async fn exact_id_target_unchanged_not_found_error() {
        let ctx = test_context().await;
        let node = node_with("researcher");

        let err = execute(&ctx, &node, vec![])
            .await
            .expect_err("no flow, no STM info");
        match err {
            RuntimeError::Operation { message, .. } => {
                assert!(
                    message.contains("Agent 'researcher' not found"),
                    "unexpected message: {message}"
                );
                assert!(message.contains("no 'delegate'/'main' flow"));
            }
            other => panic!("expected Operation error, got {other:?}"),
        }
    }

    /// RTG-8: `topic:<subject>` resolves to a concrete registered member via
    /// the routing pipeline, and DELEGATE proceeds exactly as if that member
    /// had been named directly.
    #[tokio::test]
    async fn topic_target_resolves_and_executes() {
        let ctx = test_context().await;
        ctx.flow_registry
            .register_agent(agent_with("billing", &[], &["receivables"]));
        let node = node_with("topic:receivables");

        let result = execute(&ctx, &node, vec![])
            .await
            .expect("resolves and executes");
        let Value::Object(obj) = result else {
            panic!("expected object result");
        };
        let task_handle = obj
            .get(apxm_core::constants::runtime::response_keys::TASK_HANDLE)
            .and_then(|v| v.as_string())
            .expect("task_handle present");
        assert!(task_handle.starts_with("delegate_billing_"));
    }

    /// RTG-8: `capability:<cap-id>` resolves to the declaring member.
    #[tokio::test]
    async fn capability_target_resolves_and_executes() {
        let ctx = test_context().await;
        ctx.flow_registry
            .register_agent(agent_with("writer", &["draft_report"], &[]));
        let node = node_with("capability:draft_report");

        let result = execute(&ctx, &node, vec![])
            .await
            .expect("resolves and executes");
        let Value::Object(obj) = result else {
            panic!("expected object result");
        };
        let task_handle = obj
            .get(apxm_core::constants::runtime::response_keys::TASK_HANDLE)
            .and_then(|v| v.as_string())
            .expect("task_handle present");
        assert!(task_handle.starts_with("delegate_writer_"));
    }

    /// RTG-8: zero candidates for a routed target never falls back to a
    /// broadcast — it's a typed no-route error (platform.md rule 5).
    #[tokio::test]
    async fn topic_target_with_no_candidate_returns_no_route_error() {
        let ctx = test_context().await;
        let node = node_with("topic:receivables");

        let err = execute(&ctx, &node, vec![])
            .await
            .expect_err("no agent declares this topic");
        assert!(matches!(err, RuntimeError::NoRouteFound { .. }));
    }
}
