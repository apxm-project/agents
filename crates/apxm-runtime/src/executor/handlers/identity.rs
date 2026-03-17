//! IDENTITY operation - Identity passthrough with AAM transition recorded

use super::{ExecutionContext, Node, Result, Value, get_input};
use crate::aam::TransitionLabel;
use apxm_core::constants::runtime::{belief_keys, transition_labels};

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let value = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        Value::Null
    };

    // Record an identity transition in the AAM (state unchanged but recorded)
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::IDENTITY_NODE_PREFIX, node.id),
        value.clone(),
        TransitionLabel::Custom(transition_labels::IDENTITY.to_string()),
    );

    Ok(value)
}
