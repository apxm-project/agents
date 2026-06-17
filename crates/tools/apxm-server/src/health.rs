use axum::Json;
use axum::extract::State;
use std::collections::HashSet;

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
/// Sourced from the credential store plus the live runtime registry, so clients
/// can see env-provided dev backends as well as stored provider credentials.
/// Credentials (`api_key`, `headers`, `endpoint`) are deliberately never
/// serialized.
pub(crate) async fn list_backends(State(state): State<AppState>) -> Json<BackendList> {
    let mut seen = HashSet::new();
    let mut data: Vec<BackendEntry> =
        match apxm_credentials::BackendStore::open().and_then(|store| store.list()) {
            Ok(backends) => backends
                .into_iter()
                .map(|backend| BackendEntry {
                    name: backend.name.clone(),
                    protocol: backend.protocol.as_str().to_string(),
                    models: backend
                        .models
                        .into_iter()
                        .map(|model| BackendModelEntry { id: model.id })
                        .collect(),
                })
                .inspect(|backend| {
                    seen.insert(backend.name.clone());
                })
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "failed to read backend store for /v1/backends");
                Vec::new()
            }
        };
    for name in state.runtime.llm_registry().backend_names() {
        if seen.insert(name.clone()) {
            data.push(BackendEntry {
                protocol: if name == "mock" {
                    "mock".to_string()
                } else {
                    "runtime".to_string()
                },
                models: vec![BackendModelEntry { id: name.clone() }],
                name,
            });
        }
    }
    Json(BackendList {
        object: "list",
        data,
    })
}
