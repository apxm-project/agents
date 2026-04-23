use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct StoreFactRequest {
    text: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    source: String,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct StoreFactResponse {
    id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchFactsRequest {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

pub(crate) fn default_limit() -> usize {
    5
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteFactRequest {
    id: String,
}

pub(crate) async fn store_fact(
    State(state): State<AppState>,
    Json(req): Json<StoreFactRequest>,
) -> Result<Json<StoreFactResponse>, ApiError> {
    let id = state
        .runtime
        .memory()
        .store_fact(&req.text, &req.tags, &req.source, req.session_id)
        .await
        .map_err(ApiError::runtime)?;
    Ok(Json(StoreFactResponse { id }))
}

pub(crate) async fn search_facts(
    State(state): State<AppState>,
    Json(req): Json<SearchFactsRequest>,
) -> Result<Json<JsonValue>, ApiError> {
    let results = state
        .runtime
        .memory()
        .search_facts(&req.query, req.limit)
        .await
        .map_err(ApiError::runtime)?;
    Ok(Json(
        serde_json::to_value(results).map_err(|e| ApiError::internal_message(e.to_string()))?,
    ))
}

pub(crate) async fn delete_fact(
    State(state): State<AppState>,
    Json(req): Json<DeleteFactRequest>,
) -> Result<Json<JsonValue>, ApiError> {
    state
        .runtime
        .memory()
        .delete_fact(&req.id)
        .await
        .map_err(ApiError::runtime)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
