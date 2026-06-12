//! Model Allowlist Validation Pass
//!
//! Validates that all models referenced in graph nodes are in the configured allowlist.
//! If no allowlist is configured, this governance check is disabled.
//!
//! Does NOT query external APIs or check model health — that's the runtime ModelRouter's job.

use crate::air_builder::AirModule;
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::span::Span;
use apxm_core::error::{Error, ErrorCode};
use std::collections::{HashMap, HashSet};

/// Validates that all models in the module are in the allowlist (if configured).
///
/// # Arguments
/// * `module` - The module to validate
/// * `allowlist` - Optional list of approved model names. If None, validation is skipped.
///
/// # Returns
/// * `Ok(())` - All models are in allowlist (or no allowlist configured)
/// * `Err(CompilerError)` - One or more models are not in allowlist
///
/// # Errors
/// Returns an error listing ALL violations, not just the first one.
pub fn validate_model_allowlist(module: &AirModule, allowlist: Option<&Vec<String>>) -> Result<()> {
    let Some(allowed_models) = allowlist else {
        return Ok(());
    };

    let allowed_set: HashSet<&str> = allowed_models.iter().map(String::as_str).collect();
    let mut violations: Vec<String> = Vec::new();
    let mut seen_models: HashMap<String, Vec<String>> = HashMap::new();

    // Check all nodes for model attributes
    for node in &module.nodes {
        if let Some(model_value) = node.attributes.get("model") {
            // Extract the string value from the Value enum
            let Some(model) = model_value.as_str() else {
                // Model attribute exists but is not a string - skip it
                continue;
            };

            // Skip empty or placeholder models
            if model.is_empty() || model == "null" {
                continue;
            }

            // Check if model is in allowlist
            if !allowed_set.contains(model) {
                seen_models
                    .entry(model.to_string())
                    .or_default()
                    .push(node.name.clone());
            }
        }
    }

    // Build violation messages (grouped by model for clarity)
    for (model, nodes) in seen_models {
        let node_list = if nodes.len() <= 3 {
            nodes.join(", ")
        } else {
            format!("{}, {} and {} more", nodes[0], nodes[1], nodes.len() - 2)
        };

        violations.push(format!(
            "Model '{}' not in allowlist (used by node{})",
            model,
            if nodes.len() == 1 {
                format!(": {}", nodes[0])
            } else {
                format!("s: {}", node_list)
            }
        ));
    }

    if violations.is_empty() {
        return Ok(());
    }

    // Build comprehensive error message
    let error_msg = format!(
        "Model allowlist validation failed:\n\n{}\n\nAllowed models: {}",
        violations.join("\n"),
        if allowed_models.is_empty() {
            "(empty allowlist)".to_string()
        } else if allowed_models.len() <= 5 {
            allowed_models.join(", ")
        } else {
            format!(
                "{}, {} and {} more",
                allowed_models[0],
                allowed_models[1],
                allowed_models.len() - 2
            )
        }
    );

    Err(CompilerError::Verification(Box::new(Error::new(
        ErrorCode::InvalidConfiguration,
        error_msg,
        Span::new("<graph>".to_string(), 1, 1, 0),
    ))))
}

