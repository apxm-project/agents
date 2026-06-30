//! ERR operation - Create error value

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let message = get_string_attribute(node, graph_attrs::RECOVERY_TEMPLATE)?;

    // Record error creation in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::ERR_PREFIX,
            ctx.execution_id,
            node.id
        ),
        Value::String(message.clone()),
        label,
    );

    // Return error message as string value
    // Actual error propagation handled by scheduler
    Ok(Value::String(format!("Error: {}", message)))
}
