//! QMEM operation - Query memory

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute, get_string_attribute,
};
use crate::{
    aam::{STAGED_BELIEF_PREFIX, TransitionLabel},
    memory::MemorySpace,
};
use apxm_core::constants::graph::attrs as graph_attrs;

/// Execute QMEM operation - Query memory from specified tier
pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let query = get_string_attribute(node, graph_attrs::QUERY)?;
    let memory_tier = get_optional_string_attribute(node, graph_attrs::MEMORY_TIER)?;
    let limit = node
        .attributes
        .get(graph_attrs::LIMIT)
        .and_then(|v| v.as_u64())
        .unwrap_or(apxm_core::constants::defaults::DEFAULT_MEMORY_LIMIT) as usize;

    // Determine memory space
    let space = match memory_tier.as_deref() {
        Some(tier) => tier.parse::<MemorySpace>()?,
        None => MemorySpace::Stm, // Default to STM
    };

    // Search memory. Scope by session (when present) so a later turn's QMEM
    // reads the memory an earlier turn's UMEM wrote; falls back to scope_id.
    let results = ctx
        .memory
        .search_scoped(space, ctx.memory_scope(), &query, limit)
        .await?;

    // Emit memory-read event
    if let Some(emitter) = &ctx.event_emitter {
        let scope = memory_tier.as_deref().unwrap_or("stm");
        emitter.emit_memory_read(scope, &query);
    }

    // Convert search results to Value::Array
    let values: Vec<Value> = results
        .into_iter()
        .map(|r| {
            // Create object with key and value
            let mut obj = std::collections::HashMap::new();
            obj.insert("key".to_string(), Value::String(r.key));
            obj.insert("value".to_string(), r.value);
            obj.insert(
                "score".to_string(),
                Value::Number(apxm_core::types::values::Number::Float(r.score)),
            );
            Value::Object(obj)
        })
        .collect();

    let array_value = Value::Array(values.clone());

    // Stage results as a belief so downstream ops can refer to them.
    let staging_id = node
        .attributes
        .get(graph_attrs::STAGING_ID)
        .and_then(|v| v.as_string().map(|s| s.to_string()))
        .unwrap_or_else(|| format!("{}:{}", ctx.execution_id, node.id));
    let stage_key = format!("{}{}", STAGED_BELIEF_PREFIX, staging_id);
    ctx.aam.set_belief(
        stage_key,
        array_value.clone(),
        TransitionLabel::operation(node.id, node.op_type),
    );

    Ok(array_value)
}

