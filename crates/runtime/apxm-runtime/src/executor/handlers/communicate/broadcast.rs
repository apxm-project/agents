//! Broadcast COMMUNICATE: fan-out to all agents registered in FlowRegistry.

use super::super::{ExecutionContext, Node, Result, Value};
use crate::aam::ScopeSpec;
use crate::executor::ExecutorEngine;
use crate::flow_names::COMMUNICATE_FLOWS as COMMUNICATE_FLOW_NAMES;
use crate::metadata_keys as metadata;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::types::CommunicateProtocol;

/// Execute a broadcast sub-flow for a single agent.
///
/// Extracted into a standalone async fn so the compiler can prove `Send + 'static`
/// for the returned future — inline async blocks with captured `ExecutionContext`
/// trigger DashMap `Map` trait invariance errors.
async fn broadcast_one_agent(
    child_ctx: ExecutionContext,
    agent: String,
    dag: apxm_core::types::execution::ExecutionDag,
    msg: Value,
) -> (String, Result<Value>) {
    let _ = child_ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            child_ctx.scope_id(),
            belief_keys::COMMUNICATE_MESSAGE.to_string(),
            msg,
        )
        .await;

    let engine = ExecutorEngine::new(child_ctx);
    match engine.execute_dag(dag).await {
        Ok(result) => {
            let response = result
                .results
                .values()
                .find(|v| !matches!(v, Value::Null))
                .cloned()
                .unwrap_or(Value::Null);
            (agent, Ok(response))
        }
        Err(e) => (agent, Err(e)),
    }
}

/// Fan-out COMMUNICATE to ALL agents registered in the FlowRegistry in parallel.
///
/// Dispatches to every unique agent name that has a known "communicate" or "main"
/// flow. Each agent is called concurrently via `tokio::spawn`. Non-fatal errors
/// are captured as `Value::String("<agent>: <error>")` so the caller receives a
/// complete picture rather than a partial failure.
///
/// Returns `Value::Array` of all responses (one per agent, in arbitrary order).
pub(super) async fn execute_broadcast(
    ctx: &ExecutionContext,
    _node: &Node,
    message: Value,
) -> Result<Value> {
    let all_flows = ctx.flow_registry.list_flows();
    let agent_names: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        let mut agents = Vec::new();
        for (agent_name, flow_name) in &all_flows {
            if COMMUNICATE_FLOW_NAMES.contains(&flow_name.as_str())
                && seen.insert(agent_name.clone())
            {
                agents.push(agent_name.clone());
            }
        }
        agents
    };

    if agent_names.is_empty() {
        tracing::warn!(
            execution_id = %ctx.execution_id,
            "BROADCAST: no agents with communicate/main flows registered"
        );
        return Ok(Value::Array(vec![]));
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        agents = agent_names.len(),
        "COMMUNICATE BROADCAST fan-out starting"
    );

    let mut handles = Vec::with_capacity(agent_names.len());
    for agent_name in &agent_names {
        let sub_dag = {
            let mut found = None;
            for flow_name in COMMUNICATE_FLOW_NAMES {
                if let Some(dag) = ctx.flow_registry.get_flow(agent_name, flow_name) {
                    found = Some(dag);
                    break;
                }
            }
            match found {
                Some(dag) => dag,
                None => continue,
            }
        };

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
                agent_name.clone(),
            )
            .with_metadata(
                metadata::COMMUNICATE_MODE.to_string(),
                CommunicateProtocol::Broadcast.as_str().to_string(),
            );

        let msg = message.clone();
        let agent = agent_name.clone();
        let dag_clone = (*sub_dag).clone();

        handles.push(tokio::spawn(broadcast_one_agent(
            child_ctx, agent, dag_clone, msg,
        )));
    }

    let mut responses = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok((agent, Ok(val))) => {
                tracing::debug!(agent = %agent, "BROADCAST response received");
                responses.push(val);
            }
            Ok((agent, Err(e))) => {
                tracing::warn!(agent = %agent, error = %e, "BROADCAST agent returned error");
                responses.push(Value::String(format!("{}: {}", agent, e)));
            }
            Err(join_err) => {
                tracing::error!(error = %join_err, "BROADCAST task panicked");
                responses.push(Value::String(format!("panic: {}", join_err)));
            }
        }
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        responses = responses.len(),
        "COMMUNICATE BROADCAST completed"
    );

    Ok(Value::Array(responses))
}

