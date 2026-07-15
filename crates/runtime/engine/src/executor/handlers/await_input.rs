//! AWAIT_INPUT — generic durable external-input parking.
//!
//! This operation only returns a supplied seed value or parks until the host
//! wakes an opaque correlation key. It has no model, prompt, persona,
//! transcript, capability, Agent Skill, or conversational behavior.

use super::{ExecutionContext, Node, Result, Value, get_optional_string_attribute};
use apxm_core::{constants::graph::attrs as graph_attrs, error::RuntimeError};

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    if let Some(seed) = inputs.into_iter().next() {
        return Ok(seed);
    }

    let wait_key = match get_optional_string_attribute(node, graph_attrs::WAIT_KEY)? {
        Some(key) if !key.trim().is_empty() => key,
        _ => ctx
            .session_id
            .as_deref()
            .map(crate::scheduler::park_registry::session_input_key)
            .ok_or_else(|| RuntimeError::Operation {
                op_type: node.op_type,
                message: "AWAIT_INPUT requires wait_key outside a session".to_string(),
            })?,
    };

    tracing::info!(
        execution_id = %ctx.execution_id,
        node_id = node.id,
        %wait_key,
        "AWAIT_INPUT parking for host-delivered input"
    );
    Err(RuntimeError::OperationParked { wait_key })
}
