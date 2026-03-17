//! SPAWN_AGENT operation - Create a new agent instance at runtime
//!
//! Registers a new agent in the flow registry. The agent can then receive
//! COMMUNICATE or DELEGATE messages. Returns the agent's identifier.
//!
//! ## Attributes
//! - `agent_name`    (required): name for the new agent
//! - `capabilities`  (optional): list of capabilities
//! - `goals`         (optional): initial goals

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use crate::aam::TransitionLabel;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let agent_name = get_string_attribute(node, graph_attrs::AGENT_NAME)?;

    tracing::info!(
        execution_id = %ctx.execution_id,
        agent_name = %agent_name,
        "Executing SPAWN_AGENT operation"
    );

    // Check if agent already exists
    let existing_flows = ctx.flow_registry.flows_for_agent(&agent_name);
    if !existing_flows.is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Agent '{}' already exists in the flow registry", agent_name),
        });
    }

    // Record agent spawn in AAM
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::SPAWNED_AGENT_PREFIX, agent_name),
        Value::String(agent_name.clone()),
        TransitionLabel::Custom(format!("spawn_agent:{}", agent_name)),
    );

    // Store the new agent's metadata in STM for later reference
    let mut agent_info = HashMap::new();
    agent_info.insert(response_keys::NAME.to_string(), Value::String(agent_name.clone()));
    agent_info.insert(
        response_keys::SPAWNED_BY.to_string(),
        Value::String(ctx.execution_id.clone()),
    );

    if let Some(capabilities) = node.attributes.get(response_keys::CAPABILITIES) {
        agent_info.insert(response_keys::CAPABILITIES.to_string(), capabilities.clone());
    }
    if let Some(goals) = node.attributes.get(response_keys::GOALS) {
        agent_info.insert(response_keys::GOALS.to_string(), goals.clone());
    }

    let _ = ctx
        .memory
        .write(
            crate::memory::MemorySpace::Stm,
            format!("{}{}", belief_keys::AGENT_INFO_PREFIX, agent_name),
            Value::Object(agent_info.clone()),
        )
        .await;

    tracing::info!(
        execution_id = %ctx.execution_id,
        agent_name = %agent_name,
        "SPAWN_AGENT completed successfully"
    );

    Ok(Value::Object(agent_info))
}
