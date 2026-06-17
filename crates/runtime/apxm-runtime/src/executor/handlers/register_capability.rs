//! REGISTER_CAPABILITY operation - Register a new capability in the runtime registry
//!
//! Dynamically registers a stub capability in the runtime's capability registry.
//! The capability becomes available for INV operations after registration.
//!
//! ## Attributes
//! - `capability_name` (required): name for the capability to register
//! - `description`     (optional): human-readable description
//! - `parameters_schema` (optional): JSON schema for parameters

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use crate::aam::TransitionLabel;
use apxm_core::constants::defaults;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let capability_name = get_string_attribute(node, graph_attrs::CAPABILITY_NAME)?;
    let description = node
        .attributes
        .get(graph_attrs::DESCRIPTION)
        .and_then(|v| v.as_string())
        .cloned()
        .unwrap_or_else(|| defaults::DEFAULT_DESCRIPTION.to_string());

    tracing::info!(
        execution_id = %ctx.execution_id,
        capability_name = %capability_name,
        description = %description,
        "Executing REGISTER_CAPABILITY operation"
    );

    // Record capability registration in AAM
    ctx.aam.set_belief(
        format!(
            "{}{}",
            belief_keys::REGISTERED_CAPABILITY_PREFIX,
            capability_name
        ),
        Value::String(description.clone()),
        TransitionLabel::Custom(format!("register_capability:{}", capability_name)),
    );

    // Store the registration metadata in STM
    let _ = ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            ctx.scope_id(),
            format!(
                "{}{}",
                belief_keys::CAPABILITY_REGISTERED_PREFIX,
                capability_name
            ),
            Value::String(description.clone()),
        )
        .await;

    // Build result
    let mut result = HashMap::new();
    result.insert(
        response_keys::CAPABILITY_NAME.to_string(),
        Value::String(capability_name.clone()),
    );
    result.insert(
        response_keys::DESCRIPTION.to_string(),
        Value::String(description),
    );
    result.insert(response_keys::REGISTERED.to_string(), Value::Bool(true));

    tracing::info!(
        execution_id = %ctx.execution_id,
        capability_name = %capability_name,
        "REGISTER_CAPABILITY completed successfully"
    );

    Ok(Value::Object(result))
}
