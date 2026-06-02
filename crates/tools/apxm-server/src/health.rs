use axum::Json;
use axum::extract::State;

use crate::state::AppState;
use crate::types::responses::{
    BackendEntry, BackendList, BackendModelEntry, HealthResponse, ModelEntry, ModelList,
};

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

/// `GET /v1/backends` — the live backend registry with the model ids each
/// backend serves, projected to non-secret fields.
///
/// Sourced from the same [`apxm_credentials::BackendStore`] the runtime loads
/// its backends from, so a client that compiles graphs (apxm-studio) can pin
/// the exact backends the runtime will dispatch to — no duplicate, drifting
/// registry. Credentials (`api_key`, `headers`, `endpoint`) are deliberately
/// never serialized.
pub(crate) async fn list_backends(State(_state): State<AppState>) -> Json<BackendList> {
    let data = match apxm_credentials::BackendStore::open().and_then(|store| store.list()) {
        Ok(backends) => backends
            .into_iter()
            .map(|backend| BackendEntry {
                name: backend.name,
                protocol: backend.protocol.as_str().to_string(),
                models: backend
                    .models
                    .into_iter()
                    .map(|model| BackendModelEntry { id: model.id })
                    .collect(),
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "failed to read backend store for /v1/backends");
            Vec::new()
        }
    };
    Json(BackendList {
        object: "list",
        data,
    })
}
