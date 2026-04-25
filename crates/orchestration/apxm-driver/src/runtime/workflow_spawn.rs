use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};

use apxm_artifact::Artifact;
use apxm_core::paths::ApxmPaths;
use apxm_core::types::{SessionStatus, WorkflowInvocation, WorkflowTarget};
use apxm_runtime::{
    ExecutionEventEmitter, Runtime, RuntimeError, RuntimeExecutionResult, WorkflowSpawnResult,
    WorkflowSpawner,
};
use async_trait::async_trait;

use crate::compiler::Compiler;
use crate::hooks;
use crate::session_output::{SessionEventEmitter, SessionOutputWriter};

pub struct DriverWorkflowSpawner {
    runtime: Mutex<Option<Weak<Runtime>>>,
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
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
    ) -> Pin<Box<dyn Future<Output = Result<WorkflowSpawnResult, RuntimeError>> + Send + 'a>> {
        Box::pin(async move {
            let session_base_dir = resolve_session_base_dir(invocation.session_root.as_deref())?;
            match invocation.target {
                WorkflowTarget::GraphPath { path } => {
                    self.execute_graph_path(Path::new(&path), &invocation.args, &session_base_dir)
                        .await
                }
                WorkflowTarget::ArtifactPath { path } => {
                    self.execute_artifact_path(
                        Path::new(&path),
                        &invocation.args,
                        &session_base_dir,
                    )
                    .await
                }
                WorkflowTarget::WorkflowPath { path } => {
                    self.execute_workflow_path(Path::new(&path), invocation.args, &session_base_dir)
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

    async fn execute_graph_path(
        &self,
        graph_path: &Path,
        args: &HashMap<String, serde_json::Value>,
        session_base_dir: &Path,
    ) -> Result<WorkflowSpawnResult, RuntimeError> {
        let artifact = {
            let compiler = Compiler::new()
                .map_err(|e| RuntimeError::State(format!("Failed to initialize compiler: {e}")))?;
            let module = compiler
                .compile(graph_path)
                .map_err(|e| RuntimeError::State(format!("Failed to compile graph: {e}")))?;
            let artifact_bytes = module
                .generate_artifact_bytes()
                .map_err(|e| RuntimeError::State(format!("Failed to emit graph artifact: {e}")))?;
            Artifact::from_bytes(&artifact_bytes)
                .map_err(|e| RuntimeError::State(format!("Failed to load graph artifact: {e}")))?
        };

        let ordered_args = ordered_args_from_artifact(&artifact, args)?;
        let input_graph = load_graph_for_session(graph_path).ok();
        let execution_id = child_execution_id("graph", graph_path);
        let writer = create_session_writer(
            session_base_dir,
            &execution_id,
            graph_path.file_stem().and_then(|s| s.to_str()),
            input_graph.as_ref(),
        )?;
        let session_dir = writer.session_dir().to_path_buf();
        let runtime = self.runtime()?;
        let emitter = create_session_emitter(
            writer.session_dir(),
            &execution_id,
            input_graph.as_ref(),
            artifact
                .entry_dag()
                .or_else(|| artifact.dag())
                .map(|dag| dag.nodes.len()),
            runtime.memory_system_arc(),
            self.configured_emitter.as_ref().map(Arc::clone),
        )?;

        let execution = runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                ordered_args,
                None,
                emitter,
                Some(session_dir.to_string_lossy().to_string()),
            )
            .await;

        finalize_child_session(
            &writer,
            &execution_id,
            graph_path.file_stem().and_then(|s| s.to_str()),
            execution.as_ref(),
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
    ) -> Result<WorkflowSpawnResult, RuntimeError> {
        let artifact = Artifact::read_from_path(artifact_path).map_err(|e| {
            RuntimeError::State(format!(
                "Failed to read artifact '{}': {e}",
                artifact_path.display()
            ))
        })?;
        let ordered_args = ordered_args_from_artifact(&artifact, args)?;
        let execution_id = child_execution_id("artifact", artifact_path);
        let writer = create_session_writer(
            session_base_dir,
            &execution_id,
            artifact_path.file_stem().and_then(|s| s.to_str()),
            None,
        )?;
        let session_dir = writer.session_dir().to_path_buf();
        let runtime = self.runtime()?;
        let emitter = create_session_emitter(
            writer.session_dir(),
            &execution_id,
            None,
            artifact
                .entry_dag()
                .or_else(|| artifact.dag())
                .map(|dag| dag.nodes.len()),
            runtime.memory_system_arc(),
            self.configured_emitter.as_ref().map(Arc::clone),
        )?;

        let execution = runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                ordered_args,
                None,
                emitter,
                Some(session_dir.to_string_lossy().to_string()),
            )
            .await;

        finalize_child_session(
            &writer,
            &execution_id,
            artifact_path.file_stem().and_then(|s| s.to_str()),
            execution.as_ref(),
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
        let mut step_outputs: HashMap<String, String> = HashMap::new();
        let mut step_results = HashMap::new();

        for phase in execution_phases(&def.graphs)
            .map_err(|e| RuntimeError::State(format!("Workflow planning failed: {e}")))?
        {
            for step_id in phase {
                let step = def
                    .graphs
                    .iter()
                    .find(|candidate| candidate.id == step_id)
                    .ok_or_else(|| {
                        RuntimeError::State(format!("Unknown workflow step '{step_id}'"))
                    })?;

                let should_skip = step.depends_on.iter().any(|dep| {
                    step_results
                        .get(dep)
                        .map(|result: &apxm_runtime::workflow::StepResult| {
                            result.status != apxm_runtime::workflow::StepStatus::Success
                        })
                        .unwrap_or(false)
                });
                if should_skip {
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

                match self.execute_invocation(child_invocation).await {
                    Ok(child_result) => {
                        let output = value_to_output_string(&child_result.value);
                        if let Some(ref text) = output {
                            step_outputs.insert(step.id.clone(), text.clone());
                        }
                        step_results.insert(
                            step.id.clone(),
                            apxm_runtime::workflow::StepResult {
                                id: step.id.clone(),
                                status: apxm_runtime::workflow::StepStatus::Success,
                                output,
                                duration_ms: 0,
                                session_dir: child_result.session_dir.map(PathBuf::from),
                                error: None,
                            },
                        );
                    }
                    Err(error) => {
                        step_results.insert(
                            step.id.clone(),
                            apxm_runtime::workflow::StepResult {
                                id: step.id.clone(),
                                status: apxm_runtime::workflow::StepStatus::Failed,
                                output: None,
                                duration_ms: 0,
                                session_dir: Some(workflow_session_dir.clone()),
                                error: Some(error.to_string()),
                            },
                        );
                    }
                }
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
            duration_ms: 0,
        };

        match workflow_result.status {
            apxm_runtime::workflow::WorkflowStatus::Success => Ok(WorkflowSpawnResult {
                value: workflow_result
                    .output
                    .map(apxm_core::types::Value::String)
                    .unwrap_or(apxm_core::types::Value::Null),
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
    ) -> Result<WorkflowSpawnResult, RuntimeError> {
        self.execute_invocation(invocation).await
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
        .or_else(|| artifact.dag())
        .ok_or_else(|| RuntimeError::State("Artifact contains no DAGs".to_string()))?;

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
    graph_name: Option<&str>,
    input_graph: Option<&apxm_compiler::AirModule>,
) -> Result<SessionOutputWriter, RuntimeError> {
    let writer = SessionOutputWriter::new(session_base_dir, execution_id).map_err(|e| {
        RuntimeError::State(format!(
            "Failed to create child session directory '{}': {e}",
            session_base_dir.display()
        ))
    })?;
    writer
        .write_manifest(
            execution_id,
            graph_name,
            SessionStatus::Running,
            0,
            input_graph
                .map(|graph| graph.nodes.len())
                .unwrap_or_default(),
            false,
        )
        .map_err(|e| RuntimeError::State(format!("Failed to write child session manifest: {e}")))?;
    if let Some(graph) = input_graph {
        writer
            .write_input_graph(graph)
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
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
) -> Result<Option<Arc<dyn ExecutionEventEmitter>>, RuntimeError> {
    let project_root = std::env::current_dir().ok();
    let emitter = Arc::new(
        SessionEventEmitter::new(
            session_dir,
            execution_id.to_string(),
            input_graph,
            project_root.as_deref(),
        )
        .map_err(|e| RuntimeError::State(format!("Failed to create session emitter: {e}")))?,
    );
    if let Some(total_nodes) = total_nodes {
        emitter.set_total_nodes(total_nodes as u64);
    }
    emitter.set_memory(memory);
    Ok(hooks::compose_emitters(
        Some(emitter as Arc<dyn ExecutionEventEmitter>),
        configured_emitter,
    ))
}

fn finalize_child_session(
    writer: &SessionOutputWriter,
    execution_id: &str,
    graph_name: Option<&str>,
    execution: Result<&RuntimeExecutionResult, &RuntimeError>,
) -> Result<(), RuntimeError> {
    match execution {
        Ok(execution) => writer
            .finalize(
                execution_id,
                graph_name,
                execution.stats.duration_ms as u128,
                execution.stats.executed_nodes + execution.stats.failed_nodes,
                execution.stats.failed_nodes == 0,
                execution.all_outputs.as_ref(),
                execution.node_output_map.as_ref(),
                &execution.results,
                &build_metrics_report(execution),
                &execution.stats.node_statuses,
                None,
            )
            .map_err(|e| RuntimeError::State(format!("Failed to finalize child session: {e}"))),
        Err(_) => writer
            .finalize_live_with_id(false, Some(execution_id), graph_name)
            .map_err(|e| RuntimeError::State(format!("Failed to mark child session failed: {e}"))),
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
        use metrics_keys::{execution_keys, llm_keys};

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
        #[cfg(feature = "metrics")]
        {
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
    if let Ok(compiler) = Compiler::new()
        && let Ok(graph) = compiler.load_graph(input)
    {
        return Ok(graph);
    }

    let text = std::fs::read_to_string(input).map_err(|e| {
        RuntimeError::State(format!("Failed to read graph '{}': {e}", input.display()))
    })?;
    serde_json::from_str::<apxm_compiler::AirModule>(&text).map_err(|e| {
        RuntimeError::State(format!("Failed to parse graph '{}': {e}", input.display()))
    })
}
