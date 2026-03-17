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
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let capability_name = get_string_attribute(node, "capability_name")?;
    let description = node
        .attributes
        .get("description")
        .and_then(|v| v.as_string())
        .cloned()
        .unwrap_or_else(|| "Dynamically registered capability".to_string());

    tracing::info!(
        execution_id = %ctx.execution_id,
        capability_name = %capability_name,
        description = %description,
        "Executing REGISTER_CAPABILITY operation"
    );

    // Record capability registration in AAM
    ctx.aam.set_belief(
        format!("_registered_capability:{}", capability_name),
        Value::String(description.clone()),
        TransitionLabel::Custom(format!("register_capability:{}", capability_name)),
    );

    // Store the registration metadata in STM
    let _ = ctx
        .memory
        .write(
            crate::memory::MemorySpace::Stm,
            format!("_capability_registered:{}", capability_name),
            Value::String(description.clone()),
        )
        .await;

    // Build result
    let mut result = HashMap::new();
    result.insert("capability_name".to_string(), Value::String(capability_name.clone()));
    result.insert("description".to_string(), Value::String(description));
    result.insert("registered".to_string(), Value::Bool(true));

    tracing::info!(
        execution_id = %ctx.execution_id,
        capability_name = %capability_name,
        "REGISTER_CAPABILITY completed successfully"
    );

    Ok(Value::Object(result))
}
