use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};

use apxm_artifact::Artifact;
use apxm_core::paths::ApxmPaths;
use apxm_core::types::{SessionStatus, WorkflowInvocation, WorkflowTarget};
use apxm_runtime::{
    CancellationToken, ExecutionEventEmitter, Runtime, RuntimeError, RuntimeExecutionResult,
    WorkflowSpawnResult, WorkflowSpawner,
};
use async_trait::async_trait;
use futures::future::join_all;

use crate::compiler::Compiler;
use crate::hooks;
use crate::session_output::{SessionEventEmitter, SessionOutputWriter, SessionProvenance};

pub struct DriverWorkflowSpawner {
    runtime: Mutex<Option<Weak<Runtime>>>,
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
}

struct WorkflowPhaseStepOutcome {
    step_id: String,
    step_index: usize,
    duration_ms: u64,
    workflow_session_dir: PathBuf,
    outcome: Result<WorkflowSpawnResult, RuntimeError>,
}

impl DriverWorkflowSpawner {
    pub fn new(configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>) -> Self {
        Self {
            runtime: Mutex::new(None),
            configured_emitter,
        }
    }

    pub fn attach_runtime(&self, runtime: Weak<Runtime>) {
        *self.runtime.lock().unwrap() = Some(runtime);
    }

    fn execute_invocation<'a>(
        &'a self,
        invocation: WorkflowInvocation,
        parent_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        cancellation_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = Result<WorkflowSpawnResult, RuntimeError>> + Send + 'a>> {
        Box::pin(async move {
            let session_base_dir = resolve_session_base_dir(invocation.session_root.as_deref())?;
            match &invocation.target {
                WorkflowTarget::AirPath { path } => {
                    self.execute_air_path(
                        Path::new(&path),
                        &invocation.args,
                        &session_base_dir,
                        &invocation,
                        parent_emitter,
                        cancellation_token,
                    )
                    .await
                }
                WorkflowTarget::ArtifactPath { path } => {
                    self.execute_artifact_path(
                        Path::new(&path),
                        &invocation.args,
                        &session_base_dir,
                        &invocation,
                        parent_emitter,
                        cancellation_token,
                    )
                    .await
                }
                WorkflowTarget::WorkflowPath { path } => {
                    self.execute_workflow_path(
                        Path::new(&path),
                        invocation.args.clone(),
                        &session_base_dir,
                        &invocation,
                        parent_emitter,
                        cancellation_token,
                    )
                    .await
                }
                WorkflowTarget::RegisteredFlow {
                    agent_name,
                    flow_name,
                } => Err(RuntimeError::State(format!(
                    "WORKFLOW_SPAWN does not support registered-flow targets. Use FLOW_CALL for '{}.{}'",
                    agent_name, flow_name
                ))),
            }
        })
    }

    async fn execute_air_path(
        &self,
        air_path: &Path,
        args: &HashMap<String, serde_json::Value>,
        session_base_dir: &Path,
        invocation: &WorkflowInvocation,
        parent_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<WorkflowSpawnResult, RuntimeError> {
        let artifact = {
            let compiler = Compiler::new()
                .map_err(|e| RuntimeError::State(format!("Failed to initialize compiler: {e}")))?;
            let module = compiler
                .compile(air_path)
                .map_err(|e| RuntimeError::State(format!("Failed to compile AIR: {e}")))?;
            let artifact_bytes = module
                .generate_artifact_bytes()
                .map_err(|e| RuntimeError::State(format!("Failed to emit AIR artifact: {e}")))?;
            Artifact::from_bytes(&artifact_bytes)
                .map_err(|e| RuntimeError::State(format!("Failed to load AIR artifact: {e}")))?
        };

        let ordered_args = ordered_args_from_artifact(&artifact, args)?;
        let input_graph = load_graph_for_session(air_path).ok();
        let execution_id = child_execution_id("air", air_path);
        let provenance = provenance_from_invocation(invocation);
        let writer = create_session_writer(
            session_base_dir,
            &execution_id,
            air_path.file_stem().and_then(|s| s.to_str()),
            input_graph.as_ref(),
            &provenance,
        )?;
        let session_dir = writer.session_dir().to_path_buf();
        let runtime = self.runtime()?;
        let emitter = create_session_emitter(
            writer.session_dir(),
            &execution_id,
            input_graph.as_ref(),
            artifact.entry_dag().map(|dag| dag.nodes.len()),
            runtime.memory_system_arc(),
            parent_emitter,
            self.configured_emitter.as_ref().map(Arc::clone),
            &provenance,
        )?;

        let execution = if let Some(cancellation_token) = cancellation_token {
            runtime
                .execute_artifact_with_session_emitter_metadata_and_cancellation(
                    artifact,
                    ordered_args,
                    None,
                    emitter,
                    Some(session_dir.to_string_lossy().to_string()),
                    invocation.authority_metadata.clone(),
                    cancellation_token,
                )
                .await
        } else {
            runtime
                .execute_artifact_with_session_emitter_and_metadata(
                    artifact,
                    ordered_args,
                    None,
                    emitter,
                    Some(session_dir.to_string_lossy().to_string()),
                    invocation.authority_metadata.clone(),
                )
                .await
        };

        finalize_child_session(
            &writer,
            &execution_id,
            air_path.file_stem().and_then(|s| s.to_str()),
            execution.as_ref(),
            &provenance,
        )?;

        let execution = execution?;
        Ok(WorkflowSpawnResult {
            value: primary_result_value(&execution),
            session_dir: Some(session_dir.to_string_lossy().to_string()),
        })
    }

    async fn execute_artifact_path(
        &self,
        artifact_path: &Path,
        args: &HashMap<String, serde_json::Value>,
        session_base_dir: &Path,
        invocation: &WorkflowInvocation,
        parent_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<WorkflowSpawnResult, RuntimeError> {
        let artifact = Artifact::read_from_path(artifact_path).map_err(|e| {
            RuntimeError::State(format!(
                "Failed to read artifact '{}': {e}",
                artifact_path.display()
            ))
        })?;
        let ordered_args = ordered_args_from_artifact(&artifact, args)?;
        let execution_id = child_execution_id("artifact", artifact_path);
        let provenance = provenance_from_invocation(invocation);
        let writer = create_session_writer(
            session_base_dir,
            &execution_id,
            artifact_path.file_stem().and_then(|s| s.to_str()),
            None,
            &provenance,
        )?;
        let session_dir = writer.session_dir().to_path_buf();
        let runtime = self.runtime()?;
        let emitter = create_session_emitter(
            writer.session_dir(),
            &execution_id,
            None,
            artifact.entry_dag().map(|dag| dag.nodes.len()),
            runtime.memory_system_arc(),
            parent_emitter,
            self.configured_emitter.as_ref().map(Arc::clone),
            &provenance,
        )?;

        let execution = if let Some(cancellation_token) = cancellation_token {
            runtime
                .execute_artifact_with_session_emitter_metadata_and_cancellation(
                    artifact,
                    ordered_args,
                    None,
                    emitter,
                    Some(session_dir.to_string_lossy().to_string()),
                    invocation.authority_metadata.clone(),
                    cancellation_token,
                )
                .await
        } else {
            runtime
                .execute_artifact_with_session_emitter_and_metadata(
                    artifact,
                    ordered_args,
                    None,
                    emitter,
                    Some(session_dir.to_string_lossy().to_string()),
                    invocation.authority_metadata.clone(),
                )
                .await
        };

        finalize_child_session(
            &writer,
            &execution_id,
            artifact_path.file_stem().and_then(|s| s.to_str()),
            execution.as_ref(),
            &provenance,
        )?;

        let execution = execution?;
        Ok(WorkflowSpawnResult {
            value: primary_result_value(&execution),
            session_dir: Some(session_dir.to_string_lossy().to_string()),
        })
    }

    async fn execute_workflow_path(
        &self,
        workflow_path: &Path,
        args: HashMap<String, serde_json::Value>,
        session_base_dir: &Path,
        invocation: &WorkflowInvocation,
        parent_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<WorkflowSpawnResult, RuntimeError> {
        use apxm_runtime::workflow::{WorkflowDef, WorkflowResult, execution_phases, resolve};

        let def = WorkflowDef::from_file(workflow_path).map_err(|e| {
            RuntimeError::State(format!(
                "Failed to load workflow '{}': {e}",
                workflow_path.display()
            ))
        })?;
        let errors = def.validate();
        if !errors.is_empty() {
            return Err(RuntimeError::State(format!(
                "Workflow validation failed: {}",
                errors.join(", ")
            )));
        }

        let param_values = normalize_named_args(args);
        validate_workflow_params(&def, &param_values)?;

        let base_dir = workflow_path
            .parent()
            .ok_or_else(|| {
                RuntimeError::State("Workflow path has no parent directory".to_string())
            })?
            .to_path_buf();
        let workflow_session_dir = create_workflow_session_dir(session_base_dir, &def.name)?;
        apxm_runtime::workflow::write_workflow_session_started(
            &workflow_session_dir,
            &def.name,
            def.steps.len(),
        )
        .map_err(|e| {
            RuntimeError::State(format!("Failed to write workflow session start files: {e}"))
        })?;
        let workflow_session_dir_text = workflow_session_dir.to_string_lossy().to_string();
        if let Some(emitter) = parent_emitter.as_ref() {
            emitter.emit_workflow_started(&def.name, &workflow_session_dir_text, def.steps.len());
        }
        let step_index_by_id: HashMap<String, usize> = def
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| (step.id.clone(), index))
            .collect();
        let mut step_outputs: HashMap<String, String> = HashMap::new();
        let mut step_results = HashMap::new();
        let workflow_start = std::time::Instant::now();

        for phase in execution_phases(&def.steps)
            .map_err(|e| RuntimeError::State(format!("Workflow planning failed: {e}")))?
        {
            if cancellation_token
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
            {
                return Err(RuntimeError::SchedulerCancelled);
            }
            let mut phase_jobs = Vec::new();

            for step_id in phase {
                let step = def
                    .steps
                    .iter()
                    .find(|candidate| candidate.id == step_id)
                    .ok_or_else(|| {
                        RuntimeError::State(format!("Unknown workflow step '{step_id}'"))
                    })?;
                let step_index = step_index_by_id.get(&step_id).copied().ok_or_else(|| {
                    RuntimeError::State(format!("Unknown workflow step '{step_id}'"))
                })?;

                let should_skip = step.depends_on.iter().any(|dep| {
                    step_results.get(dep).is_some_and(
                        |result: &apxm_runtime::workflow::StepResult| {
                            result.status != apxm_runtime::workflow::StepStatus::Success
                        },
                    )
                });
                if should_skip {
                    apxm_runtime::workflow::write_workflow_step_finished(
                        &workflow_session_dir,
                        &def.name,
                        &step_id,
                        step_index,
                        apxm_runtime::workflow::StepStatus::Skipped,
                        0,
                        step_results.len() + 1,
                        def.steps.len(),
                        workflow_start.elapsed().as_millis(),
                    )
                    .map_err(|e| {
                        RuntimeError::State(format!(
                            "Failed to write skipped workflow step '{step_id}': {e}"
                        ))
                    })?;
                    if let Some(emitter) = parent_emitter.as_ref() {
                        emitter.emit_workflow_step_completed(
                            &def.name,
                            &workflow_session_dir_text,
                            &step_id,
                            step_index,
                            workflow_step_status_wire(apxm_runtime::workflow::StepStatus::Skipped),
                            false,
                            std::time::Duration::ZERO,
                            None,
                            Some("skipped because a dependency did not succeed"),
                        );
                    }
                    step_results.insert(
                        step_id.clone(),
                        apxm_runtime::workflow::StepResult {
                            id: step_id.clone(),
                            status: apxm_runtime::workflow::StepStatus::Skipped,
                            ..Default::default()
                        },
                    );
                    continue;
                }

                let resolved_params: HashMap<String, serde_json::Value> = step
                    .params
                    .iter()
                    .map(|(key, template)| {
                        (
                            key.clone(),
                            serde_json::Value::String(resolve(
                                template,
                                &step_outputs,
                                &param_values,
                            )),
                        )
                    })
                    .collect();
                let mut child_invocation = step.spawn_invocation(&base_dir, resolved_params);
                child_invocation.session_root =
                    Some(workflow_session_dir.to_string_lossy().to_string());
                child_invocation.authority_metadata = invocation.authority_metadata.clone();

                apxm_runtime::workflow::write_workflow_step_started(
                    &workflow_session_dir,
                    &def.name,
                    &step_id,
                    step_index,
                    step_results.len(),
                    def.steps.len(),
                    workflow_start.elapsed().as_millis(),
                )
                .map_err(|e| {
                    RuntimeError::State(format!(
                        "Failed to write workflow step start for '{step_id}': {e}"
                    ))
                })?;
                if let Some(emitter) = parent_emitter.as_ref() {
                    emitter.emit_workflow_step_started(
                        &def.name,
                        &workflow_session_dir_text,
                        &step_id,
                        step_index,
                        def.steps.len(),
                    );
                }

                let step_id = step.id.clone();
                let workflow_session_dir = workflow_session_dir.clone();
                let parent_emitter = parent_emitter.as_ref().map(Arc::clone);
                let child_cancellation = cancellation_token.as_ref().map(CancellationToken::child);
                phase_jobs.push(async move {
                    let step_start = std::time::Instant::now();
                    let outcome = self
                        .execute_invocation(child_invocation, parent_emitter, child_cancellation)
                        .await;
                    WorkflowPhaseStepOutcome {
                        step_id,
                        step_index,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                        workflow_session_dir,
                        outcome,
                    }
                });
            }

            let mut phase_outputs = Vec::new();
            for completed in join_all(phase_jobs).await {
                match completed.outcome {
                    Ok(child_result) => {
                        let output = value_to_output_string(&child_result.value);
                        let child_session_dir = child_result.session_dir.map(PathBuf::from);
                        let child_session_dir_text = child_session_dir
                            .as_ref()
                            .map(|path| path.to_string_lossy().to_string());
                        apxm_runtime::workflow::write_workflow_step_finished(
                            &workflow_session_dir,
                            &def.name,
                            &completed.step_id,
                            completed.step_index,
                            apxm_runtime::workflow::StepStatus::Success,
                            completed.duration_ms,
                            step_results.len() + 1,
                            def.steps.len(),
                            workflow_start.elapsed().as_millis(),
                        )
                        .map_err(|e| {
                            RuntimeError::State(format!(
                                "Failed to write completed workflow step '{}': {e}",
                                completed.step_id
                            ))
                        })?;
                        if let Some(emitter) = parent_emitter.as_ref() {
                            emitter.emit_workflow_step_completed(
                                &def.name,
                                &workflow_session_dir_text,
                                &completed.step_id,
                                completed.step_index,
                                workflow_step_status_wire(
                                    apxm_runtime::workflow::StepStatus::Success,
                                ),
                                true,
                                std::time::Duration::from_millis(completed.duration_ms),
                                child_session_dir_text.as_deref(),
                                None,
                            );
                        }
                        if let Some(ref text) = output {
                            phase_outputs.push((completed.step_id.clone(), text.clone()));
                        }
                        step_results.insert(
                            completed.step_id.clone(),
                            apxm_runtime::workflow::StepResult {
                                id: completed.step_id,
                                status: apxm_runtime::workflow::StepStatus::Success,
                                output,
                                duration_ms: completed.duration_ms,
                                session_dir: child_session_dir,
                                error: None,
                            },
                        );
                    }
                    Err(error) => {
                        let error_text = error.to_string();
                        let fallback_session_dir_text =
                            completed.workflow_session_dir.to_string_lossy().to_string();
                        apxm_runtime::workflow::write_workflow_step_finished(
                            &workflow_session_dir,
                            &def.name,
                            &completed.step_id,
                            completed.step_index,
                            apxm_runtime::workflow::StepStatus::Failed,
                            completed.duration_ms,
                            step_results.len() + 1,
                            def.steps.len(),
                            workflow_start.elapsed().as_millis(),
                        )
                        .map_err(|e| {
                            RuntimeError::State(format!(
                                "Failed to write failed workflow step '{}': {e}",
                                completed.step_id
                            ))
                        })?;
                        if let Some(emitter) = parent_emitter.as_ref() {
                            emitter.emit_workflow_step_completed(
                                &def.name,
                                &workflow_session_dir_text,
                                &completed.step_id,
                                completed.step_index,
                                workflow_step_status_wire(
                                    apxm_runtime::workflow::StepStatus::Failed,
                                ),
                                false,
                                std::time::Duration::from_millis(completed.duration_ms),
                                Some(&fallback_session_dir_text),
                                Some(&error_text),
                            );
                        }
                        step_results.insert(
                            completed.step_id.clone(),
                            apxm_runtime::workflow::StepResult {
                                id: completed.step_id,
                                status: apxm_runtime::workflow::StepStatus::Failed,
                                output: None,
                                duration_ms: completed.duration_ms,
                                session_dir: Some(completed.workflow_session_dir),
                                error: Some(error_text),
                            },
                        );
                    }
                }
            }

            for (step_id, output) in phase_outputs {
                step_outputs.insert(step_id, output);
            }
        }

        let workflow_result = WorkflowResult {
            workflow_name: def.name.clone(),
            status: workflow_status_from_steps(&step_results),
            step_results,
            output: def
                .output
                .as_ref()
                .map(|template| resolve(template, &step_outputs, &param_values)),
            duration_ms: workflow_start.elapsed().as_millis() as u64,
        };
        apxm_runtime::workflow::write_workflow_session_finished(
            &workflow_session_dir,
            &workflow_result,
        )
        .map_err(|e| {
            RuntimeError::State(format!(
                "Failed to write workflow session result files: {e}"
            ))
        })?;
        if let Some(emitter) = parent_emitter.as_ref() {
            emitter.emit_workflow_finished(
                &workflow_result.workflow_name,
                &workflow_session_dir_text,
                workflow_status_wire(workflow_result.status),
                workflow_result.status == apxm_runtime::workflow::WorkflowStatus::Success,
                std::time::Duration::from_millis(workflow_result.duration_ms),
                workflow_result.step_results.len(),
            );
        }

        match workflow_result.status {
            apxm_runtime::workflow::WorkflowStatus::Success => Ok(WorkflowSpawnResult {
                value: workflow_result.output.map_or(
                    apxm_core::types::Value::Null,
                    apxm_core::types::Value::String,
                ),
                session_dir: Some(workflow_session_dir.to_string_lossy().to_string()),
            }),
            apxm_runtime::workflow::WorkflowStatus::PartialFailure
            | apxm_runtime::workflow::WorkflowStatus::Failed => Err(RuntimeError::State(format!(
                "Workflow '{}' completed with status {:?}",
                workflow_result.workflow_name, workflow_result.status
            ))),
        }
    }

    fn runtime(&self) -> Result<Arc<Runtime>, RuntimeError> {
        self.runtime
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or_else(|| {
                RuntimeError::State(
                    "Driver workflow spawner lost access to the runtime".to_string(),
                )
            })
    }
}

#[async_trait]
impl WorkflowSpawner for DriverWorkflowSpawner {
    async fn spawn_workflow(
        &self,
        invocation: WorkflowInvocation,
        parent_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        parent_cancellation: Option<CancellationToken>,
    ) -> Result<WorkflowSpawnResult, RuntimeError> {
        self.execute_invocation(invocation, parent_emitter, parent_cancellation)
            .await
    }
}

fn resolve_session_base_dir(explicit_root: Option<&str>) -> Result<PathBuf, RuntimeError> {
    if let Some(root) = explicit_root {
        let path = PathBuf::from(root);
        std::fs::create_dir_all(&path).map_err(|e| {
            RuntimeError::State(format!(
                "Failed to create workflow session root '{}': {e}",
                path.display()
            ))
        })?;
        return Ok(path);
    }

    ApxmPaths::discover()
        .and_then(|paths| paths.sessions_dir())
        .map_err(|e| RuntimeError::State(format!("Failed to resolve sessions dir: {e}")))
}

fn normalize_named_args(args: HashMap<String, serde_json::Value>) -> HashMap<String, String> {
    args.into_iter()
        .map(|(key, value)| {
            let normalized = match value {
                serde_json::Value::String(text) => text,
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            (key, normalized)
        })
        .collect()
}

fn ordered_args_from_artifact(
    artifact: &Artifact,
    args: &HashMap<String, serde_json::Value>,
) -> Result<Vec<String>, RuntimeError> {
    let named = normalize_named_args(args.clone());
    let entry = artifact
        .entry_dag()
        .ok_or_else(|| RuntimeError::State("Artifact contains no entry DAG".to_string()))?;

    let declared = &entry.metadata.parameters;
    let unknown: Vec<String> = named
        .keys()
        .filter(|name| !declared.iter().any(|param| param.name == name.as_str()))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(RuntimeError::State(format!(
            "Unknown child argument(s): {}. Expected: {}",
            unknown.join(", "),
            declared
                .iter()
                .map(|param| param.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let missing: Vec<String> = declared
        .iter()
        .filter(|param| !named.contains_key(&param.name))
        .map(|param| param.name.clone())
        .collect();
    if !missing.is_empty() {
        return Err(RuntimeError::State(format!(
            "Missing child argument(s): {}",
            missing.join(", ")
        )));
    }

    Ok(declared
        .iter()
        .map(|param| named.get(&param.name).cloned().unwrap_or_default())
        .collect())
}

fn validate_workflow_params(
    def: &apxm_runtime::workflow::WorkflowDef,
    params: &HashMap<String, String>,
) -> Result<(), RuntimeError> {
    let missing: Vec<String> = def
        .parameters
        .iter()
        .filter(|param| !params.contains_key(&param.name))
        .map(|param| param.name.clone())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(RuntimeError::State(format!(
            "Missing workflow parameter(s): {}",
            missing.join(", ")
        )))
    }
}

fn workflow_status_from_steps(
    step_results: &HashMap<String, apxm_runtime::workflow::StepResult>,
) -> apxm_runtime::workflow::WorkflowStatus {
    let mut success_count = 0;
    let mut failed_count = 0;

    for result in step_results.values() {
        match result.status {
            apxm_runtime::workflow::StepStatus::Success => success_count += 1,
            apxm_runtime::workflow::StepStatus::Failed => failed_count += 1,
            apxm_runtime::workflow::StepStatus::Skipped => {}
        }
    }

    if failed_count > 0 {
        if success_count > 0 {
            apxm_runtime::workflow::WorkflowStatus::PartialFailure
        } else {
            apxm_runtime::workflow::WorkflowStatus::Failed
        }
    } else {
        apxm_runtime::workflow::WorkflowStatus::Success
    }
}

fn workflow_step_status_wire(status: apxm_runtime::workflow::StepStatus) -> &'static str {
    match status {
        apxm_runtime::workflow::StepStatus::Success => "success",
        apxm_runtime::workflow::StepStatus::Failed => "failed",
        apxm_runtime::workflow::StepStatus::Skipped => "skipped",
    }
}

fn workflow_status_wire(status: apxm_runtime::workflow::WorkflowStatus) -> &'static str {
    match status {
        apxm_runtime::workflow::WorkflowStatus::Success => "success",
        apxm_runtime::workflow::WorkflowStatus::PartialFailure => "partial_failure",
        apxm_runtime::workflow::WorkflowStatus::Failed => "failed",
    }
}

fn child_execution_id(prefix: &str, path: &Path) -> String {
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(prefix);
    format!(
        "{prefix}-{name}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S-%6f")
    )
}

fn create_workflow_session_dir(
    session_base_dir: &Path,
    workflow_name: &str,
) -> Result<PathBuf, RuntimeError> {
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S-%6f");
    let mut dir = session_base_dir.join(format!("workflow-{workflow_name}-{timestamp}"));
    let mut attempt = 1_u32;
    while dir.exists() {
        dir = session_base_dir.join(format!("workflow-{workflow_name}-{timestamp}-{attempt}"));
        attempt += 1;
    }
    std::fs::create_dir_all(&dir).map_err(|e| {
        RuntimeError::State(format!(
            "Failed to create workflow session directory '{}': {e}",
            dir.display()
        ))
    })?;
    Ok(dir)
}

fn create_session_writer(
    session_base_dir: &Path,
    execution_id: &str,
    workflow_name: Option<&str>,
    input_graph: Option<&apxm_compiler::AirModule>,
    provenance: &SessionProvenance,
) -> Result<SessionOutputWriter, RuntimeError> {
    let writer = SessionOutputWriter::new(session_base_dir, execution_id).map_err(|e| {
        RuntimeError::State(format!(
            "Failed to create child session directory '{}': {e}",
            session_base_dir.display()
        ))
    })?;
    writer
        .write_manifest_with_provenance(
            execution_id,
            workflow_name,
            SessionStatus::Running,
            0,
            input_graph
                .map(|graph| graph.nodes.len())
                .unwrap_or_default(),
            false,
            provenance,
        )
        .map_err(|e| RuntimeError::State(format!("Failed to write child session manifest: {e}")))?;
    if let Some(graph) = input_graph {
        writer
            .write_input_air(graph)
            .map_err(|e| RuntimeError::State(format!("Failed to write child input graph: {e}")))?;
    }
    Ok(writer)
}

fn create_session_emitter(
    session_dir: &Path,
    execution_id: &str,
    input_graph: Option<&apxm_compiler::AirModule>,
    total_nodes: Option<usize>,
    memory: Arc<apxm_runtime::memory::MemorySystem>,
    parent_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
    provenance: &SessionProvenance,
) -> Result<Option<Arc<dyn ExecutionEventEmitter>>, RuntimeError> {
    let project_root = std::env::current_dir().ok();
    let emitter = Arc::new(
        SessionEventEmitter::new_with_provenance(
            session_dir,
            execution_id.to_string(),
            input_graph,
            project_root.as_deref(),
            provenance.clone(),
        )
        .map_err(|e| RuntimeError::State(format!("Failed to create session emitter: {e}")))?,
    );
    if let Some(total_nodes) = total_nodes {
        emitter.set_total_nodes(total_nodes as u64);
    }
    emitter.set_memory(memory);
    let session_emitter = Some(emitter as Arc<dyn ExecutionEventEmitter>);
    let composed = hooks::compose_emitters(session_emitter, parent_emitter);
    Ok(hooks::compose_emitters(composed, configured_emitter))
}

fn finalize_child_session(
    writer: &SessionOutputWriter,
    execution_id: &str,
    workflow_name: Option<&str>,
    execution: Result<&RuntimeExecutionResult, &RuntimeError>,
    provenance: &SessionProvenance,
) -> Result<(), RuntimeError> {
    match execution {
        Ok(execution) => writer
            .finalize_with_provenance(
                execution_id,
                workflow_name,
                execution.stats.duration_ms,
                execution.stats.executed_nodes + execution.stats.failed_nodes,
                execution.stats.failed_nodes == 0,
                execution.all_outputs.as_ref(),
                execution.node_output_map.as_ref(),
                &execution.results,
                &build_metrics_report(execution),
                &execution.stats.node_statuses,
                None,
                provenance,
            )
            .map_err(|e| RuntimeError::State(format!("Failed to finalize child session: {e}"))),
        Err(_) => writer
            .finalize_live_with_id_and_provenance(
                false,
                Some(execution_id),
                workflow_name,
                provenance,
            )
            .map_err(|e| RuntimeError::State(format!("Failed to mark child session failed: {e}"))),
    }
}

fn provenance_from_invocation(invocation: &WorkflowInvocation) -> SessionProvenance {
    SessionProvenance {
        scope_id: invocation.parent_scope_id.clone(),
        parent_execution_id: invocation.parent_execution_id.clone(),
        parent_session_dir: invocation.parent_session_dir.clone(),
        parent_scope_id: invocation.parent_scope_id.clone(),
        spawn_node_id: invocation.spawn_node_id,
    }
}

/// Runtime metrics source for the unified `MetricsReport`.
struct RuntimeMetricsSource<'a> {
    execution: &'a RuntimeExecutionResult,
}

impl apxm_core::MetricsSource for RuntimeMetricsSource<'_> {
    fn section_name(&self) -> &'static str {
        apxm_core::constants::session::metrics_keys::SECTION_RUNTIME
    }

    fn collect(&self) -> serde_json::Value {
        use apxm_core::constants::session::metrics_keys;
        use metrics_keys::execution_keys;

        let mut map = serde_json::Map::new();
        let mut exec = serde_json::Map::new();
        exec.insert(
            execution_keys::NODES_EXECUTED.to_owned(),
            self.execution.stats.executed_nodes.into(),
        );
        exec.insert(
            execution_keys::NODES_FAILED.to_owned(),
            self.execution.stats.failed_nodes.into(),
        );
        exec.insert(
            execution_keys::DURATION_MS.to_owned(),
            (self.execution.stats.duration_ms as u64).into(),
        );
        map.insert(
            metrics_keys::RUNTIME_EXECUTION.to_owned(),
            serde_json::Value::Object(exec),
        );
        map.insert(
            metrics_keys::RUNTIME_SCHEDULER.to_owned(),
            self.execution.scheduler_metrics.to_json(),
        );
        let token_json = self.execution.token_snapshot.to_json();
        if let Some(obj) = token_json.get(metrics_keys::TOKEN_ACCOUNTING).cloned() {
            map.insert(metrics_keys::TOKEN_ACCOUNTING.to_owned(), obj);
        }
        let graph_metrics_json = self.execution.graph_metrics_snapshot.to_json();
        if let Some(obj) = graph_metrics_json
            .get(metrics_keys::RUNTIME_GRAPH_METRICS)
            .cloned()
        {
            map.insert(metrics_keys::RUNTIME_GRAPH_METRICS.to_owned(), obj);
        }
        if !self.execution.dispatch_ir_metrics.is_null() {
            map.insert(
                metrics_keys::RUNTIME_DISPATCH_IR_V1.to_owned(),
                self.execution.dispatch_ir_metrics.clone(),
            );
        }
        if let Some(observed) = &self.execution.stats.observed_graph {
            map.insert(
                metrics_keys::RUNTIME_OBSERVED_GRAPH.to_owned(),
                serde_json::to_value(observed).unwrap_or(serde_json::Value::Null),
            );
        }
        #[cfg(feature = "metrics")]
        {
            use metrics_keys::llm_keys;

            let mut llm = serde_json::Map::new();
            llm.insert(
                llm_keys::TOTAL_REQUESTS.to_owned(),
                self.execution.llm_metrics.total_requests.into(),
            );
            llm.insert(
                llm_keys::TOTAL_INPUT_TOKENS.to_owned(),
                self.execution.llm_metrics.total_input_tokens.into(),
            );
            llm.insert(
                llm_keys::TOTAL_OUTPUT_TOKENS.to_owned(),
                self.execution.llm_metrics.total_output_tokens.into(),
            );
            map.insert(
                metrics_keys::RUNTIME_LLM.to_owned(),
                serde_json::Value::Object(llm),
            );
        }
        serde_json::Value::Object(map)
    }
}

fn build_metrics_report(execution: &RuntimeExecutionResult) -> serde_json::Value {
    let mut report = apxm_core::MetricsReport::new();
    report.add_source(&RuntimeMetricsSource { execution });
    report.to_json()
}

fn primary_result_value(execution: &RuntimeExecutionResult) -> apxm_core::types::Value {
    execution
        .results
        .values()
        .find(|value| !matches!(value, apxm_core::types::Value::Null))
        .cloned()
        .unwrap_or(apxm_core::types::Value::Null)
}

fn value_to_output_string(value: &apxm_core::types::Value) -> Option<String> {
    match value {
        apxm_core::types::Value::Null => None,
        apxm_core::types::Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

fn load_graph_for_session(input: &Path) -> Result<apxm_compiler::AirModule, RuntimeError> {
    let compiler = Compiler::with_opt_level(apxm_core::types::OptimizationLevel::O0)
        .map_err(|e| RuntimeError::State(format!("Failed to initialize compiler: {e}")))?;
    let module = compiler.compile(input).map_err(|e| {
        RuntimeError::State(format!(
            "Failed to compile graph '{}': {e}",
            input.display()
        ))
    })?;
    let artifact_bytes = module
        .generate_artifact_bytes()
        .map_err(|e| RuntimeError::State(format!("Failed to emit inspection artifact: {e}")))?;
    let artifact = Artifact::from_bytes(&artifact_bytes)
        .map_err(|e| RuntimeError::State(format!("Failed to parse inspection artifact: {e}")))?;
    let dag = artifact.entry_dag().ok_or_else(|| {
        RuntimeError::State("Inspection artifact contains no entry DAG".to_string())
    })?;
    Ok(graph_from_execution_dag(dag))
}

fn graph_from_execution_dag(
    dag: &apxm_core::types::execution::ExecutionDag,
) -> apxm_compiler::AirModule {
    use apxm_compiler::{AirEdge, AirNode, AirParam};
    use std::collections::HashMap;

    let nodes = dag
        .nodes
        .iter()
        .map(|node| AirNode {
            id: node.id,
            name: node
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| format!("node_{}", node.id)),
            op: node.op_type,
            attributes: node.attributes.clone(),
        })
        .collect::<Vec<_>>();

    let edges = dag
        .edges
        .iter()
        .map(|edge| AirEdge {
            from: edge.from,
            to: edge.to,
            dependency: edge.dependency_type.clone(),
        })
        .collect::<Vec<_>>();

    let parameters = dag
        .metadata
        .parameters
        .iter()
        .map(|param| AirParam {
            name: param.name.clone(),
            type_name: param.type_name.clone(),
        })
        .collect::<Vec<_>>();

    let mut metadata = HashMap::new();
    if dag.metadata.is_entry {
        metadata.insert(
            apxm_core::constants::graph::metadata::IS_ENTRY.to_string(),
            apxm_core::types::Value::Bool(true),
        );
    }

    apxm_compiler::AirModule {
        name: dag
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "artifact".to_string()),
        nodes,
        edges,
        parameters,
        metadata,
    }
}
