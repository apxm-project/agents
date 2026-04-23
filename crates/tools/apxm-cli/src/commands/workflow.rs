//! Legacy workflow-file execution commands.

use std::path::PathBuf;

use anyhow::{Context, Result};
#[cfg(feature = "driver")]
use apxm_driver::{Linker, LinkerConfig};
use colored::Colorize;

use super::cli::*;
#[cfg(feature = "driver")]
use super::implementations::load_config;

#[cfg(feature = "driver")]
pub async fn workflow_command(action: WorkflowAction, json: bool) -> Result<()> {
    match action {
        WorkflowAction::Run { file, args } => workflow_run_command(file, args).await,
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
            "Legacy workflow execution requires the `driver` feature. Rebuild through `dekk apxm build`, then re-run `dekk apxm workflow run ...`."
        )),
    }
}

pub fn workflow_validate_command(file: PathBuf, json: bool) -> Result<()> {
    use apxm_runtime::workflow::WorkflowDef;

    let def = WorkflowDef::from_file(&file)
        .with_context(|| format!("Failed to load legacy workflow file {}", file.display()))?;

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
                "{} Legacy workflow is valid",
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
        .with_context(|| format!("Failed to load legacy workflow file {}", file.display()))?;

    let errors = def.validate();
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(
            "Legacy workflow validation failed: {}",
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
        println!("Legacy workflow: {}", def.name.bold());
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
pub async fn workflow_run_command(file: PathBuf, args: Vec<String>) -> Result<()> {
    use apxm_runtime::workflow::{WorkflowDef, execution_phases};
    use std::collections::HashMap;
    use std::time::Instant;

    // Parse legacy workflow
    let def = WorkflowDef::from_file(&file)
        .with_context(|| format!("Failed to load legacy workflow file {}", file.display()))?;

    // Validate legacy workflow
    let errors = def.validate();
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(
            "Legacy workflow validation failed: {}",
            errors.join(", ")
        ));
    }

    // Parse arguments (name=value format)
    let mut params = HashMap::new();
    for arg in &args {
        if let Some((key, value)) = arg.split_once('=') {
            params.insert(key.to_string(), value.to_string());
        } else {
            return Err(anyhow::anyhow!(
                "Invalid argument format '{}'. Expected name=value",
                arg
            ));
        }
    }

    // Check that all required parameters are provided
    for param in &def.parameters {
        if !params.contains_key(&param.name) {
            return Err(anyhow::anyhow!(
                "Missing required parameter: {} (type: {})",
                param.name,
                param.type_name
            ));
        }
    }

    println!("Executing legacy workflow: {}", def.name.bold());
    println!();

    let base_dir = file
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Failed to get parent directory"))?
        .to_path_buf();

    let phases = execution_phases(&def.graphs)?;
    let start = Instant::now();

    // Create workflow session directory
    let paths = apxm_core::paths::ApxmPaths::discover()?;
    let workflow_session_dir = paths.sessions_dir()?.join(format!(
        "workflow-{}-{}",
        def.name,
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));

    println!("Session directory: {}", workflow_session_dir.display());
    println!();

    let mut step_outputs: HashMap<String, String> = HashMap::new();
    let mut step_results: HashMap<String, apxm_runtime::workflow::StepResult> = HashMap::new();

    // Load config and create linker
    let config = load_config(None)?;
    let linker_config = LinkerConfig::from_apxm_config(config);
    let linker = Linker::new(linker_config).await?;

    // Execute phases
    for (phase_idx, phase) in phases.iter().enumerate() {
        println!("Phase {}: {} step(s)", phase_idx, phase.len());

        for step_id in phase {
            let step = def.graphs.iter().find(|s| &s.id == step_id).unwrap();

            // Check if any dependency failed → skip this step
            let should_skip = step.depends_on.iter().any(|dep| {
                step_results.get(dep).map_or(false, |r| {
                    r.status != apxm_runtime::workflow::StepStatus::Success
                })
            });

            if should_skip {
                println!("  {} Skipping (failed dependency)", step_id);
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

            // Resolve parameters
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

            let step_session_dir = workflow_session_dir.join(&step.id);
            std::fs::create_dir_all(&step_session_dir)?;

            let graph_path = base_dir.join(&step.path);

            println!(
                "  {} Starting: {} ({})",
                apxm_core::constants::ui::icons::STARTED,
                step_id,
                graph_path.display()
            );

            let step_start = Instant::now();

            // Convert resolved params to Vec<String> for the graph's parameters
            let graph_args: Vec<String> = {
                // Load the graph to get its parameter order
                let graph_bytes = std::fs::read(&graph_path)?;
                let graph_text = std::str::from_utf8(&graph_bytes)?;
                let graph: apxm_compiler::AirModule = serde_json::from_str(graph_text)?;

                graph
                    .parameters
                    .iter()
                    .map(|p| resolved_params.get(&p.name).cloned().unwrap_or_default())
                    .collect()
            };

            // Execute the graph
            let result = linker
                .run_graph(&graph_path, graph_args, None, Some(&step_session_dir))
                .await;

            let duration_ms = step_start.elapsed().as_millis() as u64;

            match result {
                Ok(link_result) => {
                    // Extract the final output value
                    let output =
                        link_result
                            .execution
                            .results
                            .values()
                            .last()
                            .and_then(|v| match v {
                                apxm_core::types::Value::String(s) => Some(s.clone()),
                                _ => Some(v.to_string()),
                            });

                    println!(
                        "  {} Completed: {} ({:.1}s)",
                        apxm_core::constants::ui::icons::SUCCESS,
                        step_id,
                        duration_ms as f64 / 1000.0
                    );

                    if let Some(ref out) = output {
                        step_outputs.insert(step_id.clone(), out.clone());
                    }

                    step_results.insert(
                        step_id.clone(),
                        apxm_runtime::workflow::StepResult {
                            id: step_id.clone(),
                            status: apxm_runtime::workflow::StepStatus::Success,
                            output,
                            duration_ms,
                            session_dir: Some(step_session_dir),
                            error: None,
                        },
                    );
                }
                Err(e) => {
                    println!(
                        "  {} Failed: {} - {}",
                        apxm_core::constants::ui::icons::FAILED,
                        step_id,
                        e
                    );

                    step_results.insert(
                        step_id.clone(),
                        apxm_runtime::workflow::StepResult {
                            id: step_id.clone(),
                            status: apxm_runtime::workflow::StepStatus::Failed,
                            output: None,
                            duration_ms,
                            session_dir: Some(step_session_dir),
                            error: Some(e.to_string()),
                        },
                    );
                }
            }
        }

        println!();
    }

    // Resolve final output
    let output = def
        .output
        .as_ref()
        .map(|tmpl| apxm_runtime::workflow::resolve(tmpl, &step_outputs, &params));

    let total_duration = start.elapsed();

    println!(
        "Legacy workflow completed in {:.1}s",
        total_duration.as_secs_f64()
    );
    println!();
    println!("Results:");
    for (step_id, result) in &step_results {
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

    if let Some(output) = output {
        println!();
        println!("Final output:");
        println!("{}", output);
    }

    Ok(())
}
