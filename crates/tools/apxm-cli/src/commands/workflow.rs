//! Workflow-file execution commands.

use anyhow::{Context, Result};
#[cfg(feature = "driver")]
use apxm_artifact::Artifact;
#[cfg(feature = "driver")]
use apxm_driver::{Linker, LinkerConfig};
use colored::Colorize;

use super::cli::*;
#[cfg(feature = "driver")]
use super::compile::air_graph_from_source;
#[cfg(not(feature = "driver"))]
use super::dekk_hints;
#[cfg(feature = "driver")]
use std::collections::HashMap;
#[cfg(feature = "driver")]
use std::fs::OpenOptions;
#[cfg(feature = "driver")]
use std::future::Future;
#[cfg(feature = "driver")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(feature = "driver")]
use std::pin::Pin;
#[cfg(feature = "driver")]
use std::process::{Command, Stdio};
#[cfg(feature = "driver")]
use std::sync::Arc;
#[cfg(feature = "driver")]
use std::time::Instant;

#[cfg(feature = "driver")]
use super::implementations::load_config;

#[cfg(feature = "driver")]
struct CliWorkflowStepOutcome {
    step_id: String,
    step_index: usize,
    result: apxm_runtime::workflow::StepResult,
}

#[cfg(feature = "driver")]
pub async fn workflow_command(
    action: WorkflowAction,
    config: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    match action {
        WorkflowAction::Run {
            file,
            background,
            args_json,
            args,
            session_root,
            session_dir,
        } => {
            workflow_run_command(
                file,
                background,
                args_json,
                args,
                session_root,
                session_dir,
                config,
                json,
            )
            .await
        }
        WorkflowAction::Validate { file } => workflow_validate_command(file, json),
        WorkflowAction::Analyze { file } => workflow_analyze_command(file, json),
    }
}

#[cfg(not(feature = "driver"))]
pub fn workflow_command_no_driver(action: WorkflowAction, json: bool) -> Result<()> {
    match action {
        WorkflowAction::Validate { file } => workflow_validate_command(file, json),
        WorkflowAction::Analyze { file } => workflow_analyze_command(file, json),
        _ => Err(anyhow::anyhow!(
            "Workflow execution requires the `driver` feature. Rebuild through `{}`, then re-run `{}`.",
            dekk_hints::BUILD,
            dekk_hints::WORKFLOW_RUN
        )),
    }
}

pub fn workflow_validate_command(file: PathBuf, json: bool) -> Result<()> {
    use apxm_runtime::workflow::WorkflowDef;

    let def = WorkflowDef::from_file(&file)
        .with_context(|| format!("Failed to load workflow file {}", file.display()))?;

    let errors = def.validate();

    if json {
        let output = serde_json::json!({
            "valid": errors.is_empty(),
            "errors": errors,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if errors.is_empty() {
            println!(
                "{} Workflow is valid",
                apxm_core::constants::ui::icons::SUCCESS
            );
            println!("  Name: {}", def.name);
            println!("  Steps: {}", def.graphs.len());
            println!(
                "  Parameters: {}",
                def.parameters
                    .iter()
                    .map(|p| format!("{}: {}", p.name, p.type_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        } else {
            println!(
                "{} Validation failed:",
                apxm_core::constants::ui::icons::FAILED
            );
            for error in &errors {
                println!("  - {}", error);
            }
            return Err(anyhow::anyhow!("Validation failed"));
        }
    }

    Ok(())
}

pub fn workflow_analyze_command(file: PathBuf, json: bool) -> Result<()> {
    use apxm_runtime::workflow::{WorkflowDef, execution_phases};

    let def = WorkflowDef::from_file(&file)
        .with_context(|| format!("Failed to load workflow file {}", file.display()))?;

    let errors = def.validate();
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(
            "Workflow validation failed: {}",
            errors.join(", ")
        ));
    }

    let phases = execution_phases(&def.graphs)?;

    if json {
        let output = serde_json::json!({
            "name": def.name,
            "total_steps": def.graphs.len(),
            "phases": phases.len(),
            "max_parallelism": phases.iter().map(|p| p.len()).max().unwrap_or(0),
            "execution_plan": phases,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Workflow: {}", def.name.bold());
        println!();
        println!("  Total steps: {}", def.graphs.len());
        println!("  Execution phases: {}", phases.len());
        println!(
            "  Max parallelism: {}",
            phases.iter().map(|p| p.len()).max().unwrap_or(0)
        );
        println!();
        println!("Execution plan:");
        for (i, phase) in phases.iter().enumerate() {
            println!("  Phase {}: {} step(s) in parallel", i, phase.len());
            for step_id in phase {
                let step = def.graphs.iter().find(|s| &s.id == step_id).unwrap();
                println!("    - {} ({})", step_id, step.path);
            }
        }
    }

    Ok(())
}

#[cfg(feature = "driver")]
pub async fn workflow_run_command(
    file: PathBuf,
    background: bool,
    args_json: Option<String>,
    args: Vec<String>,
    session_root: Option<PathBuf>,
    session_dir: Option<PathBuf>,
    config: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    if background {
        return workflow_run_background_command(file, args_json, args, session_root, config, json)
            .await;
    }

    let params = parse_workflow_args(args_json.as_deref(), &args)?;
    let config = load_config(config)?;
    let linker = Linker::new(LinkerConfig::from_apxm_config(config)).await?;
    let session_base_dir = if let Some(exact_session_dir) = session_dir.as_deref() {
        exact_session_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        resolve_workflow_sessions_dir(session_root.as_deref())?
    };

    let (result, session_dir) = execute_workflow_file(
        &file,
        params,
        &session_base_dir,
        session_dir.as_deref(),
        &linker,
        0,
        !json,
    )
    .await?;

    if json {
        let output = serde_json::json!({
            "workflow_name": result.workflow_name,
            "status": format!("{:?}", result.status),
            "duration_ms": result.duration_ms,
            "session_dir": session_dir,
            "step_results": result.step_results,
            "output": result.output,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("Session directory: {}", session_dir.display());
    println!();
    println!(
        "Workflow completed in {:.1}s",
        result.duration_ms as f64 / 1000.0
    );
    println!();
    println!("Results:");
    for (step_id, result) in &result.step_results {
        let status_icon = match result.status {
            apxm_runtime::workflow::StepStatus::Success => apxm_core::constants::ui::icons::SUCCESS,
            apxm_runtime::workflow::StepStatus::Failed => apxm_core::constants::ui::icons::FAILED,
            apxm_runtime::workflow::StepStatus::Skipped => apxm_core::constants::ui::icons::WARNING,
        };
        println!(
            "  {} {}: {:?} ({:.1}s)",
            status_icon,
            step_id,
            result.status,
            result.duration_ms as f64 / 1000.0
        );
    }

    if let Some(output) = result.output {
        println!();
        println!("Final output:");
        println!("{}", output);
    }

    Ok(())
}

#[cfg(feature = "driver")]
async fn workflow_run_background_command(
    file: PathBuf,
    args_json: Option<String>,
    args: Vec<String>,
    session_root: Option<PathBuf>,
    config: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    use apxm_runtime::workflow::WorkflowDef;

    let params = parse_workflow_args(args_json.as_deref(), &args)?;
    let def = WorkflowDef::from_file(&file)
        .with_context(|| format!("Failed to load workflow file {}", file.display()))?;
    let errors = def.validate();
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(
            "Workflow validation failed: {}",
            errors.join(", ")
        ));
    }
    validate_workflow_params(&def, &params)?;

    let session_base_dir = resolve_workflow_sessions_dir(session_root.as_deref())?;
    let session_dir = create_workflow_session_dir(&session_base_dir, &def.name)?;
    apxm_runtime::workflow::write_workflow_session_started(
        &session_dir,
        &def.name,
        def.graphs.len(),
    )?;

    let log_file = session_dir.join("background.log");
    let child_args = build_background_workflow_args(
        &file,
        args_json.as_deref(),
        &args,
        &session_dir,
        config.as_deref(),
        json,
    );
    let exe = std::env::current_exe().context("Failed to resolve current APXM executable")?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .with_context(|| format!("Failed to open background log {}", log_file.display()))?;
    let log_for_stderr = log
        .try_clone()
        .with_context(|| format!("Failed to clone background log {}", log_file.display()))?;
    let mut command = background_command(&exe, &child_args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_for_stderr));
    let child = command
        .spawn()
        .with_context(|| format!("Failed to spawn background workflow {}", file.display()))?;
    let pid = child.id();

    let command_line = background_command_line(&exe, &child_args);
    apxm_runtime::workflow::write_workflow_background_started(
        &session_dir,
        pid,
        &log_file,
        &command_line,
    )?;

    if json {
        let output = serde_json::json!({
            "status": "background",
            "pid": pid,
            "workflow_name": def.name,
            "session_dir": session_dir,
            "log_file": log_file,
            "command": command_line,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("Workflow started in background");
    println!("  PID: {pid}");
    println!("  Session directory: {}", session_dir.display());
    println!("  Log file: {}", log_file.display());
    println!("  Follow: apxm session inspect {}", session_dir.display());
    Ok(())
}

#[cfg(feature = "driver")]
fn build_background_workflow_args(
    file: &Path,
    args_json: Option<&str>,
    args: &[String],
    session_dir: &Path,
    config: Option<&Path>,
    json: bool,
) -> Vec<String> {
    let mut child_args = vec!["workflow".to_string(), "run".to_string()];
    if json {
        child_args.push("--json".to_string());
    }
    if let Some(config) = config {
        child_args.push("--config".to_string());
        child_args.push(config.to_string_lossy().to_string());
    }
    child_args.push("--session-dir".to_string());
    child_args.push(session_dir.to_string_lossy().to_string());
    if let Some(args_json) = args_json {
        child_args.push("--args-json".to_string());
        child_args.push(args_json.to_string());
    }
    child_args.push(file.to_string_lossy().to_string());
    child_args.extend(args.iter().cloned());
    child_args
}

#[cfg(feature = "driver")]
#[cfg(unix)]
fn background_command(exe: &Path, child_args: &[String]) -> Command {
    // Use the platform `setsid` helper instead of unsafe pre_exec hooks. This
    // keeps background workflows alive after PTY-based callers return.
    let mut command = Command::new("setsid");
    command.arg(exe).args(child_args);
    command
}

#[cfg(feature = "driver")]
#[cfg(not(unix))]
fn background_command(exe: &Path, child_args: &[String]) -> Command {
    let mut command = Command::new(exe);
    command.args(child_args);
    command
}

#[cfg(feature = "driver")]
#[cfg(unix)]
fn background_command_line(exe: &Path, child_args: &[String]) -> Vec<String> {
    std::iter::once("setsid".to_string())
        .chain(std::iter::once(exe.to_string_lossy().to_string()))
        .chain(child_args.iter().cloned())
        .collect()
}

#[cfg(feature = "driver")]
#[cfg(not(unix))]
fn background_command_line(exe: &Path, child_args: &[String]) -> Vec<String> {
    std::iter::once(exe.to_string_lossy().to_string())
        .chain(child_args.iter().cloned())
        .collect()
}

#[cfg(feature = "driver")]
type WorkflowRunFuture<'a> = Pin<
    Box<dyn Future<Output = Result<(apxm_runtime::workflow::WorkflowResult, PathBuf)>> + Send + 'a>,
>;

#[cfg(feature = "driver")]
fn execute_workflow_file<'a>(
    file: &'a Path,
    params: HashMap<String, String>,
    session_base_dir: &'a Path,
    explicit_session_dir: Option<&'a Path>,
    linker: &'a Linker,
    depth: usize,
    render_progress: bool,
) -> WorkflowRunFuture<'a> {
    Box::pin(async move {
        use apxm_runtime::workflow::{WorkflowDef, execution_phases};

        let def = WorkflowDef::from_file(file)
            .with_context(|| format!("Failed to load workflow file {}", file.display()))?;
        let errors = def.validate();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Workflow validation failed: {}",
                errors.join(", ")
            ));
        }
        validate_workflow_params(&def, &params)?;

        let indent = "  ".repeat(depth);
        if render_progress {
            println!("{indent}Executing workflow: {}", def.name.bold());
            println!();
        }

        let base_dir = file
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Failed to get parent directory"))?
            .to_path_buf();
        let phases = execution_phases(&def.graphs)?;
        let workflow_session_dir = if let Some(session_dir) = explicit_session_dir {
            std::fs::create_dir_all(session_dir)?;
            session_dir.to_path_buf()
        } else {
            create_workflow_session_dir(session_base_dir, &def.name)?
        };
        apxm_runtime::workflow::write_workflow_session_started(
            &workflow_session_dir,
            &def.name,
            def.graphs.len(),
        )?;

        let start = Instant::now();
        let mut step_outputs: HashMap<String, String> = HashMap::new();
        let mut step_results: HashMap<String, apxm_runtime::workflow::StepResult> = HashMap::new();

        for (phase_idx, phase) in phases.iter().enumerate() {
            if render_progress {
                println!("{indent}Phase {phase_idx}: {} step(s)", phase.len());
            }

            let mut phase_jobs = Vec::new();
            for step_id in phase {
                let step_index = def
                    .graphs
                    .iter()
                    .position(|s| &s.id == step_id)
                    .unwrap_or(step_results.len());
                let step = def
                    .graphs
                    .iter()
                    .find(|s| &s.id == step_id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown step id '{}'", step_id))?;

                let should_skip = step.depends_on.iter().any(|dep| {
                    step_results.get(dep).map_or(false, |r| {
                        r.status != apxm_runtime::workflow::StepStatus::Success
                    })
                });

                if should_skip {
                    if render_progress {
                        println!("{indent}  {step_id} Skipping (failed dependency)");
                    }
                    apxm_runtime::workflow::write_workflow_step_finished(
                        &workflow_session_dir,
                        &def.name,
                        step_id,
                        step_index,
                        apxm_runtime::workflow::StepStatus::Skipped,
                        0,
                        step_results.len() + 1,
                        def.graphs.len(),
                        start.elapsed().as_millis(),
                    )?;
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

                let resolved_params: HashMap<String, String> = step
                    .params
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            apxm_runtime::workflow::resolve(v, &step_outputs, &params),
                        )
                    })
                    .collect();
                let step_path = base_dir.join(&step.path);
                let step_session_dir = workflow_session_dir.join(&step.id);
                std::fs::create_dir_all(&step_session_dir)?;

                if render_progress {
                    println!(
                        "{indent}  {} Starting: {} ({})",
                        apxm_core::constants::ui::icons::STARTED,
                        step_id,
                        step_path.display()
                    );
                }
                apxm_runtime::workflow::write_workflow_step_started(
                    &workflow_session_dir,
                    &def.name,
                    step_id,
                    step_index,
                    step_results.len(),
                    def.graphs.len(),
                    start.elapsed().as_millis(),
                )?;

                let step_id = step_id.clone();
                let workflow_session_dir = workflow_session_dir.clone();
                phase_jobs.push(async move {
                    let step_start = Instant::now();
                    let result = match step_path.extension().and_then(|ext| ext.to_str()) {
                        Some("apxmw") => {
                            let (child_result, child_session_dir) = execute_workflow_file(
                                &step_path,
                                resolved_params,
                                &workflow_session_dir,
                                None,
                                linker,
                                depth + 1,
                                render_progress,
                            )
                            .await?;
                            workflow_step_result_from_nested(
                                step_id.clone(),
                                child_result,
                                child_session_dir,
                                step_start.elapsed().as_millis() as u64,
                            )
                        }
                        Some("apxmobj") => {
                            execute_artifact_step(
                                linker,
                                &step_id,
                                &step_path,
                                &resolved_params,
                                &step_session_dir,
                                step_start,
                            )
                            .await?
                        }
                        _ => {
                            execute_graph_step(
                                linker,
                                &step_id,
                                &step_path,
                                &resolved_params,
                                &step_session_dir,
                                step_start,
                            )
                            .await?
                        }
                    };
                    Ok::<_, anyhow::Error>(CliWorkflowStepOutcome {
                        step_id,
                        step_index,
                        result,
                    })
                });
            }

            let mut phase_outputs = Vec::new();
            for completed in futures::future::join_all(phase_jobs).await {
                let completed = completed?;
                if let Some(ref out) = completed.result.output
                    && completed.result.status == apxm_runtime::workflow::StepStatus::Success
                {
                    phase_outputs.push((completed.step_id.clone(), out.clone()));
                }

                let status_icon = match completed.result.status {
                    apxm_runtime::workflow::StepStatus::Success => {
                        apxm_core::constants::ui::icons::SUCCESS
                    }
                    apxm_runtime::workflow::StepStatus::Failed => {
                        apxm_core::constants::ui::icons::FAILED
                    }
                    apxm_runtime::workflow::StepStatus::Skipped => {
                        apxm_core::constants::ui::icons::WARNING
                    }
                };
                if render_progress {
                    println!(
                        "{indent}  {} {} ({:.1}s)",
                        status_icon,
                        completed.step_id,
                        completed.result.duration_ms as f64 / 1000.0
                    );
                }
                apxm_runtime::workflow::write_workflow_step_finished(
                    &workflow_session_dir,
                    &def.name,
                    &completed.step_id,
                    completed.step_index,
                    completed.result.status,
                    completed.result.duration_ms,
                    step_results.len() + 1,
                    def.graphs.len(),
                    start.elapsed().as_millis(),
                )?;

                step_results.insert(completed.step_id, completed.result);
            }

            for (step_id, output) in phase_outputs {
                step_outputs.insert(step_id, output);
            }

            if render_progress {
                println!();
            }
        }

        let output = def
            .output
            .as_ref()
            .map(|tmpl| apxm_runtime::workflow::resolve(tmpl, &step_outputs, &params));
        let status = workflow_status_from_steps(&step_results);

        let result = apxm_runtime::workflow::WorkflowResult {
            workflow_name: def.name,
            status,
            step_results,
            output,
            duration_ms: start.elapsed().as_millis() as u64,
        };
        apxm_runtime::workflow::write_workflow_session_finished(&workflow_session_dir, &result)?;

        Ok((result, workflow_session_dir))
    })
}

#[cfg(feature = "driver")]
fn parse_workflow_args(
    args_json: Option<&str>,
    args: &[String],
) -> Result<HashMap<String, String>> {
    if let Some(raw_json) = args_json {
        let parsed: HashMap<String, serde_json::Value> = serde_json::from_str(raw_json)
            .with_context(|| "Failed to parse --args-json as a JSON object")?;
        return Ok(parsed
            .into_iter()
            .map(|(key, value)| {
                let normalized = match value {
                    serde_json::Value::String(text) => text,
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                };
                (key, normalized)
            })
            .collect());
    }

    let mut params = HashMap::new();
    for arg in args {
        if let Some((key, value)) = arg.split_once('=') {
            params.insert(key.to_string(), value.to_string());
        } else {
            return Err(anyhow::anyhow!(
                "Invalid argument format '{}'. Expected name=value",
                arg
            ));
        }
    }
    Ok(params)
}

#[cfg(all(test, feature = "driver"))]
mod tests {
    #[cfg(unix)]
    use super::background_command_line;
    use super::parse_workflow_args;
    #[cfg(unix)]
    use std::path::Path;

    #[test]
    fn parse_workflow_args_accepts_json_object() {
        let params = parse_workflow_args(
            Some(r#"{"topic":"middleware","limit":3,"enabled":true,"empty":null}"#),
            &[],
        )
        .expect("parse args json");
        assert_eq!(params.get("topic").map(String::as_str), Some("middleware"));
        assert_eq!(params.get("limit").map(String::as_str), Some("3"));
        assert_eq!(params.get("enabled").map(String::as_str), Some("true"));
        assert_eq!(params.get("empty").map(String::as_str), Some(""));
    }

    #[test]
    fn parse_workflow_args_rejects_invalid_pairs() {
        let error = parse_workflow_args(None, &[String::from("missing_delimiter")])
            .expect_err("invalid args must fail");
        assert!(error.to_string().contains("Expected name=value"));
    }

    #[cfg(unix)]
    #[test]
    fn background_command_line_uses_setsid_on_unix() {
        let command_line = background_command_line(
            Path::new("/tmp/apxm"),
            &["workflow".to_string(), "run".to_string()],
        );
        assert_eq!(command_line[0], "setsid");
        assert_eq!(command_line[1], "/tmp/apxm");
    }
}

#[cfg(feature = "driver")]
fn validate_workflow_params(
    def: &apxm_runtime::workflow::WorkflowDef,
    params: &HashMap<String, String>,
) -> Result<()> {
    for param in &def.parameters {
        if !params.contains_key(&param.name) {
            return Err(anyhow::anyhow!(
                "Missing required parameter: {} (type: {})",
                param.name,
                param.type_name
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "driver")]
fn resolve_workflow_sessions_dir(explicit_root: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit_root {
        std::fs::create_dir_all(path)?;
        return Ok(path.to_path_buf());
    }
    Ok(apxm_core::paths::ApxmPaths::discover()?.sessions_dir()?)
}

#[cfg(feature = "driver")]
fn create_workflow_session_dir(session_base_dir: &Path, workflow_name: &str) -> Result<PathBuf> {
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S-%6f");
    let mut dir = session_base_dir.join(format!("workflow-{workflow_name}-{timestamp}"));
    let mut attempt = 1_u32;
    while dir.exists() {
        dir = session_base_dir.join(format!("workflow-{workflow_name}-{timestamp}-{attempt}"));
        attempt += 1;
    }
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(feature = "driver")]
fn execution_output(result: &apxm_runtime::RuntimeExecutionResult) -> Option<String> {
    result.results.values().last().map(|value| match value {
        apxm_core::types::Value::String(s) => s.clone(),
        _ => value.to_string(),
    })
}

#[cfg(feature = "driver")]
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

#[cfg(feature = "driver")]
fn workflow_step_result_from_nested(
    step_id: String,
    child_result: apxm_runtime::workflow::WorkflowResult,
    child_session_dir: PathBuf,
    duration_ms: u64,
) -> apxm_runtime::workflow::StepResult {
    let status = match child_result.status {
        apxm_runtime::workflow::WorkflowStatus::Success => {
            apxm_runtime::workflow::StepStatus::Success
        }
        apxm_runtime::workflow::WorkflowStatus::PartialFailure
        | apxm_runtime::workflow::WorkflowStatus::Failed => {
            apxm_runtime::workflow::StepStatus::Failed
        }
    };
    apxm_runtime::workflow::StepResult {
        id: step_id,
        status,
        output: child_result.output,
        duration_ms,
        session_dir: Some(child_session_dir),
        error: None,
    }
}

#[cfg(feature = "driver")]
async fn execute_graph_step(
    linker: &Linker,
    step_id: &str,
    graph_path: &Path,
    resolved_params: &HashMap<String, String>,
    step_session_dir: &Path,
    step_start: Instant,
) -> Result<apxm_runtime::workflow::StepResult> {
    let graph_args = load_graph_step_args(graph_path, resolved_params)?;
    let input_graph = load_graph_for_session(graph_path).ok();
    let (writer, emitter, ticker_handle) = create_step_session_capture(
        linker,
        step_session_dir.parent().unwrap_or(step_session_dir),
        step_id,
        input_graph.as_ref(),
    )?;
    let result = linker
        .run_graph(
            graph_path,
            graph_args,
            Some(emitter.clone() as Arc<dyn apxm_runtime::ExecutionEventEmitter>),
            Some(writer.session_dir()),
        )
        .await;
    let duration_ms = step_start.elapsed().as_millis() as u64;

    match result {
        Ok(link_result) => {
            finish_step_session(
                linker,
                &writer,
                &emitter,
                step_id,
                graph_path.file_stem().and_then(|s| s.to_str()),
                duration_ms,
                &link_result.execution,
                Some(
                    link_result.execution.stats.executed_nodes
                        + link_result.execution.stats.failed_nodes,
                ),
                build_graph_metrics_json(graph_path, &link_result),
                ticker_handle,
            )
            .await?;
            Ok(apxm_runtime::workflow::StepResult {
                id: step_id.to_string(),
                status: apxm_runtime::workflow::StepStatus::Success,
                output: execution_output(&link_result.execution),
                duration_ms,
                session_dir: Some(writer.session_dir().to_path_buf()),
                error: None,
            })
        }
        Err(error) => {
            fail_step_session(
                &writer,
                &emitter,
                step_id,
                graph_path.file_stem().and_then(|s| s.to_str()),
                ticker_handle,
            )?;
            Ok(apxm_runtime::workflow::StepResult {
                id: step_id.to_string(),
                status: apxm_runtime::workflow::StepStatus::Failed,
                output: None,
                duration_ms,
                session_dir: Some(writer.session_dir().to_path_buf()),
                error: Some(error.to_string()),
            })
        }
    }
}

#[cfg(feature = "driver")]
async fn execute_artifact_step(
    linker: &Linker,
    step_id: &str,
    artifact_path: &Path,
    resolved_params: &HashMap<String, String>,
    step_session_dir: &Path,
    step_start: Instant,
) -> Result<apxm_runtime::workflow::StepResult> {
    let artifact = Artifact::read_from_path(artifact_path)
        .with_context(|| format!("Failed to read artifact {}", artifact_path.display()))?;
    let artifact_args = load_artifact_step_args(&artifact, resolved_params)?;
    let (writer, emitter, ticker_handle) = create_step_session_capture(
        linker,
        step_session_dir.parent().unwrap_or(step_session_dir),
        step_id,
        None,
    )?;
    let result = linker
        .runtime_executor()
        .execute_artifact_with_emitter(
            artifact,
            artifact_args,
            Some(emitter.clone() as Arc<dyn apxm_runtime::ExecutionEventEmitter>),
            Some(writer.session_dir().to_string_lossy().to_string()),
        )
        .await;
    let duration_ms = step_start.elapsed().as_millis() as u64;

    match result {
        Ok(execution) => {
            finish_step_session(
                linker,
                &writer,
                &emitter,
                step_id,
                artifact_path.file_stem().and_then(|s| s.to_str()),
                duration_ms,
                &execution,
                Some(execution.stats.executed_nodes + execution.stats.failed_nodes),
                build_artifact_metrics_json(artifact_path, &execution),
                ticker_handle,
            )
            .await?;
            Ok(apxm_runtime::workflow::StepResult {
                id: step_id.to_string(),
                status: apxm_runtime::workflow::StepStatus::Success,
                output: execution_output(&execution),
                duration_ms,
                session_dir: Some(writer.session_dir().to_path_buf()),
                error: None,
            })
        }
        Err(error) => {
            fail_step_session(
                &writer,
                &emitter,
                step_id,
                artifact_path.file_stem().and_then(|s| s.to_str()),
                ticker_handle,
            )?;
            Ok(apxm_runtime::workflow::StepResult {
                id: step_id.to_string(),
                status: apxm_runtime::workflow::StepStatus::Failed,
                output: None,
                duration_ms,
                session_dir: Some(writer.session_dir().to_path_buf()),
                error: Some(error.to_string()),
            })
        }
    }
}

#[cfg(feature = "driver")]
fn load_graph_for_session(input: &Path) -> Result<apxm_compiler::AirModule> {
    air_graph_from_source(input)
}

#[cfg(feature = "driver")]
fn create_step_session_capture(
    linker: &Linker,
    session_base_dir: &Path,
    execution_id: &str,
    input_graph: Option<&apxm_compiler::AirModule>,
) -> Result<(
    apxm_driver::session_output::SessionOutputWriter,
    Arc<apxm_driver::session_output::SessionEventEmitter>,
    tokio::task::JoinHandle<()>,
)> {
    use apxm_driver::session_output::{SessionEventEmitter, SessionOutputWriter};

    let writer = SessionOutputWriter::new(session_base_dir, execution_id)
        .context("Failed to create step session output directory")?;
    writer
        .write_manifest(
            execution_id,
            input_graph.map(|graph| graph.name.as_str()),
            apxm_core::types::SessionStatus::Running,
            0,
            input_graph.map_or(0, |graph| graph.nodes.len()),
            false,
        )
        .context("Failed to write step manifest")?;
    if let Some(graph) = input_graph {
        writer
            .write_input_graph(graph)
            .context("Failed to write step input graph")?;
    }

    let project_root = std::env::current_dir().ok();
    let emitter = Arc::new(
        SessionEventEmitter::new(
            writer.session_dir(),
            execution_id.to_string(),
            input_graph,
            project_root.as_deref(),
        )
        .context("Failed to create step session event emitter")?,
    );
    if let Some(graph) = input_graph {
        emitter.set_total_nodes(graph.nodes.len() as u64);
    }
    emitter.set_memory(linker.runtime_executor().memory_system());

    let ticker_emitter = Arc::clone(&emitter);
    let ticker_handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            ticker_emitter.tick();
        }
    });

    Ok((writer, emitter, ticker_handle))
}

#[cfg(feature = "driver")]
fn build_graph_metrics_json(
    graph_path: &Path,
    result: &apxm_driver::LinkResult,
) -> serde_json::Value {
    let mut metrics_json = serde_json::json!({
        "input": graph_path.display().to_string(),
        "execution": {
            "nodes_executed": result.execution.stats.executed_nodes,
            "nodes_failed": result.execution.stats.failed_nodes,
            "duration_ms": result.execution.stats.duration_ms,
            "status": if result.execution.stats.failed_nodes == 0 { "success" } else { "partial_failure" }
        },
        "scheduler": result.execution.scheduler_metrics.to_json()
    });
    let token_json = result.execution.token_snapshot.to_json();
    use apxm_core::constants::session::metrics_keys;
    if let Some(obj) = token_json.get(metrics_keys::TOKEN_ACCOUNTING).cloned() {
        metrics_json[metrics_keys::TOKEN_ACCOUNTING] = obj;
    }
    let graph_metrics_json = result.execution.graph_metrics_snapshot.to_json();
    if let Some(obj) = graph_metrics_json
        .get(metrics_keys::RUNTIME_GRAPH_METRICS)
        .cloned()
    {
        metrics_json[metrics_keys::RUNTIME_GRAPH_METRICS] = obj;
    }
    if let Some(observed) = &result.execution.stats.observed_graph {
        metrics_json[metrics_keys::RUNTIME_OBSERVED_GRAPH] =
            serde_json::to_value(observed).unwrap_or(serde_json::Value::Null);
    }
    metrics_json[metrics_keys::RUNTIME_NODE_STATUSES] =
        serde_json::to_value(&result.execution.stats.node_statuses)
            .unwrap_or(serde_json::Value::Null);
    #[cfg(feature = "metrics")]
    {
        let llm_metrics = &result.execution.llm_metrics;
        metrics_json["llm"] = serde_json::json!({
            "total_requests": llm_metrics.total_requests,
            "total_input_tokens": llm_metrics.total_input_tokens,
            "total_output_tokens": llm_metrics.total_output_tokens,
            "avg_latency_ms": llm_metrics.average_latency.as_millis(),
            "p50_latency_ms": llm_metrics.p50_latency.as_millis(),
            "p99_latency_ms": llm_metrics.p99_latency.as_millis()
        });
        metrics_json["link_phases"] = serde_json::json!({
            "compile_ms": result.metrics.compile_time.as_secs_f64() * 1000.0,
            "runtime_ms": result.metrics.runtime_time.as_secs_f64() * 1000.0
        });
    }
    metrics_json
}

#[cfg(feature = "driver")]
fn build_artifact_metrics_json(
    artifact_path: &Path,
    execution: &apxm_runtime::RuntimeExecutionResult,
) -> serde_json::Value {
    let mut metrics_json = serde_json::json!({
        "input": artifact_path.display().to_string(),
        "execution": {
            "nodes_executed": execution.stats.executed_nodes,
            "nodes_failed": execution.stats.failed_nodes,
            "duration_ms": execution.stats.duration_ms,
            "status": if execution.stats.failed_nodes == 0 { "success" } else { "partial_failure" }
        },
        "scheduler": execution.scheduler_metrics.to_json()
    });
    let token_json = execution.token_snapshot.to_json();
    use apxm_core::constants::session::metrics_keys;
    if let Some(obj) = token_json.get(metrics_keys::TOKEN_ACCOUNTING).cloned() {
        metrics_json[metrics_keys::TOKEN_ACCOUNTING] = obj;
    }
    let graph_metrics_json = execution.graph_metrics_snapshot.to_json();
    if let Some(obj) = graph_metrics_json
        .get(metrics_keys::RUNTIME_GRAPH_METRICS)
        .cloned()
    {
        metrics_json[metrics_keys::RUNTIME_GRAPH_METRICS] = obj;
    }
    if let Some(observed) = &execution.stats.observed_graph {
        metrics_json[metrics_keys::RUNTIME_OBSERVED_GRAPH] =
            serde_json::to_value(observed).unwrap_or(serde_json::Value::Null);
    }
    #[cfg(feature = "metrics")]
    {
        let llm_metrics = &execution.llm_metrics;
        metrics_json["llm"] = serde_json::json!({
            "total_requests": llm_metrics.total_requests,
            "total_input_tokens": llm_metrics.total_input_tokens,
            "total_output_tokens": llm_metrics.total_output_tokens,
            "avg_latency_ms": llm_metrics.average_latency.as_millis(),
            "p50_latency_ms": llm_metrics.p50_latency.as_millis(),
            "p99_latency_ms": llm_metrics.p99_latency.as_millis()
        });
    }
    metrics_json
}

#[cfg(feature = "driver")]
async fn finish_step_session(
    linker: &Linker,
    writer: &apxm_driver::session_output::SessionOutputWriter,
    emitter: &apxm_driver::session_output::SessionEventEmitter,
    execution_id: &str,
    graph_name: Option<&str>,
    duration_ms: u64,
    execution: &apxm_runtime::RuntimeExecutionResult,
    node_count: Option<usize>,
    metrics_json: serde_json::Value,
    ticker_handle: tokio::task::JoinHandle<()>,
) -> Result<()> {
    ticker_handle.abort();
    let episodic_entries = linker
        .runtime_executor()
        .memory_system()
        .query_episodes(execution_id)
        .await
        .ok();
    writer
        .finalize(
            execution_id,
            graph_name,
            duration_ms as u128,
            node_count.unwrap_or(execution.stats.executed_nodes + execution.stats.failed_nodes),
            execution.stats.failed_nodes == 0,
            execution.all_outputs.as_ref(),
            execution.node_output_map.as_ref(),
            &execution.results,
            &metrics_json,
            &execution.stats.node_statuses,
            episodic_entries.as_deref(),
        )
        .context("Failed to finalize step session")?;
    emitter
        .finalize_live(execution.stats.failed_nodes == 0)
        .context("Failed to finalize step live state")?;
    Ok(())
}

#[cfg(feature = "driver")]
fn fail_step_session(
    writer: &apxm_driver::session_output::SessionOutputWriter,
    emitter: &apxm_driver::session_output::SessionEventEmitter,
    execution_id: &str,
    graph_name: Option<&str>,
    ticker_handle: tokio::task::JoinHandle<()>,
) -> Result<()> {
    ticker_handle.abort();
    writer
        .finalize_live_with_id(false, Some(execution_id), graph_name)
        .context("Failed to mark step session as failed")?;
    emitter
        .finalize_live(false)
        .context("Failed to finalize step live state")?;
    Ok(())
}

#[cfg(feature = "driver")]
fn load_graph_step_args(
    graph_path: &Path,
    resolved_params: &HashMap<String, String>,
) -> Result<Vec<String>> {
    let graph = air_graph_from_source(graph_path)?;
    Ok(graph
        .parameters
        .iter()
        .map(|p| resolved_params.get(&p.name).cloned().unwrap_or_default())
        .collect())
}

#[cfg(feature = "driver")]
fn load_artifact_step_args(
    artifact: &Artifact,
    resolved_params: &HashMap<String, String>,
) -> Result<Vec<String>> {
    let entry = artifact
        .entry_dag()
        .ok_or_else(|| anyhow::anyhow!("Artifact contains no entry DAG"))?;
    Ok(entry
        .metadata
        .parameters
        .iter()
        .map(|p| resolved_params.get(&p.name).cloned().unwrap_or_default())
        .collect())
}
