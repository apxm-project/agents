//! OpenAPI export for the session + permission surface.
//!
//! Wire shapes are derived from server types via `utoipa`; CI diff-tests the
//! export against `specs/0002-apxm-chat-thin-clients/contracts/openapi-session-v1.yaml`.

use utoipa::OpenApi;

use crate::permissions::{PermissionDecision, PermissionResponse};
use crate::sessions::{GrantUpdate, SessionLedgerView, SessionStatus};
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
        doc_update_session_grants,
        doc_compact_session,
        doc_list_session_events,
        doc_stream_session_events,
        doc_respond_permission,
    ),
    components(schemas(
        SessionStatus,
        SessionLedgerView,
        GrantUpdate,
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
    path = "/v1/sessions/{session_id}/grants",
    operation_id = "updateSessionGrants",
    summary = "Add or revoke capability grants",
    params(("session_id" = String, Path, description = "Session identifier")),
    request_body(content = GrantUpdate, description = "Grant delta"),
    responses(
        (status = 200, description = "Grants updated"),
    ),
    tag = "session"
)]
fn doc_update_session_grants(_session_id: String, _body: GrantUpdate) {}

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
