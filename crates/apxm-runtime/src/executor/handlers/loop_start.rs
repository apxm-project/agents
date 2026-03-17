//! LOOP_START operation - Loop initialization

use super::{ExecutionContext, Node, Result, Value};
use apxm_core::constants::graph::attrs as graph_attrs;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    // Initialize loop counter
    let max_iterations = node
        .attributes
        .get(graph_attrs::MAX_ITERATIONS)
        .and_then(|v| v.as_u64())
        .unwrap_or(100);

    // Record loop initialization in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!("_loop_start:{}:{}", ctx.execution_id, node.id),
        Value::Number(apxm_core::types::values::Number::Integer(max_iterations as i64)),
        label,
    );

    Ok(Value::Number(apxm_core::types::values::Number::Integer(
        max_iterations as i64,
    )))
}
