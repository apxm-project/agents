//! Session-file helpers for workflow-level followability.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use apxm_core::constants;
use apxm_core::events::payload::{
    EventPayload, PlanCreatedPayload, PlanStepCompletedPayload, PlanStepStartedPayload,
    SessionEndPayload, SessionStartPayload,
};
use apxm_core::events::{ApxmEvent, EventSource};
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::{
    CompletedNodeInfo, LiveSessionState, NodeInfo, SessionManifest, SessionStatus,
};

use super::{StepStatus, WorkflowResult, WorkflowStatus};

const BACKGROUND_FILE: &str = "background.json";

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
            workflow_name: Some(workflow_name.to_string()),
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
    initialize_workflow_trace(session_dir, &workflow_name, step_count)?;
    Ok(())
}

/// Persist background-launch metadata for a workflow session.
pub fn write_workflow_background_started(
    session_dir: &Path,
    pid: u32,
    log_file: &Path,
    command: &[String],
) -> Result<()> {
    std::fs::create_dir_all(session_dir)?;
    write_json(
        session_dir.join(BACKGROUND_FILE),
        &serde_json::json!({
            "pid": pid,
            "status": "background",
            "log_file": log_file,
            "command": command,
            "started_at": chrono::Utc::now().to_rfc3339(),
        }),
    )
}

/// Persist workflow-root progress when a child step starts.
pub fn write_workflow_step_started(
    session_dir: &Path,
    workflow_name: &str,
    step_id: &str,
    step_index: usize,
    completed: usize,
    total: usize,
    elapsed_ms: u128,
) -> Result<()> {
    append_next_trace_event(
        session_dir,
        PlanStepStartedPayload {
            plan_id: workflow_name.to_string(),
            step_index,
        },
    )?;
    let existing_live = read_live(session_dir);
    let completed_nodes = existing_live
        .as_ref()
        .map(|live| live.completed_nodes.clone())
        .unwrap_or_default();
    let mut running_nodes = existing_live
        .map(|live| live.running_nodes)
        .unwrap_or_default();
    let node_id = workflow_step_node_id(step_index);
    running_nodes.retain(|node| node.id != node_id);
    running_nodes.push(NodeInfo {
        id: node_id,
        name: step_id.to_string(),
        op: AISOperationType::WorkflowSpawn,
    });
    write_live(
        session_dir,
        &LiveSessionState {
            status: SessionStatus::Running,
            running_nodes,
            completed_nodes,
            completed,
            total: Some(total),
            elapsed_ms,
            success: false,
            current_phase: Some(format!("step:{step_id}")),
        },
    )
}

/// Persist workflow-root progress when a child step finishes or is skipped.
pub fn write_workflow_step_finished(
    session_dir: &Path,
    workflow_name: &str,
    step_id: &str,
    step_index: usize,
    status: StepStatus,
    duration_ms: u64,
    completed: usize,
    total: usize,
    elapsed_ms: u128,
) -> Result<()> {
    let success = status == StepStatus::Success;
    append_next_trace_event(
        session_dir,
        PlanStepCompletedPayload {
            plan_id: workflow_name.to_string(),
            step_index,
            success,
        },
    )?;
    let existing_live = read_live(session_dir);
    let mut running_nodes = existing_live
        .as_ref()
        .map(|live| live.running_nodes.clone())
        .unwrap_or_default();
    let node_id = workflow_step_node_id(step_index);
    running_nodes.retain(|node| node.id != node_id);
    let mut completed_nodes = existing_live
        .map(|live| live.completed_nodes)
        .unwrap_or_default();
    completed_nodes.push(CompletedNodeInfo {
        id: node_id,
        name: step_id.to_string(),
        op: AISOperationType::WorkflowSpawn,
        duration_ms,
        status: step_session_status(status),
        input_tokens: None,
        output_tokens: None,
        prefill_ms: None,
        decode_ms: None,
    });
    if completed_nodes.len() > 10 {
        completed_nodes = completed_nodes.split_off(completed_nodes.len() - 10);
    }

    write_live(
        session_dir,
        &LiveSessionState {
            status: SessionStatus::Running,
            running_nodes,
            completed_nodes,
            completed,
            total: Some(total),
            elapsed_ms,
            success: false,
            current_phase: Some(format!("step:{step_id}")),
        },
    )
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
    let completed_nodes = read_live(session_dir)
        .map(|live| live.completed_nodes)
        .unwrap_or_default();

    write_manifest(
        session_dir,
        &SessionManifest {
            execution_id: execution_id.clone(),
            workflow_name: Some(result.workflow_name.clone()),
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
            completed_nodes,
            completed: result.step_results.len(),
            total: Some(result.step_results.len()),
            elapsed_ms: result.duration_ms as u128,
            success,
            current_phase: None,
        },
    )?;
    write_results(session_dir, result)?;
    write_metrics(session_dir, result)?;
    append_next_trace_event(
        session_dir,
        SessionEndPayload {
            session_id: execution_id,
            total_turns: result.step_results.len(),
        },
    )?;
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

fn initialize_workflow_trace(
    session_dir: &Path,
    workflow_name: &str,
    step_count: usize,
) -> Result<()> {
    let trace_path = session_dir.join(constants::session::files::TRACE);
    if trace_path.is_file() && trace_path.metadata()?.len() > 0 {
        return Ok(());
    }

    let execution_id = workflow_execution_id(session_dir);
    append_trace_event(
        session_dir,
        &execution_id,
        0,
        SessionStartPayload {
            session_id: execution_id.clone(),
        },
    )?;
    append_trace_event(
        session_dir,
        &execution_id,
        1,
        PlanCreatedPayload {
            plan_id: workflow_name.to_string(),
            steps: step_count,
        },
    )
}

fn append_next_trace_event<P: EventPayload>(session_dir: &Path, payload: P) -> Result<()> {
    let execution_id = workflow_execution_id(session_dir);
    let seq = next_trace_seq(session_dir)?;
    append_trace_event(session_dir, &execution_id, seq, payload)
}

fn append_trace_event<P: EventPayload>(
    session_dir: &Path,
    trace_id: &str,
    seq: u64,
    payload: P,
) -> Result<()> {
    std::fs::create_dir_all(session_dir)?;
    let event = ApxmEvent::root(payload, EventSource::Session, trace_id).with_seq(seq);
    let line = serde_json::to_string(&event)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(session_dir.join(constants::session::files::TRACE))?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn next_trace_seq(session_dir: &Path) -> Result<u64> {
    let path = session_dir.join(constants::session::files::TRACE);
    if !path.is_file() {
        return Ok(0);
    }
    let text = std::fs::read_to_string(path)?;
    Ok(text.lines().count() as u64)
}

fn read_live(session_dir: &Path) -> Option<LiveSessionState> {
    let path = session_dir.join(constants::session::files::LIVE);
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn workflow_step_node_id(step_index: usize) -> u64 {
    step_index.saturating_add(1) as u64
}

fn step_session_status(status: StepStatus) -> SessionStatus {
    match status {
        StepStatus::Success => SessionStatus::Completed,
        StepStatus::Failed | StepStatus::Skipped => SessionStatus::Failed,
    }
}

fn write_json(path: std::path::PathBuf, value: &impl serde::Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    std::fs::write(path, json)?;
    Ok(())
}
