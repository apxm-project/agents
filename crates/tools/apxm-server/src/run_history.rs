//! Workflow-scoped run history index (spec 0013).
//!
//! Provides two read paths over the durable `ExecutionStore`:
//!
//! - `GET /v1/workflows/{id}/runs` — list all run summaries whose
//!   `workflow_id` matches `{id}`, newest-first.
//! - `GET /v1/runs/{execution_id}` — read one run summary by execution id
//!   (additive alias; the full record is still at `/v1/executions/{id}`).
//!
//! The reindex route (`POST /v1/runs/reindex`, spec 0013 Phase 4) is scaffolded
//! here and returns 501 until the artifact-walk rebuild is implemented.

use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;

use crate::error::ApiError;
use crate::executions::ExecutionStatus;
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
    pub(crate) session_id: Option<String>,
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
        .list()
        .into_iter()
        .filter(|record| record.workflow_id.as_deref() == Some(workflow_id.as_str()))
        .map(|record| RunRecord {
            run_id: record.execution_id,
            workflow_id: record.workflow_id,
            session_id: Some(record.session_id),
            status: record.status,
            started_at: record.started_at_ms,
            finished_at: record.completed_at_ms,
            duration_ms: record
                .completed_at_ms
                .and_then(|end| end.checked_sub(record.started_at_ms)),
        })
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
        .get(&execution_id)
        .ok_or_else(|| ApiError::not_found(format!("run not found: {execution_id}")))?;
    Ok(Json(RunRecord {
        run_id: record.execution_id,
        workflow_id: record.workflow_id,
        session_id: Some(record.session_id),
        status: record.status,
        started_at: record.started_at_ms,
        finished_at: record.completed_at_ms,
        duration_ms: record
            .completed_at_ms
            .and_then(|end| end.checked_sub(record.started_at_ms)),
    }))
}

/// `POST /v1/runs/reindex` — rebuild the run-history index from durable
/// artifacts on disk (spec 0013 User Story 2).
///
/// Phase 4 placeholder — returns 501 until the artifact-walk rebuild is
/// implemented.
pub(crate) async fn reindex_runs(
    State(_state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Err(ApiError::from_parts(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "run reindex is not yet implemented",
    ))
}
