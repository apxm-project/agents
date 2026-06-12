//! BRANCH operation - Conditional branching

use super::{ExecutionContext, Node, Result, Value, get_input};
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // First input is the condition
    let condition = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        Value::Bool(false)
    };

    // Evaluate condition as boolean
    let is_true = condition.as_boolean().unwrap_or(false);

    // Record branch decision in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::BRANCH_PREFIX,
            ctx.execution_id,
            node.id
        ),
        Value::Bool(is_true),
        label,
    );

    // Return boolean result (actual branching handled by scheduler)
    Ok(Value::Bool(is_true))
}

