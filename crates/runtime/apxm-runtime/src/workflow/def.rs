//! Workflow definition types and parsing.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use apxm_core::types::{WorkflowInvocation, WorkflowInvocationKind, WorkflowTarget};

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
    /// Path to the .air file (relative to .apxmw file)
    pub path: String,
    /// Step IDs this step depends on
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Parameters to pass to the graph (param_name → template or literal)
    #[serde(default)]
    pub params: HashMap<String, String>,
}

impl GraphStep {
    /// Resolve this step into an explicit workflow-spawn target.
    pub fn resolved_target(&self, base_dir: &Path) -> WorkflowTarget {
        let path = base_dir.join(&self.path);
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("apxmobj") => WorkflowTarget::ArtifactPath {
                path: path.display().to_string(),
            },
            Some("apxmw") => WorkflowTarget::WorkflowPath {
                path: path.display().to_string(),
            },
            _ => WorkflowTarget::GraphPath {
                path: path.display().to_string(),
            },
        }
    }

    /// Build the explicit invocation frame for this workflow step.
    pub fn spawn_invocation(
        &self,
        base_dir: &Path,
        args: HashMap<String, serde_json::Value>,
    ) -> WorkflowInvocation {
        WorkflowInvocation {
            kind: WorkflowInvocationKind::WorkflowSpawn,
            target: self.resolved_target(base_dir),
            args,
            await_result: true,
            session_root: None,
            session_dir: None,
        }
    }
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
                    path: "a.air".to_string(),
                    depends_on: vec![],
                    params: HashMap::new(),
                },
                GraphStep {
                    id: "a".to_string(),
                    path: "b.air".to_string(),
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
                path: "a.air".to_string(),
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
                    path: "a.air".to_string(),
                    depends_on: vec!["b".to_string()],
                    params: HashMap::new(),
                },
                GraphStep {
                    id: "b".to_string(),
                    path: "b.air".to_string(),
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
                    path: "a.air".to_string(),
                    depends_on: vec![],
                    params: HashMap::new(),
                },
                GraphStep {
                    id: "b".to_string(),
                    path: "b.air".to_string(),
                    depends_on: vec!["a".to_string()],
                    params: HashMap::new(),
                },
            ],
            output: None,
        };

        let errors = def.validate();
        assert!(errors.is_empty());
    }

    #[test]
    fn graph_step_resolves_air_to_graph_target() {
        let step = GraphStep {
            id: "review".to_string(),
            path: "graphs/reviewer.air".to_string(),
            depends_on: vec![],
            params: HashMap::new(),
        };
        let target = step.resolved_target(Path::new("/tmp/project"));
        assert!(matches!(target, WorkflowTarget::GraphPath { .. }));
        assert_eq!(target.label(), "/tmp/project/graphs/reviewer.air");
    }

    #[test]
    fn graph_step_spawn_invocation_uses_workflow_spawn_kind() {
        let step = GraphStep {
            id: "review".to_string(),
            path: "flows/review.apxmw".to_string(),
            depends_on: vec![],
            params: HashMap::from([("topic".to_string(), "{{topic}}".to_string())]),
        };
        let invocation = step.spawn_invocation(
            Path::new("/workspace"),
            HashMap::from([("topic".to_string(), serde_json::json!("apxm"))]),
        );
        assert_eq!(invocation.kind, WorkflowInvocationKind::WorkflowSpawn);
        assert!(invocation.is_cross_execution());
        assert!(matches!(
            invocation.target,
            WorkflowTarget::WorkflowPath { .. }
        ));
        assert_eq!(
            invocation.args.get("topic"),
            Some(&serde_json::json!("apxm"))
        );
    }
}
