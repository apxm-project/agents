use axum::Json;
use axum::extract::State;
use serde_json::Value as JsonValue;

use crate::state::AppState;

pub(crate) async fn health(State(state): State<AppState>) -> Json<JsonValue> {
    let uptime_secs = state.start_time.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime_secs,
    }))
}

// ─── Model Discovery (/v1/models) ────────────────────────────────────────────

/// OpenAI-compatible `/v1/models` endpoint.
///
/// Returns a list of LLM backends registered with the runtime so clients
/// (AgentMate, OpenAI SDK, etc.) can discover which models are available.
pub(crate) async fn list_models(State(state): State<AppState>) -> Json<JsonValue> {
    // The APXM runtime exposes registered backend names via the LLM registry.
    // We surface them in the OpenAI models format for compatibility.
    let models: Vec<JsonValue> = state
        .runtime
        .llm_registry()
        .backend_names()
        .into_iter()
        .map(|name| {
            serde_json::json!({
                "id": name,
                "object": "model",
                "created": 0,
                "owned_by": "apxm",
            })
        })
        .collect();
    Json(serde_json::json!({
        "object": "list",
        "data": models,
    }))
}
