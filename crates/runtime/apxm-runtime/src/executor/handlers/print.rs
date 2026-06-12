//! PRINT operation - Print output to stdout with markdown rendering (void operation)

use super::{
    ExecutionContext, Node, Result, Value, get_string_attribute,
    template::{input_names_from_node, render_named},
};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use termimad::MadSkin;

/// Execute a print operation. This is a void operation (no output tokens).
/// Output is rendered as markdown for terminal display.
///
/// `message` is a named-placeholder template; `{name}` references the
/// corresponding input via the node's `input_names` parallel array.
pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let message = get_string_attribute(node, graph_attrs::MESSAGE).unwrap_or_default();
    let input_names = input_names_from_node(node);
    let output = render_named(&message, &inputs, &input_names)?;

    // Render markdown to terminal
    let skin = MadSkin::default();
    skin.print_text(&output);

    // Record print in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::PRINT_PREFIX,
            ctx.execution_id,
            node.id
        ),
        Value::String(output.chars().take(200).collect::<String>()),
        label,
    );

    // Void operation - return Null (no output tokens in artifact)
    Ok(Value::Null)
}

