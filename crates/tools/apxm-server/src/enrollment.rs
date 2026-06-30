//! Host enrollment API stubs.
//!
//! These routes are declared here to make the API surface visible. The real
//! implementations live in the `auth` repo. When the auth service is available,
//! these stubs are replaced by proxy handlers or removed in favor of auth-side routing.

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

/// POST /v1/hosts/enroll — initiate host enrollment, returns an enrollment token.
/// Owned by auth. This stub returns 501 until the auth integration is wired.
pub(crate) async fn enroll_host(
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Err(ApiError::not_implemented(
        "host enrollment is handled by the auth service; \
         configure APXM_AUTH_URL and the auth middleware will proxy this request",
    ))
}

/// GET /v1/hosts/{host_id} — get host registration record.
/// Owned by auth.
pub(crate) async fn get_host(
    State(_state): State<AppState>,
    axum::extract::Path(_host_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Err(ApiError::not_implemented(
        "host record lookup is handled by the auth service",
    ))
}

/// DELETE /v1/hosts/{host_id} — revoke host enrollment.
/// Owned by auth.
pub(crate) async fn revoke_host(
    State(_state): State<AppState>,
    axum::extract::Path(_host_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Err(ApiError::not_implemented(
        "host revocation is handled by the auth service",
    ))
}

/// POST /v1/hosts/{host_id}/conformance — submit a conformance report.
/// Accepted by the server, stored for Studio consumption.
#[derive(Debug, Deserialize)]
pub(crate) struct SubmitConformanceReportRequest {
    pub report: apxm_core::types::conformance::ConformanceReport,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubmitConformanceReportResponse {
    pub accepted: bool,
    pub report_id: String,
}

pub(crate) async fn submit_conformance_report(
    State(_state): State<AppState>,
    axum::extract::Path(_host_id): axum::extract::Path<String>,
    Json(body): Json<SubmitConformanceReportRequest>,
) -> Result<Json<SubmitConformanceReportResponse>, ApiError> {
    // Validate internal consistency before accepting.
    let harness = apxm_core::types::conformance::ConformanceHarness::new();
    if !harness.verify_report(&body.report) {
        return Err(ApiError::bad_request(
            "conformance report is internally inconsistent \
             (pass field does not match vector results)",
        ));
    }
    // TODO(band-f): persist to conformance store; for now accept and return a stub id.
    let report_id = format!("conf-{}-stub", body.report.adapter_id);
    Ok(Json(SubmitConformanceReportResponse {
        accepted: true,
        report_id,
    }))
}

/// GET /v1/adapters — list registered adapter profiles.
/// Owned by studio catalog. Stub returns empty list.
pub(crate) async fn list_adapters(
    State(_state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!({
        "adapters": [],
        "note": "adapter catalog is managed by the studio service; this stub returns an empty list"
    })))
}
