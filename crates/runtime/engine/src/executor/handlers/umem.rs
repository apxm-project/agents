//! UMEM operation - Update memory

use super::{
    ExecutionContext, Node, Result, Value, get_input, get_optional_string_attribute,
    get_string_attribute,
};
use crate::{
    aam::TransitionLabel,
    memory::{MemorySpace, parse_memory_space},
};
use apxm_core::constants::graph::attrs as graph_attrs;

/// Execute UMEM operation - Store value in specified memory tier
pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let key = get_string_attribute(node, graph_attrs::KEY)?;
    let memory_tier = get_optional_string_attribute(node, graph_attrs::MEMORY_TIER)?;

    // Get value from first input or attribute
    let value = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        node.attributes
            .get(graph_attrs::VALUE)
            .cloned()
            .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
                op_type: node.op_type,
                message: "UMEM requires either input or value attribute".to_string(),
            })?
    };

    // Determine memory space
    let space = match memory_tier.as_deref() {
        Some(tier) => parse_memory_space(tier)?,
        None => MemorySpace::Stm, // Default to STM
    };

    // Store in memory - episodic uses record (append-only), others use write
    match space {
        MemorySpace::Episodic => {
            ctx.memory
                .record_episode(
                    key.clone(),
                    value.clone(),
                    ctx.execution_id.clone(),
                    Some(node.id),
                    None,
                )
                .await?;
        }
        _ => {
            // Scope by session (when present) so the write survives across
            // turns sharing the session; falls back to scope_id. The Episodic
            // branch above stays keyed by execution_id.
            ctx.memory
                .write_scoped(space, ctx.memory_scope(), key.clone(), value.clone())
                .await?;
        }
    }

    // Emit memory-write event
    if let Some(emitter) = &ctx.event_emitter {
        let scope = memory_tier.as_deref().unwrap_or("stm");
        emitter.emit_memory_write(scope, &key);
    }

    ctx.aam.set_belief(
        key,
        value.clone(),
        TransitionLabel::operation(node.id, node.op_type),
    );

    // Return the stored value
    Ok(value)
}
