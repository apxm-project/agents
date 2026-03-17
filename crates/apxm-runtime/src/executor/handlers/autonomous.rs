//! AUTONOMOUS operation - Autonomous execution stub
//!
//! Placeholder for future autonomous agent execution mode.
//! Currently passes through the first input and records an AAM transition.

use super::{ExecutionContext, Node, Result, Value, get_input};
use crate::aam::TransitionLabel;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let value = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        Value::Null
    };

    // Record autonomous transition in AAM
    ctx.aam.set_belief(
        format!("_autonomous_node:{}", node.id),
        value.clone(),
        TransitionLabel::operation(node.op_type),
    );

    tracing::info!(
        execution_id = %ctx.execution_id,
        node_id = node.id,
        "AUTONOMOUS operation executed (stub)"
    );

    Ok(value)
}
