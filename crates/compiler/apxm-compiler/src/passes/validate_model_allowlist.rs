//! Model Allowlist Validation Pass
//!
//! Validates that all models referenced in graph nodes are in the configured allowlist.
//! If no allowlist is configured, validation passes silently (backward compatible).
//!
//! This is a compile-time governance check (Plan A from model-profiles-design.md).
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
    // If no allowlist is configured, skip validation (backward compatible)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_builder::{AirModule, AirNode};
    use apxm_core::types::{AISOperationType, Value};
    use std::collections::HashMap;

    fn make_test_graph(models: Vec<&str>) -> AirModule {
        let mut nodes = Vec::new();
        for (i, model) in models.iter().enumerate() {
            let mut attributes = HashMap::new();
            attributes.insert("model".to_string(), Value::String(model.to_string()));

            nodes.push(AirNode {
                id: (i + 1) as u64,
                name: format!("node_{}", i + 1),
                op: AISOperationType::Ask,
                attributes,
            });
        }

        AirModule {
            name: "test_graph".to_string(),
            nodes,
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_no_allowlist_passes_silently() {
        let graph = make_test_graph(vec!["claude-opus-4-6", "gpt-4o", "random-model"]);
        let result = validate_model_allowlist(&graph, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_graph_passes() {
        let graph = AirModule {
            name: "empty".to_string(),
            nodes: vec![],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };
        let allowlist = vec!["claude-opus-4-6".to_string()];
        let result = validate_model_allowlist(&graph, Some(&allowlist));
        assert!(result.is_ok());
    }

    #[test]
    fn test_model_in_allowlist_passes() {
        let graph = make_test_graph(vec!["claude-opus-4-6"]);
        let allowlist = vec!["claude-opus-4-6".to_string(), "gpt-4o".to_string()];
        let result = validate_model_allowlist(&graph, Some(&allowlist));
        assert!(result.is_ok());
    }

    #[test]
    fn test_all_models_in_allowlist_passes() {
        let graph = make_test_graph(vec!["claude-opus-4-6", "gpt-4o", "claude-opus-4-6"]);
        let allowlist = vec![
            "claude-opus-4-6".to_string(),
            "gpt-4o".to_string(),
            "gemini-2.0-flash".to_string(),
        ];
        let result = validate_model_allowlist(&graph, Some(&allowlist));
        assert!(result.is_ok());
    }

    #[test]
    fn test_model_not_in_allowlist_fails() {
        let graph = make_test_graph(vec!["claude-opus-5"]);
        let allowlist = vec!["claude-opus-4-6".to_string(), "gpt-4o".to_string()];
        let result = validate_model_allowlist(&graph, Some(&allowlist));
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("claude-opus-5"));
        assert!(err_msg.contains("not in allowlist"));
    }

    #[test]
    fn test_multiple_violations_all_reported() {
        let graph = make_test_graph(vec!["claude-opus-5", "gpt-5", "claude-opus-5"]);
        let allowlist = vec!["claude-opus-4-6".to_string(), "gpt-4o".to_string()];
        let result = validate_model_allowlist(&graph, Some(&allowlist));
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        // Both violations should be reported
        assert!(err_msg.contains("claude-opus-5"));
        assert!(err_msg.contains("gpt-5"));
        // Allowed models should be shown
        assert!(err_msg.contains("claude-opus-4-6"));
        assert!(err_msg.contains("gpt-4o"));
    }

    #[test]
    fn test_empty_allowlist_rejects_all_models() {
        let graph = make_test_graph(vec!["claude-opus-4-6"]);
        let allowlist: Vec<String> = vec![];
        let result = validate_model_allowlist(&graph, Some(&allowlist));
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("empty allowlist"));
    }

    #[test]
    fn test_nodes_without_model_attribute_ignored() {
        // Create graph with nodes that don't have model attributes
        let mut nodes = vec![];
        let mut attrs = HashMap::new();
        attrs.insert("prompt".to_string(), Value::String("test".to_string()));

        nodes.push(AirNode {
            id: 1,
            name: "no_model".to_string(),
            op: AISOperationType::Ask,
            attributes: attrs,
        });

        let graph = AirModule {
            name: "test".to_string(),
            nodes,
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let allowlist = vec!["claude-opus-4-6".to_string()];
        let result = validate_model_allowlist(&graph, Some(&allowlist));
        // Should pass because node has no model attribute
        assert!(result.is_ok());
    }
}
