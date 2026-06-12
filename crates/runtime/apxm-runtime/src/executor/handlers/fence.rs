//! FENCE operation - Memory fence/barrier

use super::{ExecutionContext, Node, Result, Value, get_input};

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // FENCE ensures ordering - pass through first input or Null
    let result = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        Value::Null
    };

    // Emit node output for session recording
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_node_output_with_name(node.id, node.metadata.name.as_deref(), &result);
    }

    Ok(result)
}

