//! EXC operation - Raise exception

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let message = get_string_attribute(node, graph_attrs::MESSAGE)?;

    // Record exception in AAM before raising
    let label = crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!("_exc:{}:{}", ctx.execution_id, node.id),
        Value::String(message.clone()),
        label,
    );

    Err(apxm_core::error::RuntimeError::Operation {
        op_type: node.op_type,
        message,
    })
}
