//! Model Profile Validation Pass (Plan B from model-profiles-design.md)
//!
//! Validates that:
//! 1. All `model_profile` attributes reference valid profiles in the registry
//! 2. All candidates in those profiles exist in the ModelRegistry
//! 3. All candidates pass the allowlist check (if configured)
//!
//! This is a compile-time semantic check. It does NOT query external APIs or check
//! model health — that's the runtime ProfileRouter's job.

use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::{Error, ErrorCode};
use apxm_core::error::span::Span;
use apxm_graph::ApxmGraph;
use std::collections::{HashMap, HashSet};

/// Validates that all model_profile attributes in the graph reference valid profiles.
///
/// # Arguments
/// * `graph` - The graph to validate
/// * `profiles` - Map of profile name to profile definition
/// * `models` - Set of valid model names (from ModelRegistry)
/// * `allowlist` - Optional list of approved model names (from config)
///
/// # Returns
/// * `Ok(())` - All model profiles are valid
/// * `Err(CompilerError)` - One or more validation failures
///
/// # Errors
/// Returns an error if:
/// - A profile referenced in a node does not exist
/// - A profile has no candidates configured (warning level)
/// - A candidate model is not in the models set
/// - A candidate model is not in the allowlist (when allowlist is configured)
pub fn validate_model_profile(
    graph: &ApxmGraph,
    profiles: &HashMap<String, apxm_core::model_profiles::ModelProfile>,
    models: &HashSet<String>,
    allowlist: Option<&Vec<String>>,
) -> Result<()> {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut seen_profiles: HashMap<String, Vec<String>> = HashMap::new();

    // Build allowlist set if provided
    let allowed_set: Option<HashSet<&str>> =
        allowlist.map(|list| list.iter().map(String::as_str).collect());

    // Check all nodes for model_profile attributes
    for node in &graph.nodes {
        if let Some(profile_value) = node.attributes.get("model_profile") {
            let Some(profile_name) = profile_value.as_str() else {
                // model_profile attribute exists but is not a string - skip it
                continue;
            };

            // Skip empty or placeholder profiles
            if profile_name.is_empty() || profile_name == "null" {
                continue;
            }

            // Track which nodes use this profile
            seen_profiles
                .entry(profile_name.to_string())
                .or_default()
                .push(node.name.clone());

            // Check if profile exists
            let Some(profile) = profiles.get(profile_name) else {
                errors.push(format!(
                    "Profile '{}' not found in registry (used by node: {})",
                    profile_name, node.name
                ));
                continue;
            };

            // Warn if profile has no candidates
            if !profile.has_candidates() {
                warnings.push(format!(
                    "Profile '{}' has no candidates configured (used by node: {})",
                    profile_name, node.name
                ));
                continue;
            }

            // Validate each candidate
            for candidate in &profile.candidates {
                // Check if model exists in models set
                if !models.contains(&candidate.model) {
                    errors.push(format!(
                        "Model '{}' in profile '{}' not found in model registry",
                        candidate.model, profile_name
                    ));
                }

                // Check if model is in allowlist (if configured)
                if let Some(ref allowed) = allowed_set {
                    if !allowed.contains(candidate.model.as_str()) {
                        errors.push(format!(
                            "Model '{}' in profile '{}' not in allowlist (priority: {})",
                            candidate.model, profile_name, candidate.priority
                        ));
                    }
                }
            }
        }
    }

    // Note: Warnings would be logged by the caller

    if errors.is_empty() {
        return Ok(());
    }

    // Build comprehensive error message
    let error_msg = format!(
        "Model profile validation failed:\n\n{}{}{}",
        errors.join("\n"),
        if !warnings.is_empty() {
            format!("\n\nWarnings:\n{}", warnings.join("\n"))
        } else {
            String::new()
        },
        if let Some(allowed) = allowlist {
            if allowed.is_empty() {
                "\n\nAllowed models: (empty allowlist)".to_string()
            } else if allowed.len() <= 5 {
                format!("\n\nAllowed models: {}", allowed.join(", "))
            } else {
                format!(
                    "\n\nAllowed models: {}, {} and {} more",
                    allowed[0],
                    allowed[1],
                    allowed.len() - 2
                )
            }
        } else {
            String::new()
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
    use apxm_core::model_profiles::{ModelProfile, ProfileCandidate};
    use apxm_core::types::{AISOperationType, Value};
    use apxm_graph::{ApxmGraph, GraphNode};
    use std::collections::{HashMap, HashSet};

    fn make_test_graph(profile_names: Vec<&str>) -> ApxmGraph {
        let mut nodes = Vec::new();
        for (i, profile) in profile_names.iter().enumerate() {
            let mut attributes = HashMap::new();
            attributes.insert("model_profile".to_string(), Value::String(profile.to_string()));

            nodes.push(GraphNode {
                id: (i + 1) as u64,
                name: format!("node_{}", i + 1),
                op: AISOperationType::Ask,
                attributes,
            });
        }

        ApxmGraph {
            name: "test_graph".to_string(),
            nodes,
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_nonexistent_profile_fails() {
        let graph = make_test_graph(vec!["nonexistent-profile"]);
        let profiles = HashMap::new();
        let models = HashSet::new();

        let result = validate_model_profile(&graph, &profiles, &models, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found in registry"));
    }

    #[test]
    fn test_empty_profile_generates_warning_but_passes() {
        let graph = make_test_graph(vec!["empty-profile"]);
        let mut profiles = HashMap::new();
        let models = HashSet::new();

        // Register empty profile
        profiles.insert(
            "empty-profile".to_string(),
            ModelProfile {
                name: "empty-profile".to_string(),
                description: "No candidates".to_string(),
                tags: vec![],
                min_context_window: None,
                max_cost_per_1k_input: None,
                candidates: vec![],
            },
        );

        let result = validate_model_profile(&graph, &profiles, &models, None);
        // Should pass but generate warning
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_profile_with_existing_model_passes() {
        let graph = make_test_graph(vec!["test-profile"]);
        let mut profiles = HashMap::new();
        let mut models = HashSet::new();

        // Register model
        models.insert("claude-opus-4-6".to_string());

        // Register profile
        profiles.insert(
            "test-profile".to_string(),
            ModelProfile {
                name: "test-profile".to_string(),
                description: "Test".to_string(),
                tags: vec![],
                min_context_window: None,
                max_cost_per_1k_input: None,
                candidates: vec![ProfileCandidate {
                    model: "claude-opus-4-6".to_string(),
                    priority: 1,
                }],
            },
        );

        let result = validate_model_profile(&graph, &profiles, &models, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_candidate_not_in_model_registry_fails() {
        let graph = make_test_graph(vec!["test-profile"]);
        let mut profiles = HashMap::new();
        let models = HashSet::new(); // Empty - no models

        // Register profile with candidate that doesn't exist in model registry
        profiles.insert(
            "test-profile".to_string(),
            ModelProfile {
                name: "test-profile".to_string(),
                description: "Test".to_string(),
                tags: vec![],
                min_context_window: None,
                max_cost_per_1k_input: None,
                candidates: vec![ProfileCandidate {
                    model: "nonexistent-model".to_string(),
                    priority: 1,
                }],
            },
        );

        let result = validate_model_profile(&graph, &profiles, &models, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not found in model registry"));
    }

    #[test]
    fn test_candidate_not_in_allowlist_fails() {
        let graph = make_test_graph(vec!["test-profile"]);
        let mut profiles = HashMap::new();
        let mut models = HashSet::new();

        // Register model
        models.insert("claude-opus-4-6".to_string());

        // Register profile
        profiles.insert(
            "test-profile".to_string(),
            ModelProfile {
                name: "test-profile".to_string(),
                description: "Test".to_string(),
                tags: vec![],
                min_context_window: None,
                max_cost_per_1k_input: None,
                candidates: vec![ProfileCandidate {
                    model: "claude-opus-4-6".to_string(),
                    priority: 1,
                }],
            },
        );

        // Allowlist doesn't include the model
        let allowlist = vec!["gpt-4o".to_string(), "gemini-2.0-flash".to_string()];

        let result = validate_model_profile(&graph, &profiles, &models, Some(&allowlist));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in allowlist"));
    }

    #[test]
    fn test_all_candidates_in_allowlist_passes() {
        let graph = make_test_graph(vec!["test-profile"]);
        let mut profiles = HashMap::new();
        let mut models = HashSet::new();

        // Register models
        models.insert("claude-opus-4-6".to_string());
        models.insert("gpt-4o".to_string());

        // Register profile with multiple candidates
        profiles.insert(
            "test-profile".to_string(),
            ModelProfile {
                name: "test-profile".to_string(),
                description: "Test".to_string(),
                tags: vec![],
                min_context_window: None,
                max_cost_per_1k_input: None,
                candidates: vec![
                    ProfileCandidate {
                        model: "claude-opus-4-6".to_string(),
                        priority: 1,
                    },
                    ProfileCandidate {
                        model: "gpt-4o".to_string(),
                        priority: 2,
                    },
                ],
            },
        );

        // Allowlist includes both models
        let allowlist = vec![
            "claude-opus-4-6".to_string(),
            "gpt-4o".to_string(),
            "gemini-2.0-flash".to_string(),
        ];

        let result = validate_model_profile(&graph, &profiles, &models, Some(&allowlist));
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_model_profile_attribute_passes() {
        // Create graph with nodes that don't have model_profile attributes
        let mut nodes = vec![];
        let mut attrs = HashMap::new();
        attrs.insert("prompt".to_string(), Value::String("test".to_string()));

        nodes.push(GraphNode {
            id: 1,
            name: "no_profile".to_string(),
            op: AISOperationType::Ask,
            attributes: attrs,
        });

        let graph = ApxmGraph {
            name: "test".to_string(),
            nodes,
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let profiles = HashMap::new();
        let models = HashSet::new();

        let result = validate_model_profile(&graph, &profiles, &models, None);
        // Should pass because node has no model_profile attribute
        assert!(result.is_ok());
    }
}
