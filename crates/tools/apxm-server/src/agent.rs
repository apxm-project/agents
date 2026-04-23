use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::info;

use crate::error::ApiError;
use crate::helpers::now_ms;
use crate::state::AppState;

/// A remote agent registered with this server.
///
/// Agents register themselves so that COMMUNICATE operations (and the A2A agent
/// card) can resolve a name → URL mapping at runtime.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AgentRegistration {
    /// Unique agent name (matches the name used in AIS `communicate "AgentName"`)
    pub(crate) name: String,
    /// Base URL of the agent's apxm-server (e.g. "http://localhost:18801")
    pub(crate) url: String,
    /// Flow names this agent exposes
    #[serde(default)]
    pub(crate) flows: Vec<String>,
    /// Capability names this agent advertises
    #[serde(default)]
    pub(crate) capabilities: Vec<String>,
    /// Unix millisecond timestamp of when the agent registered
    pub(crate) registered_at: u64,
}

// ─── Receive Message (HTTP COMMUNICATE target) ─────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ReceiveMessageRequest {
    from: String,
    message: JsonValue,
    #[serde(default)]
    channel: Option<String>,
}

pub(crate) async fn receive_message(
    State(state): State<AppState>,
    Json(req): Json<ReceiveMessageRequest>,
) -> Result<Json<JsonValue>, ApiError> {
    let text = req.message.to_string();
    let source = req.from.clone();
    let tags = req
        .channel
        .as_deref()
        .map(|c| vec![format!("channel:{}", c)])
        .unwrap_or_default();
    let id = state
        .runtime
        .memory()
        .store_fact(&text, &tags, &source, None)
        .await
        .map_err(ApiError::runtime)?;
    info!(from = %req.from, id = %id, "Received inter-agent message");
    Ok(Json(serde_json::json!({ "ok": true, "id": id })))
}

// ─── Agent Registry Handlers ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct RegisterAgentRequest {
    name: String,
    url: String,
    #[serde(default)]
    flows: Vec<String>,
    #[serde(default)]
    capabilities: Vec<String>,
}

pub(crate) async fn register_agent(
    State(state): State<AppState>,
    Json(req): Json<RegisterAgentRequest>,
) -> Result<Json<JsonValue>, ApiError> {
    let reg = AgentRegistration {
        name: req.name.clone(),
        url: req.url,
        flows: req.flows,
        capabilities: req.capabilities,
        registered_at: now_ms(),
    };
    info!(name = %req.name, "Registering agent");
    state.agent_registry.insert(req.name.clone(), reg);
    Ok(Json(serde_json::json!({ "ok": true, "name": req.name })))
}

pub(crate) async fn list_agents(State(state): State<AppState>) -> Json<JsonValue> {
    let agents: Vec<JsonValue> = state
        .agent_registry
        .iter()
        .map(|e| serde_json::to_value(e.value()).unwrap_or(JsonValue::Null))
        .collect();
    Json(JsonValue::Array(agents))
}

pub(crate) async fn get_agent(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<JsonValue>, ApiError> {
    match state.agent_registry.get(&name) {
        Some(entry) => Ok(Json(
            serde_json::to_value(entry.value())
                .map_err(|e| ApiError::internal_message(e.to_string()))?,
        )),
        None => Err(ApiError::not_found(format!(
            "Agent '{}' not registered",
            name
        ))),
    }
}

pub(crate) async fn deregister_agent(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<JsonValue>, ApiError> {
    if state.agent_registry.remove(&name).is_some() {
        info!(%name, "Deregistered agent");
        Ok(Json(serde_json::json!({ "ok": true })))
    } else {
        Err(ApiError::not_found(format!(
            "Agent '{}' not registered",
            name
        )))
    }
}

// ─── A2A AgentCard ───────────────────────────────────────────────────────────

pub(crate) async fn agent_card(State(state): State<AppState>) -> Json<JsonValue> {
    let base_url =
        std::env::var("APXM_PUBLIC_URL").unwrap_or_else(|_| crate::DEFAULT_PUBLIC_URL.to_string());

    let skills: Vec<JsonValue> = state
        .agent_registry
        .iter()
        .flat_map(|entry| {
            let agent = entry.value();
            agent
                .flows
                .iter()
                .map(|flow| {
                    serde_json::json!({
                        "id": format!("{}.{}", agent.name, flow),
                        "name": format!("{}/{}", agent.name, flow),
                        "description": format!("Execute {} flow on agent {}", flow, agent.name),
                        "inputModes": ["text"],
                        "outputModes": ["text"],
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();

    Json(serde_json::json!({
        "protocolVersion": apxm_core::constants::protocols::A2A_VERSION,
        "name": "APXM Agent Runtime",
        "description": "Program Execution Model for AI agents — parallel dataflow, multi-model councils, formal agent programs.",
        "version": env!("CARGO_PKG_VERSION"),
        "url": base_url,
        "capabilities": {
            "streaming": true,
            "pushNotifications": false,
            "stateTransitionHistory": true
        },
        "defaultInputModes": ["text"],
        "defaultOutputModes": ["text"],
        "skills": skills,
        "authentication": {
            "schemes": ["bearer"],
            "credentials": null
        }
    }))
}
