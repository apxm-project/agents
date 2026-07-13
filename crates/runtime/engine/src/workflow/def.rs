//! Workflow definition types and parsing.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use apxm_core::types::{WorkflowInvocation, WorkflowInvocationKind, WorkflowTarget};

/// A workflow that composes multiple workflow steps.
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
    /// Workflow steps to execute
    pub steps: Vec<WorkflowStep>,
    /// Maximum number of workflow steps that may execute at once.
    ///
    /// An omitted value preserves the historic unlimited workflow-level
    /// parallelism; each child graph still applies its own runtime limits.
    #[serde(default)]
    pub max_concurrency: Option<usize>,
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

/// A single workflow step in the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Unique step ID
    pub id: String,
    /// Path to the .air file (relative to .apxmw file)
    pub path: String,
    /// Step IDs this step depends on
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Parameters to pass to the step (param_name -> template or literal).
    #[serde(default)]
    pub params: HashMap<String, String>,
}

impl WorkflowStep {
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
            _ => WorkflowTarget::AirPath {
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
            parent_execution_id: None,
            parent_session_dir: None,
            parent_scope_id: None,
            spawn_node_id: None,
            authority_metadata: HashMap::new(),
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
        for step in &self.steps {
            if !seen_ids.insert(&step.id) {
                errors.push(format!("Duplicate step ID: {}", step.id));
            }
        }

        // Check for unknown dependencies
        let valid_ids: HashSet<_> = self.steps.iter().map(|s| &s.id).collect();
        for step in &self.steps {
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

        if self.max_concurrency == Some(0) {
            errors.push("Workflow max_concurrency must be greater than zero".to_string());
        }

        errors
    }

    /// Check for cycles in the dependency graph.
    fn check_cycles(&self) -> Result<(), String> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for step in &self.steps {
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
            .steps
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
    //! Workflow-definition tests cover scheduler-facing validation rules.

    use super::*;

    /// Build the minimum valid workflow definition for validation tests.
    fn workflow(max_concurrency: Option<usize>) -> WorkflowDef {
        WorkflowDef {
            name: "test".to_string(),
            description: None,
            parameters: Vec::new(),
            steps: Vec::new(),
            max_concurrency,
            output: None,
        }
    }

    #[test]
    fn rejects_zero_workflow_concurrency() {
        assert_eq!(
            workflow(Some(0)).validate(),
            vec!["Workflow max_concurrency must be greater than zero"]
        );
    }

    #[test]
    fn accepts_an_omitted_or_positive_workflow_concurrency_limit() {
        assert!(workflow(None).validate().is_empty());
        assert!(workflow(Some(1)).validate().is_empty());
    }
}
