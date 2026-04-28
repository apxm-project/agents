use axum::Json;
use axum::extract::State;

use crate::state::AppState;
use crate::types::responses::{HealthResponse, ModelEntry, ModelList};

pub(crate) async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let uptime_secs = state.start_time.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs,
    })
}

/// OpenAI-compatible `/v1/models` endpoint.
///
/// Returns a list of LLM backends registered with the runtime so clients
/// (AgentMate, OpenAI SDK, etc.) can discover which models are available.
pub(crate) async fn list_models(State(state): State<AppState>) -> Json<ModelList> {
    let models: Vec<ModelEntry> = state
        .runtime
        .llm_registry()
        .backend_names()
        .into_iter()
        .map(|name| ModelEntry {
            id: name,
            object: "model",
            created: 0,
            owned_by: "apxm",
        })
        .collect();
    Json(ModelList {
        object: "list",
        data: models,
    })
}
