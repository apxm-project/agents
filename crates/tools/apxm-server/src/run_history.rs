//! Workflow-scoped run history index (spec 0013).
//!
//! Provides two read paths over the durable `ExecutionStore`:
//!
//! - `GET /v1/workflows/{id}/runs` — list all run summaries whose
//!   `workflow_id` matches `{id}`, newest-first.
//! - `GET /v1/runs/{execution_id}` — read one run summary by execution id
//!   (additive alias; the full record is still at `/v1/executions/{id}`).
//!
//! The reindex route (`POST /v1/runs/reindex`, spec 0013 Phase 4) walks
//! workflow run artifacts and hydrates lightweight records for workflow history.

pub mod storage;

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::executions::{ExecutionStatus, ReindexedExecutionRecord};
use crate::run_history::storage::StoredRunRecord;
use crate::state::AppState;

/// Lightweight run record returned by the history index.
///
/// Wire shape matches the spec 0013 run-history-row contract.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunRecord {
    pub(crate) run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) workflow_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) skill_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) skill_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) run_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) trace_id: Option<String>,
    pub(crate) status: ExecutionStatus,
    /// Unix milliseconds — recorded when the execution was admitted.
    pub(crate) started_at: u64,
    /// Unix milliseconds — recorded when the execution settled. `null` when
    /// still running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<u64>,
    /// Wall-clock duration in milliseconds. `null` when the run has not
    /// settled yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cost_usd: Option<f64>,
    pub(crate) retention_class: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkflowRunsResponse {
    pub(crate) workflow_id: String,
    pub(crate) runs: Vec<RunRecord>,
}

/// `GET /v1/workflows/{id}/runs` — list all run summaries for a workflow.
///
/// Scans the in-memory `ExecutionStore` for records whose `workflow_id`
/// matches the path parameter, returning them newest-first. An unknown
/// workflow id returns an empty list (not 404) so callers can poll safely
/// before any runs have been submitted.
pub(crate) async fn list_workflow_runs(
    State(state): State<AppState>,
    Path(workflow_id): Path<String>,
) -> Json<WorkflowRunsResponse> {
    let runs: Vec<RunRecord> = state
        .execution_store
        .list_workflow_history(workflow_id.as_str())
        .into_iter()
        .map(RunRecord::from)
        .collect();

    Json(WorkflowRunsResponse { workflow_id, runs })
}

/// `GET /v1/runs/{execution_id}/summary` — read one run summary by execution id.
///
/// This is an additive alias that returns the lighter `RunRecord` shape
/// alongside the richer `/v1/executions/{id}` record. Returns 404 when the
/// run is unknown.
pub(crate) async fn get_run_summary(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Result<Json<RunRecord>, ApiError> {
    let record = state
        .execution_store
        .get_run_history(&execution_id)
        .ok_or_else(|| ApiError::not_found(format!("run not found: {execution_id}")))?;
    Ok(Json(RunRecord::from(record)))
}

/// Minimal run-summary artifact written to `{run_root}/run.json` on settlement
/// (spec 0009 T035). Mirrors `executions::RunArtifact`; declared here so the
/// reindex walker can deserialize it without reaching into the private type.
#[derive(Debug, Deserialize)]
struct RunArtifactFile {
    run_id: String,
    #[serde(default)]
    execution_id: Option<String>,
    workflow_id: String,
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    skill_version: Option<String>,
    #[serde(default)]
    entry_flow: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    session_dir: Option<String>,
    #[serde(default)]
    run_root: Option<String>,
    #[serde(default)]
    trace_id: Option<String>,
    status: ExecutionStatus,
    started_at_ms: u64,
    finished_at_ms: Option<u64>,
    duration_ms: Option<u64>,
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
    #[serde(default)]
    cost_usd: Option<f64>,
    #[serde(default)]
    llm_usage: Option<RunArtifactLlmUsage>,
}

#[derive(Debug, Deserialize)]
struct RunArtifactLlmUsage {
    input_tokens: u64,
    output_tokens: u64,
}

/// Outcome of a reindex walk — counts of artifacts found, loaded, and skipped.
#[derive(Debug, Serialize)]
pub(crate) struct ReindexResult {
    pub(crate) artifacts_found: usize,
    pub(crate) records_loaded: usize,
    pub(crate) diagnostics: Vec<String>,
}

/// Walk `$APXM_RUNS_ROOT/{workflow_id}/{execution_id}/run.json` and populate
/// the in-memory `ExecutionStore` from durable run artifacts.
///
/// Each artifact is a `run.json` written by `executions::write_run_artifact`
/// on settlement (spec 0009 T035). The walker treats corrupt or missing files
/// as diagnostics rather than errors so a partial artifact tree is usable.
pub(crate) fn reindex_from_runs_root(state: &AppState) -> ReindexResult {
    let Ok(runs_root) = std::env::var("APXM_RUNS_ROOT") else {
        return ReindexResult {
            artifacts_found: 0,
            records_loaded: 0,
            diagnostics: vec!["APXM_RUNS_ROOT is not set; skipping reindex".into()],
        };
    };

    let root = std::path::Path::new(&runs_root);
    if !root.is_dir() {
        return ReindexResult {
            artifacts_found: 0,
            records_loaded: 0,
            diagnostics: vec![format!(
                "APXM_RUNS_ROOT={runs_root} does not exist or is not a directory"
            )],
        };
    }

    let mut artifacts_found = 0usize;
    let mut records_loaded = 0usize;
    let mut diagnostics: Vec<String> = Vec::new();

    // Walk: runs_root/<workflow_id>/<execution_id>/run.json
    let Ok(wf_dirs) = std::fs::read_dir(root) else {
        diagnostics.push(format!("failed to read APXM_RUNS_ROOT={runs_root}"));
        return ReindexResult {
            artifacts_found,
            records_loaded,
            diagnostics,
        };
    };

    for wf_entry in wf_dirs.filter_map(Result::ok) {
        let wf_path = wf_entry.path();
        if !wf_path.is_dir() {
            continue;
        }
        let Ok(run_dirs) = std::fs::read_dir(&wf_path) else {
            diagnostics.push(format!("failed to read {}", wf_path.display()));
            continue;
        };
        for run_entry in run_dirs.filter_map(Result::ok) {
            let run_dir = run_entry.path();
            if !run_dir.is_dir() {
                continue;
            }
            let artifact_path = run_dir.join("run.json");
            if !artifact_path.exists() {
                continue;
            }
            artifacts_found += 1;

            let bytes = match std::fs::read(&artifact_path) {
                Ok(b) => b,
                Err(e) => {
                    diagnostics.push(format!("read error {}: {e}", artifact_path.display()));
                    continue;
                }
            };
            let artifact: RunArtifactFile = match serde_json::from_slice(&bytes) {
                Ok(a) => a,
                Err(e) => {
                    diagnostics.push(format!("parse error {}: {e}", artifact_path.display()));
                    continue;
                }
            };

            let execution_id = artifact
                .execution_id
                .clone()
                .unwrap_or_else(|| artifact.run_id.clone());

            // Synthesize a lightweight execution record from the run artifact so
            // workflow history remains queryable after a process restart even if
            // only `$APXM_RUNS_ROOT` is available.
            tracing::debug!(
                run_id = %execution_id,
                workflow_id = %artifact.workflow_id,
                "reindex: found settled run artifact"
            );
            let session_id = artifact
                .session_id
                .clone()
                .unwrap_or_else(|| format!("reindexed-{execution_id}"));
            let session_dir = artifact
                .session_dir
                .clone()
                .unwrap_or_else(|| run_dir.display().to_string());
            let run_root = artifact
                .run_root
                .clone()
                .or_else(|| Some(run_dir.display().to_string()));
            let input_tokens = artifact
                .input_tokens
                .or_else(|| artifact.llm_usage.as_ref().map(|usage| usage.input_tokens))
                .unwrap_or(0);
            let output_tokens = artifact
                .output_tokens
                .or_else(|| artifact.llm_usage.as_ref().map(|usage| usage.output_tokens))
                .unwrap_or(0);
            let total_tokens = artifact
                .total_tokens
                .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
            let inserted = if state.execution_store.get(&execution_id).is_some() {
                false
            } else {
                state
                    .execution_store
                    .upsert_reindexed(ReindexedExecutionRecord {
                        execution_id: execution_id.clone(),
                        skill_id: artifact.skill_id.clone().unwrap_or_default(),
                        skill_version: artifact.skill_version.clone().unwrap_or_default(),
                        entry_flow: artifact.entry_flow.clone(),
                        workflow_id: Some(artifact.workflow_id.clone()),
                        session_id: session_id.clone(),
                        session_dir: session_dir.clone(),
                        run_root: run_root.clone(),
                        trace_id: artifact.trace_id.clone(),
                        status: artifact.status.clone(),
                        started_at_ms: artifact.started_at_ms,
                        completed_at_ms: artifact.finished_at_ms,
                    })
            };
            if inserted {
                records_loaded += 1;
            }
            state
                .execution_store
                .upsert_run_history_row(StoredRunRecord {
                    run_id: execution_id,
                    workflow_id: Some(artifact.workflow_id),
                    skill_id: artifact.skill_id,
                    skill_version: artifact.skill_version,
                    session_id: artifact.session_id,
                    session_dir: artifact.session_dir,
                    run_root,
                    trace_id: artifact.trace_id,
                    status: artifact.status,
                    started_at: artifact.started_at_ms,
                    finished_at: artifact.finished_at_ms,
                    duration_ms: artifact.duration_ms,
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cost_usd: artifact.cost_usd,
                    retention_class: "standard".to_string(),
                });
        }
    }

    ReindexResult {
        artifacts_found,
        records_loaded,
        diagnostics,
    }
}

impl From<crate::executions::ExecutionRecord> for RunRecord {
    fn from(record: crate::executions::ExecutionRecord) -> Self {
        let (input_tokens, output_tokens, total_tokens) = record
            .result
            .as_ref()
            .map(|result| {
                let input = result.llm_usage.input_tokens as u64;
                let output = result.llm_usage.output_tokens as u64;
                (input, output, input.saturating_add(output))
            })
            .unwrap_or((0, 0, 0));
        RunRecord {
            run_id: record.execution_id,
            workflow_id: record.workflow_id,
            skill_id: Some(record.skill_id),
            skill_version: Some(record.skill_version),
            session_id: Some(record.session_id),
            session_dir: Some(record.session_dir),
            run_root: record.run_root,
            trace_id: record.trace_id,
            status: record.status,
            started_at: record.started_at_ms,
            finished_at: record.completed_at_ms,
            duration_ms: record
                .completed_at_ms
                .and_then(|end| end.checked_sub(record.started_at_ms)),
            input_tokens,
            output_tokens,
            total_tokens,
            cost_usd: None,
            retention_class: "standard".to_string(),
        }
    }
}

impl From<StoredRunRecord> for RunRecord {
    fn from(row: StoredRunRecord) -> Self {
        RunRecord {
            run_id: row.run_id,
            workflow_id: row.workflow_id,
            skill_id: row.skill_id,
            skill_version: row.skill_version,
            session_id: row.session_id,
            session_dir: row.session_dir,
            run_root: row.run_root,
            trace_id: row.trace_id,
            status: row.status,
            started_at: row.started_at,
            finished_at: row.finished_at,
            duration_ms: row.duration_ms,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            total_tokens: row.total_tokens,
            cost_usd: row.cost_usd,
            retention_class: row.retention_class,
        }
    }
}

/// `POST /v1/runs/reindex` — rebuild the run-history index from durable
/// artifacts under the 0009 run-root (spec 0013 User Story 2).
pub(crate) async fn reindex_runs(State(state): State<AppState>) -> Json<serde_json::Value> {
    let result = reindex_from_runs_root(&state);
    Json(serde_json::json!({
        "artifacts_found": result.artifacts_found,
        "records_loaded": result.records_loaded,
        "diagnostics": result.diagnostics,
    }))
}
