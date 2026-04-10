//! Workflow execution engine.

use super::{WorkflowDef, def::GraphStep, template, topo};
use apxm_core::log_info;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// Result of workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub workflow_name: String,
    pub status: WorkflowStatus,
    pub step_results: HashMap<String, StepResult>,
    pub output: Option<String>,
    pub duration_ms: u64,
}

/// Overall workflow execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowStatus {
    Success,
    PartialFailure,
    Failed,
}

/// Result of a single step execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub id: String,
    pub status: StepStatus,
    pub output: Option<String>,
    pub duration_ms: u64,
    pub session_dir: Option<PathBuf>,
    pub error: Option<String>,
}

impl Default for StepResult {
    fn default() -> Self {
        Self {
            id: String::new(),
            status: StepStatus::Skipped,
            output: None,
            duration_ms: 0,
            session_dir: None,
            error: None,
        }
    }
}

/// Status of a single step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Success,
    Failed,
    Skipped,
}

/// Workflow execution engine.
///
/// Orchestrates parallel execution of graph steps with dependency resolution.
pub struct WorkflowRunner {
    pub def: WorkflowDef,
    pub base_dir: PathBuf,
}

impl WorkflowRunner {
    /// Create a new workflow runner.
    ///
    /// `base_dir` is the directory containing the .apxmw file (used to resolve relative graph paths).
    pub fn new(def: WorkflowDef, base_dir: PathBuf) -> Self {
        Self { def, base_dir }
    }

    /// Execute the workflow with the given parameters.
    ///
    /// `args` is a map of parameter name → value.
    ///
    /// # Returns
    ///
    /// A `WorkflowResult` containing the status and results of all steps.
    ///
    /// # Note
    ///
    /// This method requires a runtime executor to be available. Since this is in
    /// apxm-runtime crate, we can't directly use apxm-driver's RuntimeExecutor here.
    /// Instead, this is a placeholder that would be called from the driver/CLI layer
    /// which has access to both the runtime and the linker.
    ///
    /// The actual execution logic will be in the CLI or a higher-level orchestrator.
    pub async fn run(&self, args: HashMap<String, String>) -> anyhow::Result<WorkflowResult> {
        let start = Instant::now();

        // Compute execution phases
        let phases = topo::execution_phases(&self.def.graphs)?;

        log_info!(
            "workflow",
            workflow = %self.def.name,
            phases = phases.len(),
            "Starting workflow execution"
        );

        let step_outputs: HashMap<String, String> = HashMap::new();
        let mut step_results: HashMap<String, StepResult> = HashMap::new();

        // Create workflow session directory
        let paths = apxm_core::paths::ApxmPaths::discover()
            .map_err(|e| anyhow::anyhow!("Failed to discover APXM paths: {}", e))?;
        let workflow_session_dir = paths.sessions_dir()?.join(format!(
            "workflow-{}-{}",
            self.def.name,
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ));
        std::fs::create_dir_all(&workflow_session_dir)?;

        log_info!(
            "workflow",
            session_dir = %workflow_session_dir.display(),
            "Created workflow session directory"
        );

        // Execute phases sequentially, steps within a phase in parallel
        for (_phase_idx, phase) in phases.iter().enumerate() {
            for step_id in phase {
                let step = find_step(&self.def, step_id)?;

                // Check if any dependency failed → skip this step
                let should_skip = step.depends_on.iter().any(|dep| {
                    step_results
                        .get(dep)
                        .map_or(false, |r| r.status != StepStatus::Success)
                });

                if should_skip {
                    log_info!(
                        "workflow",
                        step = %step_id,
                        "Skipping step (failed dependency)"
                    );
                    step_results.insert(
                        step_id.clone(),
                        StepResult {
                            id: step_id.clone(),
                            status: StepStatus::Skipped,
                            ..Default::default()
                        },
                    );
                    continue;
                }

                // Resolve parameters using template resolution
                let _resolved_params: HashMap<String, String> = step
                    .params
                    .iter()
                    .map(|(k, v)| (k.clone(), template::resolve(v, &step_outputs, &args)))
                    .collect();

                let _step_session_dir = workflow_session_dir.join(&step.id);
                let graph_path = self.base_dir.join(&step.path);

                log_info!(
                    "workflow",
                    step = %step_id,
                    graph = %graph_path.display(),
                    "Starting step"
                );

                // NOTE: We can't actually spawn the execution here because we need
                // access to the Linker/RuntimeExecutor which is in apxm-driver.
                // This will be handled by the CLI/driver layer.
                // For now, we'll create a placeholder that returns an error.

                anyhow::bail!(
                    "Workflow execution requires driver-level integration. \
                     Use 'apxm workflow run' command instead of calling WorkflowRunner::run() directly."
                );
            }
        }

        // This code is unreachable due to the bail! above, but kept for reference
        #[allow(unreachable_code)]
        {
            let output = self
                .def
                .output
                .as_ref()
                .map(|tmpl| template::resolve(tmpl, &step_outputs, &args));

            let status = compute_status(&step_results);
            let duration_ms = start.elapsed().as_millis() as u64;

            Ok(WorkflowResult {
                workflow_name: self.def.name.clone(),
                status,
                step_results,
                output,
                duration_ms,
            })
        }
    }
}

/// Find a step by ID in the workflow definition.
fn find_step<'a>(def: &'a WorkflowDef, step_id: &str) -> anyhow::Result<&'a GraphStep> {
    def.graphs
        .iter()
        .find(|s| s.id == step_id)
        .ok_or_else(|| anyhow::anyhow!("Step '{}' not found in workflow", step_id))
}

/// Compute overall workflow status from step results.
fn compute_status(step_results: &HashMap<String, StepResult>) -> WorkflowStatus {
    let mut success_count = 0;
    let mut failed_count = 0;

    for result in step_results.values() {
        match result.status {
            StepStatus::Success => success_count += 1,
            StepStatus::Failed => failed_count += 1,
            StepStatus::Skipped => {}
        }
    }

    if failed_count > 0 {
        if success_count > 0 {
            WorkflowStatus::PartialFailure
        } else {
            WorkflowStatus::Failed
        }
    } else {
        WorkflowStatus::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_status_all_success() {
        let mut results = HashMap::new();
        results.insert(
            "a".to_string(),
            StepResult {
                id: "a".to_string(),
                status: StepStatus::Success,
                ..Default::default()
            },
        );
        results.insert(
            "b".to_string(),
            StepResult {
                id: "b".to_string(),
                status: StepStatus::Success,
                ..Default::default()
            },
        );

        assert_eq!(compute_status(&results), WorkflowStatus::Success);
    }

    #[test]
    fn test_compute_status_all_failed() {
        let mut results = HashMap::new();
        results.insert(
            "a".to_string(),
            StepResult {
                id: "a".to_string(),
                status: StepStatus::Failed,
                ..Default::default()
            },
        );

        assert_eq!(compute_status(&results), WorkflowStatus::Failed);
    }

    #[test]
    fn test_compute_status_partial() {
        let mut results = HashMap::new();
        results.insert(
            "a".to_string(),
            StepResult {
                id: "a".to_string(),
                status: StepStatus::Success,
                ..Default::default()
            },
        );
        results.insert(
            "b".to_string(),
            StepResult {
                id: "b".to_string(),
                status: StepStatus::Failed,
                ..Default::default()
            },
        );

        assert_eq!(compute_status(&results), WorkflowStatus::PartialFailure);
    }

    #[test]
    fn test_find_step() {
        let def = WorkflowDef {
            name: "test".to_string(),
            description: None,
            parameters: vec![],
            graphs: vec![GraphStep {
                id: "a".to_string(),
                path: "a.apxm".to_string(),
                depends_on: vec![],
                params: HashMap::new(),
            }],
            output: None,
        };

        let step = find_step(&def, "a").unwrap();
        assert_eq!(step.id, "a");

        let err = find_step(&def, "unknown");
        assert!(err.is_err());
    }
}
