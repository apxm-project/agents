//! NOP operation - No-op passthrough (no AAM transition)

use super::{ExecutionContext, Node, Result, Value, get_input};

pub async fn execute(_ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Pure passthrough: return first input or Null
    if !inputs.is_empty() {
        Ok(get_input(node, &inputs, 0)?)
    } else {
        Ok(Value::Null)
    }
}

