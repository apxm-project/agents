//! CONST_STR operation - String constant

use super::{
    ExecutionContext, Node, Result, Value, get_string_attribute,
    template::{input_names_from_node, render_named},
};
use apxm_core::constants::graph::attrs as graph_attrs;

pub async fn execute(_ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let value = get_string_attribute(node, graph_attrs::VALUE)?;
    let input_names = input_names_from_node(node);
    let rendered = render_named(&value, &inputs, &input_names)?;
    Ok(Value::String(rendered))
}
