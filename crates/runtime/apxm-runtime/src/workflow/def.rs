//! Workflow definition types and parsing.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// A workflow that composes multiple graphs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    /// Workflow name
    pub name: String,
    /// Optional description
    #[serde(default)]
    pub description: Option<String>,
    /// Workflow-level parameters (passed via CLI)
    #[serde(default)]
    pub parameters: Vec<WorkflowParam>,
    /// Graph steps to execute
    pub graphs: Vec<GraphStep>,
    /// Optional output template (e.g., "{{synthesize.output}}")
    #[serde(default)]
    pub output: Option<String>,
}

/// A workflow parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowParam {
    pub name: String,
    pub type_name: String,
}

/// A single graph step in the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStep {
    /// Unique step ID
    pub id: String,
    /// Path to the .apxm file (relative to .apxmw file)
    pub path: String,
    /// Step IDs this step depends on
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Parameters to pass to the graph (param_name → template or literal)
    #[serde(default)]
    pub params: HashMap<String, String>,
}

impl WorkflowDef {
    /// Load a workflow from a .apxmw file.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let def: WorkflowDef = serde_json::from_str(&content)?;
        Ok(def)
    }

    /// Validate the workflow definition.
    ///
    /// Returns a list of validation errors. Empty list means valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Check for duplicate step IDs
        let mut seen_ids = HashSet::new();
        for step in &self.graphs {
            if !seen_ids.insert(&step.id) {
                errors.push(format!("Duplicate step ID: {}", step.id));
            }
        }

        // Check for unknown dependencies
        let valid_ids: HashSet<_> = self.graphs.iter().map(|s| &s.id).collect();
        for step in &self.graphs {
            for dep in &step.depends_on {
                if !valid_ids.contains(&dep) {
                    errors.push(format!(
                        "Step '{}' depends on unknown step '{}'",
                        step.id, dep
                    ));
                }
            }
        }

        // Check for cycles using DFS (only if no unknown dependencies)
        // because cycle check will fail with unknown steps
        if errors.is_empty() {
            if let Err(cycle_error) = self.check_cycles() {
                errors.push(cycle_error);
            }
        }

        errors
    }

    /// Check for cycles in the dependency graph.
    fn check_cycles(&self) -> Result<(), String> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for step in &self.graphs {
            if !visited.contains(&step.id) {
                if self.has_cycle(&step.id, &mut visited, &mut rec_stack)? {
                    return Err(format!("Cycle detected involving step '{}'", step.id));
                }
            }
        }

        Ok(())
    }

    fn has_cycle(
        &self,
        step_id: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
    ) -> Result<bool, String> {
        visited.insert(step_id.to_string());
        rec_stack.insert(step_id.to_string());

        let step = self
            .graphs
            .iter()
            .find(|s| s.id == step_id)
            .ok_or_else(|| format!("Step '{}' not found", step_id))?;

        for dep in &step.depends_on {
            if !visited.contains(dep) {
                if self.has_cycle(dep, visited, rec_stack)? {
                    return Ok(true);
                }
            } else if rec_stack.contains(dep) {
                return Ok(true);
            }
        }

        rec_stack.remove(step_id);
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_duplicate_ids() {
        let def = WorkflowDef {
            name: "test".to_string(),
            description: None,
            parameters: vec![],
            graphs: vec![
                GraphStep {
                    id: "a".to_string(),
                    path: "a.apxm".to_string(),
                    depends_on: vec![],
                    params: HashMap::new(),
                },
                GraphStep {
                    id: "a".to_string(),
                    path: "b.apxm".to_string(),
                    depends_on: vec![],
                    params: HashMap::new(),
                },
            ],
            output: None,
        };

        let errors = def.validate();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("Duplicate step ID"));
    }

    #[test]
    fn test_validate_unknown_dependency() {
        let def = WorkflowDef {
            name: "test".to_string(),
            description: None,
            parameters: vec![],
            graphs: vec![GraphStep {
                id: "a".to_string(),
                path: "a.apxm".to_string(),
                depends_on: vec!["unknown".to_string()],
                params: HashMap::new(),
            }],
            output: None,
        };

        let errors = def.validate();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unknown step"));
    }

    #[test]
    fn test_validate_cycle() {
        let def = WorkflowDef {
            name: "test".to_string(),
            description: None,
            parameters: vec![],
            graphs: vec![
                GraphStep {
                    id: "a".to_string(),
                    path: "a.apxm".to_string(),
                    depends_on: vec!["b".to_string()],
                    params: HashMap::new(),
                },
                GraphStep {
                    id: "b".to_string(),
                    path: "b.apxm".to_string(),
                    depends_on: vec!["a".to_string()],
                    params: HashMap::new(),
                },
            ],
            output: None,
        };

        let errors = def.validate();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("Cycle detected"));
    }

    #[test]
    fn test_validate_valid_workflow() {
        let def = WorkflowDef {
            name: "test".to_string(),
            description: None,
            parameters: vec![],
            graphs: vec![
                GraphStep {
                    id: "a".to_string(),
                    path: "a.apxm".to_string(),
                    depends_on: vec![],
                    params: HashMap::new(),
                },
                GraphStep {
                    id: "b".to_string(),
                    path: "b.apxm".to_string(),
                    depends_on: vec!["a".to_string()],
                    params: HashMap::new(),
                },
            ],
            output: None,
        };

        let errors = def.validate();
        assert!(errors.is_empty());
    }
}
