//! Agent chat endpoint — bridges ACP subprocess sessions to SSE.
//!
//! Spawns an agent subprocess (ACP) on the first chat message,
//! then streams its responses as Server-Sent Events. Subsequent messages reuse
//! the same ACP session. Sessions are stored in `AppState::agent_sessions`.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use apxm_core::events::payload::EventPayload;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use futures::stream::Stream;
use serde::Deserialize;
use tokio::sync::{Mutex, mpsc};
use tracing::{error, info};

use apxm_acp::registry::AgentRegistry;

use crate::acp_client::AgentSession;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default command to spawn the agent subprocess.
const DEFAULT_AGENT_COMMAND: &str = "claude-agent-acp-wrapper";

/// SSE heartbeat interval for agent chat streams.
const AGENT_SSE_HEARTBEAT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AgentChatRequest {
    /// Existing session ID to reuse, or `None` to create a new session.
    pub session_id: Option<String>,
    /// The user message to send to the agent.
    pub message: String,
    /// Optional command override (default: built-in wrapper or registry command).
    pub command: Option<String>,
    /// Agent profile ID (e.g. "claude", "codex").
    /// Resolved via `AgentRegistry` to get the command.
    pub agent_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/agent/chat` — start or continue an agent chat session via SSE.
///
/// If `session_id` is provided, the existing session is reused. Otherwise a new
/// agent subprocess is spawned and registered. The response is an SSE stream of
/// agent events (tokens, tool calls, usage, done/error).
pub async fn agent_chat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AgentChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<serde_json::Value>)>
{
    // Resolve command: explicit command > agent_id via registry > default
    let command = if let Some(ref cmd) = req.command {
        cmd.clone()
    } else if let Some(ref agent_id) = req.agent_id {
        let registry = AgentRegistry::load();
        registry
            .get(agent_id)
            .map(|p| p.command.clone())
            .unwrap_or_else(|| DEFAULT_AGENT_COMMAND.to_string())
    } else {
        DEFAULT_AGENT_COMMAND.to_string()
    };
    let message = req.message.clone();

    // Resolve or create a session.
    let session = if let Some(ref id) = req.session_id {
        // Look up existing session.
        let entry = state.agent_sessions.get(id).map(|e| e.value().clone());
        match entry {
            Some(s) => s,
            None => {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": format!("agent session not found: {id}")
                    })),
                ));
            }
        }
    } else {
        // Spawn a new session.
        let cwd = std::env::current_dir().unwrap_or_default();
        let session = AgentSession::spawn(&command, &cwd).await.map_err(|e| {
            error!(error = %e, "failed to spawn agent session");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
        let id = session.session_id().to_string();
        let session = Arc::new(Mutex::new(session));
        state.agent_sessions.insert(id.clone(), session.clone());
        info!(session_id = %id, command = %command, "new agent session spawned");
        session
    };

    // Create the SSE stream — prompt happens inside the stream so we can
    // start sending events immediately.
    let stream = async_stream::stream! {
        let (tx, mut rx) = mpsc::channel::<Arc<dyn EventPayload>>(128);

        // Spawn the prompt in a background task so we can yield events as
        // they arrive on the channel.
        let session_clone = session.clone();
        let message_clone = message.clone();
        let tx_clone = tx.clone();

        tokio::spawn(async move {
            let mut guard = session_clone.lock().await;
            if let Err(e) = guard.prompt(&message_clone, tx_clone).await {
                error!(error = %e, "agent prompt failed");
                // tx is dropped here, which will cause rx.recv() to return None.
            }
        });

        // Drop our copy of tx so the channel closes when the background
        // task finishes.
        drop(tx);

        while let Some(payload) = rx.recv().await {
            let kind = payload.event_kind();
            let data = payload_json_with_kind(payload.as_ref());

            yield Ok::<Event, Infallible>(
                Event::default()
                    .event(kind.sse_event_type())
                    .data(data.to_string())
            );

            if kind.is_terminal() {
                break;
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(AGENT_SSE_HEARTBEAT)
            .text("heartbeat"),
    ))
}

fn payload_json_with_kind(payload: &dyn EventPayload) -> serde_json::Value {
    let mut json = payload.to_json();
    if let Some(obj) = json.as_object_mut() {
        obj.insert("kind".into(), payload.event_kind().name().into());
    } else {
        json = serde_json::json!({
            "kind": payload.event_kind().name(),
            "value": json,
        });
    }
    json
}

/// `GET /api/agent/sessions` — list active agent sessions.
pub async fn list_agent_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let sessions: Vec<serde_json::Value> = state
        .agent_sessions
        .iter()
        .map(|entry| {
            serde_json::json!({
                "session_id": entry.key().clone(),
            })
        })
        .collect();

    Json(serde_json::json!({ "sessions": sessions }))
}

/// `GET /api/agent/profiles` — list available ACP agent profiles.
///
/// Returns the full agent registry (built-in templates + user overrides),
/// with an `available` flag indicating whether the agent's command binary
/// is found on `$PATH`. All binary checks run in parallel.
pub async fn list_agent_profiles() -> impl IntoResponse {
    let registry = AgentRegistry::load();
    let entries: Vec<_> = registry
        .list()
        .into_iter()
        .map(|(name, profile, from_template)| {
            let bin = profile
                .command
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            let command = profile.command.clone();
            let source = if from_template { "template" } else { "custom" };
            (name, command, bin, source.to_string())
        })
        .collect();

    // Check all binaries in parallel (async, non-blocking)
    let checks: Vec<_> = entries
        .iter()
        .map(|(_, _, bin, _)| async move {
            tokio::process::Command::new("which")
                .arg(bin)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .collect();

    let results = futures::future::join_all(checks).await;

    let profiles: Vec<serde_json::Value> = entries
        .into_iter()
        .zip(results)
        .map(|((name, command, _, source), available)| {
            serde_json::json!({
                "id": name,
                "command": command,
                "available": available,
                "source": source,
            })
        })
        .collect();
    Json(serde_json::json!({ "profiles": profiles }))
}

/// `DELETE /api/agent/sessions/{id}` — close and remove an agent session.
pub async fn delete_agent_session(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    match state.agent_sessions.remove(&id) {
        Some((_, session)) => {
            // Take ownership via Arc and close gracefully.
            let session = match Arc::try_unwrap(session) {
                Ok(mutex) => Some(mutex.into_inner()),
                Err(arc) => {
                    // Another task still holds a reference — just force-lock and drop.
                    let guard = arc.lock().await;
                    drop(guard);
                    None
                }
            };
            if let Some(s) = session {
                s.close().await;
            }
            info!(session_id = %id, "agent session deleted");
            StatusCode::NO_CONTENT
        }
        None => StatusCode::NOT_FOUND,
    }
}
