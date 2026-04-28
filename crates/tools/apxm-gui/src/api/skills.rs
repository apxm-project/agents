//! Skills endpoints (currently empty surface).

use axum::extract::Path;
use axum::response::Json;

use crate::error::{ApiResult, AppError};

/// GET /api/skills
pub async fn skills_handler() -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "skills": [] })))
}

/// GET /api/skills/{name}
pub async fn skill_detail_handler(
    Path(name): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(AppError::not_found(format!("skill '{name}' not found")))
}
