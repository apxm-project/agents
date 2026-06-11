use apxm_driver::RunEventsConfig;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::error::ApiError;
use crate::executions::ExecutionRecord;
use crate::goal_runs::{GoalRunRecord, GoalRunStatus};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct GoalsListQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GoalEventsQuery {
    #[serde(default)]
    since: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GoalListResponse {
    object: &'static str,
    data: Vec<GoalStatusResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GoalStatusResponse {
    goal_id: String,
    status: GoalRunStatus,
    task: GoalTaskState,
    started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at_ms: Option<u64>,
    iteration: usize,
    max_iterations: usize,
    pass_execution_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workflow_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    control: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    goal: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    cancel_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_pass: Option<GoalPassProgress>,
    totals: GoalTotals,
}

#[derive(Debug, Serialize)]
struct GoalTaskState {
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    planning: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
struct GoalPassProgress {
    execution_id: String,
    status: String,
    events: usize,
    node_outputs: usize,
    node_metrics: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GoalEventsResponse {
    goal_id: String,
    events: Vec<JsonValue>,
    next_seq: u64,
    done: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct GoalCancelResponse {
    goal_id: String,
    cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_execution_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct GoalTotals {
    events: usize,
    passes: usize,
}

pub(crate) async fn list_goals(
    State(state): State<AppState>,
    Query(query): Query<GoalsListQuery>,
) -> Json<GoalListResponse> {
    let limit = list_limit(&state.server_config.run_events, query.limit);
    let status_filter = query.status.as_deref().and_then(parse_goal_status);
    let mut goals = state
        .goal_runs
        .list()
        .into_iter()
        .filter(|record| {
            status_filter
                .as_ref()
                .is_none_or(|status| record.status == *status)
        })
        .map(|record| {
            let event_count = state.run_event_bus.snapshot(&record.goal_id).len();
            let latest_pass = latest_pass_progress(&state, &record);
            goal_status_response(record, event_count, latest_pass)
        })
        .collect::<Vec<_>>();
    goals.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
    goals.truncate(limit);
    Json(GoalListResponse {
        object: "list",
        data: goals,
    })
}

pub(crate) async fn get_goal(
    State(state): State<AppState>,
    Path(goal_id): Path<String>,
) -> Result<Json<GoalStatusResponse>, ApiError> {
    goal_status_for_state(&state, &goal_id).map(Json)
}

pub(crate) async fn get_goal_events_bulk(
    State(state): State<AppState>,
    Path(goal_id): Path<String>,
    Query(query): Query<GoalEventsQuery>,
) -> Result<Json<GoalEventsResponse>, ApiError> {
    let since = query.since.unwrap_or(0);
    let limit = events_limit(&state.server_config.run_events, query.limit);
    goal_events_for_state(&state, &goal_id, since, limit).map(Json)
}

pub(crate) async fn cancel_goal(
    State(state): State<AppState>,
    Path(goal_id): Path<String>,
) -> Result<Json<GoalCancelResponse>, ApiError> {
    goal_cancel_for_state(&state, &goal_id).map(Json)
}

pub(crate) fn goal_status_for_state(
    state: &AppState,
    goal_id: &str,
) -> Result<GoalStatusResponse, ApiError> {
    validate_goal_id(goal_id)?;
    let record = state
        .goal_runs
        .get(goal_id)
        .ok_or_else(|| ApiError::not_found(format!("goal run not found: {goal_id}")))?;
    Ok(goal_status_response(
        record.clone(),
        state.run_event_bus.snapshot(goal_id).len(),
        latest_pass_progress(state, &record),
    ))
}

pub(crate) fn goal_events_for_state(
    state: &AppState,
    goal_id: &str,
    since: u64,
    limit: usize,
) -> Result<GoalEventsResponse, ApiError> {
    validate_goal_id(goal_id)?;
    let limit = limit.clamp(1, 1000);
    let events = state.run_event_bus.snapshot(goal_id);
    if events.is_empty() && state.goal_runs.get(goal_id).is_none() {
        return Err(ApiError::not_found(format!(
            "goal run not found: {goal_id}"
        )));
    }
    let filtered = events
        .iter()
        .filter(|event| event.meta.seq >= since)
        .collect::<Vec<_>>();
    let page = filtered.iter().take(limit).copied().collect::<Vec<_>>();
    let next_seq = page
        .last()
        .map_or(since, |event| event.meta.seq.saturating_add(1));
    Ok(GoalEventsResponse {
        goal_id: goal_id.to_string(),
        events: page
            .into_iter()
            .map(|event| serde_json::to_value(event).unwrap_or(JsonValue::Null))
            .collect(),
        next_seq,
        done: filtered.len() <= limit,
    })
}

pub(crate) fn goal_cancel_for_state(
    state: &AppState,
    goal_id: &str,
) -> Result<GoalCancelResponse, ApiError> {
    validate_goal_id(goal_id)?;
    let record = state
        .goal_runs
        .request_cancel(goal_id)
        .ok_or_else(|| ApiError::not_found(format!("goal run not found: {goal_id}")))?;
    if record.status.is_terminal() {
        return Ok(GoalCancelResponse {
            goal_id: goal_id.to_string(),
            cancelled: false,
            current_execution_id: None,
        });
    }
    if let Some(execution_id) = &record.current_execution_id {
        if let Some(notify) = state.cancel_registry.get(execution_id) {
            notify.notify_one();
        } else {
            state.goal_runs.finish(
                goal_id,
                GoalRunStatus::Cancelled,
                record.goal.clone(),
                Some("cancelled".to_string()),
            );
        }
    } else {
        state.goal_runs.finish(
            goal_id,
            GoalRunStatus::Cancelled,
            record.goal.clone(),
            Some("cancelled".to_string()),
        );
    }
    Ok(GoalCancelResponse {
        goal_id: goal_id.to_string(),
        cancelled: true,
        current_execution_id: record.current_execution_id,
    })
}

fn goal_status_response(
    record: GoalRunRecord,
    event_count: usize,
    latest_pass: Option<GoalPassProgress>,
) -> GoalStatusResponse {
    GoalStatusResponse {
        goal_id: record.goal_id,
        status: record.status,
        task: GoalTaskState {
            description: record.task,
            plan: record.plan,
            planning: record.planning,
        },
        started_at_ms: record.started_at_ms,
        completed_at_ms: record.completed_at_ms,
        iteration: record.iteration,
        max_iterations: record.max_iterations,
        pass_execution_ids: record.pass_execution_ids.clone(),
        current_execution_id: record.current_execution_id,
        session_id: record.session_id,
        session_dir: record.session_dir,
        workflow_path: record.workflow_path,
        bundle_dir: record.bundle_dir,
        artifacts: record.artifacts,
        selection: record.selection,
        control: record.control,
        goal: record.goal,
        error: record.error,
        cancel_requested: record.cancel_requested,
        latest_pass,
        totals: GoalTotals {
            events: event_count,
            passes: record.pass_execution_ids.len(),
        },
    }
}

fn latest_pass_progress(state: &AppState, record: &GoalRunRecord) -> Option<GoalPassProgress> {
    let execution_id = record
        .current_execution_id
        .as_deref()
        .or_else(|| record.pass_execution_ids.last().map(String::as_str))?;
    let event_count = state.run_event_bus.snapshot(execution_id).len();
    match state.execution_store.get(execution_id) {
        Some(pass) => Some(pass_progress_from_record(pass, event_count)),
        None => Some(GoalPassProgress {
            execution_id: execution_id.to_string(),
            status: "unknown".to_string(),
            events: event_count,
            node_outputs: 0,
            node_metrics: 0,
            session_id: record.session_id.clone(),
            session_dir: record.session_dir.clone(),
            completed_at_ms: None,
            error: None,
        }),
    }
}

fn pass_progress_from_record(record: ExecutionRecord, event_count: usize) -> GoalPassProgress {
    GoalPassProgress {
        execution_id: record.execution_id,
        status: serde_json::to_value(&record.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string()),
        events: event_count,
        node_outputs: record.node_outputs.len(),
        node_metrics: record.node_metrics.len(),
        session_id: Some(record.session_id),
        session_dir: Some(record.session_dir),
        completed_at_ms: record.completed_at_ms,
        error: record.error,
    }
}

fn parse_goal_status(raw: &str) -> Option<GoalRunStatus> {
    match raw {
        "running" => Some(GoalRunStatus::Running),
        "succeeded" | "done" => Some(GoalRunStatus::Succeeded),
        "failed" => Some(GoalRunStatus::Failed),
        "cancelled" | "canceled" => Some(GoalRunStatus::Cancelled),
        _ => None,
    }
}

fn list_limit(config: &RunEventsConfig, raw: Option<usize>) -> usize {
    clamp_limit(raw, config.default_list_limit, config.max_list_limit)
}

fn events_limit(config: &RunEventsConfig, raw: Option<usize>) -> usize {
    clamp_limit(raw, config.default_events_limit, config.max_events_limit)
}

fn clamp_limit(raw: Option<usize>, default: usize, max: usize) -> usize {
    let max = max.max(1);
    let default = default.clamp(1, max);
    raw.unwrap_or(default).clamp(1, max)
}

fn validate_goal_id(goal_id: &str) -> Result<(), ApiError> {
    if goal_id.is_empty() || goal_id.len() > 96 || goal_id == "." || goal_id == ".." {
        return Err(ApiError::bad_request(
            "goal_id must be 1-96 characters and not '.' or '..'",
        ));
    }
    if !goal_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(ApiError::bad_request(
            "goal_id may contain only ASCII letters, digits, '_' and '-'",
        ));
    }
    Ok(())
}
