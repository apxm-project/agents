//! LOOP_END operation - Loop termination check

use super::{ExecutionContext, Node, Result, Value, get_input};
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Check if we should continue looping
    let counter = get_input(node, &inputs, 0)?;

    let should_continue = counter.as_u64().is_some_and(|count| count > 0);

    // Record loop check in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!("{}{}:{}", belief_keys::LOOP_END_PREFIX, ctx.execution_id, node.id),
        Value::Bool(should_continue),
        label,
    );

    Ok(Value::Bool(should_continue))
}
