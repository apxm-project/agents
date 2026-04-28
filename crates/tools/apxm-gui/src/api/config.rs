//! `/api/config` and `/api/config/update` endpoints.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use tracing::info;

use crate::error::{ApiResult, AppError};
use crate::paths::config_path;

/// GET /api/config
pub async fn config_handler() -> ApiResult<impl IntoResponse> {
    let path = config_path();

    if !path.exists() {
        return Err(AppError::not_found("~/.apxm/config.toml not found"));
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| AppError::internal(format!("failed to read config: {e}")))?;

    Ok((
        StatusCode::OK,
        [("content-type", "text/plain; charset=utf-8")],
        content,
    ))
}

/// POST /api/config/update
pub async fn config_update_handler(
    Json(payload): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let path = config_path();

    if let Some(content) = payload.get("content").and_then(|v| v.as_str()) {
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| AppError::internal(format!("failed to write config: {e}")))?;
        info!("Config updated: {}", path.display());
        return Ok(Json(serde_json::json!({ "success": true })));
    }

    Err(AppError::bad_request("missing 'content' field"))
}
