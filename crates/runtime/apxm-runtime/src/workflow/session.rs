//! Session-file helpers for workflow-level followability.

use std::path::Path;

use anyhow::Result;
use apxm_core::constants;
use apxm_core::types::{LiveSessionState, SessionManifest, SessionStatus};

use super::{StepStatus, WorkflowResult, WorkflowStatus};

/// Persist the workflow root session as running.
///
/// Step-level graph sessions already get their own rich node traces. This
/// workflow-level record makes the enclosing `.apxmw` discoverable through
/// `apxm session list` and inspectable while or after it runs.
pub fn write_workflow_session_started(
    session_dir: &Path,
    workflow_name: &str,
    step_count: usize,
) -> Result<()> {
    std::fs::create_dir_all(session_dir)?;
    let execution_id = workflow_execution_id(session_dir);
    let timestamp = chrono::Utc::now().to_rfc3339();

    write_manifest(
        session_dir,
        &SessionManifest {
            execution_id,
            graph_name: Some(workflow_name.to_string()),
            timestamp,
            status: SessionStatus::Running,
            duration_ms: 0,
            node_count: step_count,
            success: false,
            scope_id: None,
            parent_execution_id: None,
            parent_session_dir: None,
            parent_scope_id: None,
            spawn_node_id: None,
        },
    )?;
    write_live(
        session_dir,
        &LiveSessionState {
            status: SessionStatus::Running,
            running_nodes: Vec::new(),
            completed_nodes: Vec::new(),
            completed: 0,
            total: Some(step_count),
            elapsed_ms: 0,
            success: false,
            current_phase: Some("workflow".to_string()),
        },
    )?;
    Ok(())
}

/// Persist final workflow-level session files.
pub fn write_workflow_session_finished(session_dir: &Path, result: &WorkflowResult) -> Result<()> {
    std::fs::create_dir_all(session_dir)?;
    let execution_id = workflow_execution_id(session_dir);
    let timestamp =
        existing_manifest_timestamp(session_dir).unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let success = result.status == WorkflowStatus::Success;
    let status = if success {
        SessionStatus::Completed
    } else {
        SessionStatus::Failed
    };

    write_manifest(
        session_dir,
        &SessionManifest {
            execution_id,
            graph_name: Some(result.workflow_name.clone()),
            timestamp,
            status,
            duration_ms: result.duration_ms as u128,
            node_count: result.step_results.len(),
            success,
            scope_id: None,
            parent_execution_id: None,
            parent_session_dir: None,
            parent_scope_id: None,
            spawn_node_id: None,
        },
    )?;
    write_live(
        session_dir,
        &LiveSessionState {
            status,
            running_nodes: Vec::new(),
            completed_nodes: Vec::new(),
            completed: result.step_results.len(),
            total: Some(result.step_results.len()),
            elapsed_ms: result.duration_ms as u128,
            success,
            current_phase: None,
        },
    )?;
    write_results(session_dir, result)?;
    write_metrics(session_dir, result)?;
    Ok(())
}

fn workflow_execution_id(session_dir: &Path) -> String {
    session_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("workflow")
        .to_string()
}

fn existing_manifest_timestamp(session_dir: &Path) -> Option<String> {
    let path = session_dir.join(constants::session::files::MANIFEST);
    let text = std::fs::read_to_string(path).ok()?;
    let manifest: SessionManifest = serde_json::from_str(&text).ok()?;
    Some(manifest.timestamp)
}

fn write_manifest(session_dir: &Path, manifest: &SessionManifest) -> Result<()> {
    write_json(
        session_dir.join(constants::session::files::MANIFEST),
        manifest,
    )
}

fn write_live(session_dir: &Path, live: &LiveSessionState) -> Result<()> {
    write_json(session_dir.join(constants::session::files::LIVE), live)
}

fn write_results(session_dir: &Path, result: &WorkflowResult) -> Result<()> {
    write_json(
        session_dir.join(constants::session::files::RESULTS),
        &serde_json::json!({
            "workflow_name": result.workflow_name,
            "status": format!("{:?}", result.status),
            "duration_ms": result.duration_ms,
            "output": result.output,
            "step_results": result.step_results,
        }),
    )
}

fn write_metrics(session_dir: &Path, result: &WorkflowResult) -> Result<()> {
    let mut success = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    for step in result.step_results.values() {
        match step.status {
            StepStatus::Success => success += 1,
            StepStatus::Failed => failed += 1,
            StepStatus::Skipped => skipped += 1,
        }
    }

    write_json(
        session_dir.join(constants::session::files::METRICS),
        &serde_json::json!({
            "workflow": {
                "name": result.workflow_name,
                "status": format!("{:?}", result.status),
                "duration_ms": result.duration_ms,
                "step_count": result.step_results.len(),
                "successful_steps": success,
                "failed_steps": failed,
                "skipped_steps": skipped,
            }
        }),
    )
}

fn write_json(path: std::path::PathBuf, value: &impl serde::Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    std::fs::write(path, json)?;
    Ok(())
}
