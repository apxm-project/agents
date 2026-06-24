//! OpenAPI export for the session + permission surface and the run-history surface.
//!
//! Wire shapes are derived from server types via `utoipa`; CI diff-tests the
//! export against `contracts/session-api.yaml`.
//!
//! # Route inventory
//!
//! The exhaustive route list lives in `routes.rs`. The full server contract
//! artifact is at `contracts/server-api.yaml` (generated from source —
//! keep in sync). Key surface groups:
//!
//! - Execution: `POST /v1/execute` · `/execute/stream` · `/compile` · `/compile/stream`
//! - Skills: `GET /v1/skills` · `/{id}` · `POST /{id}/execute` · `/{id}/execute/stream`
//! - Runs: `GET /v1/runs` · `POST /v1/runs/clear` · `/{id}` · `/graph`
//!   · `/nodes/{node}` · `/artifacts` · `/artifacts/{path}` · `/events/stream`
//!   · `POST /{id}/cancel`
//! - Session: `GET /v1/sessions/{id}/status` · `POST /cancel` · `/compact`
//! - Goals: `GET /v1/goals` · `/{id}` · `/events/stream` · `POST /{id}/cancel`
//! - Memory: `POST /v1/memory/facts/store` · `/search` · `/delete`
//! - Capabilities: `GET /v1/capability-templates` · `/reindex` ·
//!   `POST /v1/capabilities/delegate` · `POST /{capability_id}/revoke` ·
//!   `POST /v1/capabilities/{capability_id}/invoke`
//! - Agents/A2A: `GET /.well-known/agent.json` · `POST /a2a/tasks/send`
//! - Permission: `POST /v1/permissions/{id}/respond`
//!
//! This file (`openapi.rs`) owns the utoipa-generated slice for session + permission.
//! Remaining surfaces are documented in `contracts/server-api.yaml` pending
//! full utoipa coverage.

use serde::Serialize;
use utoipa::{OpenApi, ToSchema};

use crate::permissions::{PermissionDecision, PermissionResponse};
use crate::sessions::{SessionLedgerView, SessionStatus};
use crate::types::errors::{FaultClass, TypedError};

/// OpenAPI document for session control + permission response endpoints.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "APXM Session API (contract snapshot — 0002)",
        version = "0.1.0-draft",
        description = "Draft contract generated from apxm-server types at implementation;\nthis file is the CI diff-test baseline. See review §7 session model."
    ),
    paths(
        doc_get_session_status,
        doc_cancel_session,
        doc_compact_session,
        doc_list_session_events,
        doc_stream_session_events,
        doc_respond_permission,
    ),
    components(schemas(
        SessionStatus,
        SessionLedgerView,
        PermissionResponse,
        PermissionDecision,
        TypedError,
        FaultClass,
    )),
    tags((name = "session", description = "Session control API"))
)]
pub struct SessionApiDoc;

/// Export the session API OpenAPI document as YAML (contract diff-test input).
pub fn session_api_openapi_yaml() -> String {
    serde_yaml::to_string(&SessionApiDoc::openapi())
        .expect("session OpenAPI document serializes to YAML")
}

/// Export the session API OpenAPI document as JSON (alternate diff-test input).
pub fn session_api_openapi_json() -> String {
    serde_json::to_string_pretty(&SessionApiDoc::openapi())
        .expect("session OpenAPI document serializes to JSON")
}

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/status",
    operation_id = "getSessionStatus",
    summary = "Session status, ledger snapshot, linked runs",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses(
        (status = 200, description = "Session status", body = SessionStatus),
        (status = 404, description = "Unknown session", body = TypedError),
    ),
    tag = "session"
)]
fn doc_get_session_status(_session_id: String) -> SessionStatus {
    unreachable!("OpenAPI documentation stub")
}

#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/cancel",
    operation_id = "cancelSession",
    summary = "Cancel in-flight work for session",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses(
        (status = 200, description = "Cancel accepted"),
        (status = 409, description = "Nothing to cancel", body = TypedError),
    ),
    tag = "session"
)]
fn doc_cancel_session(_session_id: String) {}

#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/compact",
    operation_id = "compactSession",
    summary = "Compact session history per server policy",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses(
        (status = 200, description = "Compaction scheduled or complete"),
    ),
    tag = "session"
)]
fn doc_compact_session(_session_id: String) {}

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/events",
    operation_id = "listSessionEvents",
    summary = "List session-scoped events (paginated)",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses(
        (status = 200, description = "Event page"),
    ),
    tag = "session"
)]
fn doc_list_session_events(_session_id: String) {}

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/events/stream",
    operation_id = "streamSessionEvents",
    summary = "Resumable SSE for session events (Last-Event-ID)",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses(
        (status = 200, description = "text/event-stream"),
    ),
    tag = "session"
)]
fn doc_stream_session_events(_session_id: String) {}

#[utoipa::path(
    post,
    path = "/v1/permissions/{permission_id}/respond",
    operation_id = "respondPermission",
    summary = "Client reply to server permission prompt",
    params(("permission_id" = String, Path, description = "Permission prompt identifier")),
    request_body(content = PermissionResponse, description = "Client decision"),
    responses(
        (status = 200, description = "Response recorded"),
    ),
    tag = "session"
)]
fn doc_respond_permission(_permission_id: String, _body: PermissionResponse) {}

// ── Run-history API ───────────────────────────────────────────────

/// Lightweight run summary returned by the run-history index.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunRecordSchema {
    /// The execution identifier (also the run id in the history index).
    pub run_id: String,
    /// The workflow this run belongs to. `null` when the caller did not supply
    /// a `workflow_id` at submission time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    /// Skill id that executed the run, when available from the live record or
    /// reindexed run artifact.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    /// Skill version that executed the run, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_version: Option<String>,
    /// Session that submitted this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Session directory containing the execution snapshot, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_dir: Option<String>,
    /// Workflow run root containing `run.json` and per-node artifacts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_root: Option<String>,
    /// Cross-plane trace id, when the inbound route supplied one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Settled status: `"running"`, `"succeeded"`, or `"failed"`.
    pub status: String,
    /// Unix milliseconds when the execution was admitted.
    pub started_at: u64,
    /// Unix milliseconds when the execution settled. `null` while still running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// Wall-clock duration in milliseconds. `null` while still running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Input tokens attributed to this run by the execution result.
    pub input_tokens: u64,
    /// Output tokens attributed to this run by the execution result.
    pub output_tokens: u64,
    /// Input + output tokens for cheap workflow-level aggregation.
    pub total_tokens: u64,
    /// Optional cost estimate in USD. `null` until pricing attribution is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Retention policy class for the row and its artifact references.
    pub retention_class: String,
}

/// Envelope for `GET /v1/workflows/{id}/runs`.
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkflowRunsResponseSchema {
    /// The workflow id echoed from the path parameter.
    pub workflow_id: String,
    /// Run summaries, newest-first.
    pub runs: Vec<RunRecordSchema>,
}

/// One durable run artifact file under a run root.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunArtifactEntrySchema {
    /// POSIX-style path relative to the run root, for example `run.json` or
    /// `nodes/01_node/output.json`.
    pub path: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Best-effort media type derived from the file extension.
    pub media_type: String,
}

/// Envelope for `GET /v1/runs/{execution_id}/artifacts`.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunArtifactListSchema {
    /// Execution/run id echoed from the path parameter.
    pub execution_id: String,
    /// Absolute server-local run root. Operators should fetch files through the
    /// artifact endpoints instead of relying on direct volume access.
    pub run_root: String,
    /// Files materialized under the run root, newest schema first.
    pub artifacts: Vec<RunArtifactEntrySchema>,
}

/// OpenAPI document for the run-history surface.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "APXM Run-History API",
        version = "0.1.0-draft",
        description = "Workflow-scoped run index. Exposes list, single-run summary, \
                       and reindex endpoints. Full audit trail remains in rollout JSONL."
    ),
    paths(
        doc_list_workflow_runs,
        doc_get_run_summary,
        doc_list_run_artifacts,
        doc_get_run_artifact,
        doc_clear_runs,
        doc_reindex_runs,
    ),
    components(schemas(
        RunRecordSchema,
        WorkflowRunsResponseSchema,
        RunArtifactEntrySchema,
        RunArtifactListSchema,
        TypedError,
        FaultClass,
    )),
    tags((name = "run-history", description = "Workflow-scoped run history API"))
)]
pub struct RunHistoryApiDoc;

/// Export the run-history API OpenAPI document as YAML.
pub fn run_history_openapi_yaml() -> String {
    serde_yaml::to_string(&RunHistoryApiDoc::openapi())
        .expect("run-history OpenAPI document serializes to YAML")
}

#[utoipa::path(
    get,
    path = "/v1/workflows/{workflow_id}/runs",
    operation_id = "listWorkflowRuns",
    summary = "List run summaries for a workflow, newest-first",
    params(("workflow_id" = String, Path, description = "Workflow identifier")),
    responses(
        (status = 200, description = "Run list (empty when unknown)", body = WorkflowRunsResponseSchema),
    ),
    tag = "run-history"
)]
fn doc_list_workflow_runs(_workflow_id: String) -> WorkflowRunsResponseSchema {
    unreachable!("OpenAPI documentation stub")
}

#[utoipa::path(
    get,
    path = "/v1/runs/{execution_id}/summary",
    operation_id = "getRunSummary",
    summary = "Read one run summary by execution id",
    params(("execution_id" = String, Path, description = "Execution identifier")),
    responses(
        (status = 200, description = "Run summary", body = RunRecordSchema),
        (status = 404, description = "Unknown run", body = TypedError),
    ),
    tag = "run-history"
)]
fn doc_get_run_summary(_execution_id: String) -> RunRecordSchema {
    unreachable!("OpenAPI documentation stub")
}

#[utoipa::path(
    get,
    path = "/v1/runs/{execution_id}/artifacts",
    operation_id = "listRunArtifacts",
    summary = "List durable files under one run root",
    params(("execution_id" = String, Path, description = "Execution identifier")),
    responses(
        (status = 200, description = "Artifact list", body = RunArtifactListSchema),
        (status = 404, description = "Unknown run or missing artifact root", body = TypedError),
    ),
    tag = "run-history"
)]
fn doc_list_run_artifacts(_execution_id: String) -> RunArtifactListSchema {
    unreachable!("OpenAPI documentation stub")
}

#[utoipa::path(
    get,
    path = "/v1/runs/{execution_id}/artifacts/{artifact_path}",
    operation_id = "getRunArtifact",
    summary = "Fetch one durable run artifact by relative path",
    params(
        ("execution_id" = String, Path, description = "Execution identifier"),
        ("artifact_path" = String, Path, description = "Relative artifact path from listRunArtifacts"),
    ),
    responses(
        (status = 200, description = "Artifact bytes"),
        (status = 400, description = "Unsafe or invalid artifact path", body = TypedError),
        (status = 404, description = "Unknown run or artifact", body = TypedError),
    ),
    tag = "run-history"
)]
fn doc_get_run_artifact(_execution_id: String, _artifact_path: String) {
    unreachable!("OpenAPI documentation stub")
}

#[utoipa::path(
    post,
    path = "/v1/runs/clear",
    operation_id = "clearRuns",
    summary = "Hide settled runs from visible run lists while preserving durable artifacts",
    responses(
        (status = 200, description = "Clear complete"),
    ),
    tag = "run-history"
)]
fn doc_clear_runs() {
    unreachable!("OpenAPI documentation stub")
}

#[utoipa::path(
    post,
    path = "/v1/runs/reindex",
    operation_id = "reindexRuns",
    summary = "Rebuild the run-history index from durable run artifacts",
    responses(
        (status = 200, description = "Reindex complete"),
        (status = 501, description = "Not yet implemented"),
    ),
    tag = "run-history"
)]
fn doc_reindex_runs() {
    unreachable!("OpenAPI documentation stub")
}
