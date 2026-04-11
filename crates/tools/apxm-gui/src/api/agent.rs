//! Agent chat endpoint — bridges ACP subprocess sessions to SSE.
//!
//! Spawns an agent subprocess (e.g. `openclaw acp`) on the first chat message,
//! then streams its responses as Server-Sent Events. Subsequent messages reuse
//! the same ACP session. Sessions are stored in `AppState::agent_sessions`.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use futures::stream::Stream;
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info};

use crate::acp_client::{AgentEvent, AgentSession};
use crate::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default command to spawn the agent subprocess.
const DEFAULT_AGENT_COMMAND: &str = "openclaw acp";

/// SSE heartbeat interval for agent chat streams.
const AGENT_SSE_HEARTBEAT: Duration = Duration::from_secs(10);

/// SSE event type names for the agent chat stream.
mod sse_event {
    pub const TOKEN: &str = "token";
    pub const TOOL_CALL: &str = "tool_call";
    pub const TOOL_RESULT: &str = "tool_result";
    pub const USAGE: &str = "usage";
    pub const DONE: &str = "done";
    pub const ERROR: &str = "error";
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AgentChatRequest {
    /// Existing session ID to reuse, or `None` to create a new session.
    pub session_id: Option<String>,
    /// The user message to send to the agent.
    pub message: String,
    /// Optional command override (default: `"openclaw acp"`).
    pub command: Option<String>,
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
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<serde_json::Value>)> {
    let command = req
        .command
        .as_deref()
        .unwrap_or(DEFAULT_AGENT_COMMAND)
        .to_string();
    let message = req.message.clone();

    // Resolve or create a session.
    let (session_id, session) = if let Some(ref id) = req.session_id {
        // Look up existing session.
        let entry = state.agent_sessions.get(id).map(|e| e.value().clone());
        match entry {
            Some(s) => (id.clone(), s),
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
        (id, session)
    };

    let session_id_for_stream = session_id.clone();

    // Create the SSE stream — prompt happens inside the stream so we can
    // start sending events immediately.
    let stream = async_stream::stream! {
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(128);

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

        while let Some(event) = rx.recv().await {
            match event {
                AgentEvent::Token(text) => {
                    yield Ok::<Event, Infallible>(
                        Event::default()
                            .event(sse_event::TOKEN)
                            .data(serde_json::json!({ "token": text }).to_string())
                    );
                }
                AgentEvent::ToolCall { id, name, args } => {
                    yield Ok(
                        Event::default()
                            .event(sse_event::TOOL_CALL)
                            .data(serde_json::json!({
                                "id": id,
                                "name": name,
                                "arguments": args,
                            }).to_string())
                    );
                }
                AgentEvent::ToolResult { id, success, output } => {
                    yield Ok(
                        Event::default()
                            .event(sse_event::TOOL_RESULT)
                            .data(serde_json::json!({
                                "id": id,
                                "success": success,
                                "output": output,
                            }).to_string())
                    );
                }
                AgentEvent::Usage { input_tokens, output_tokens } => {
                    yield Ok(
                        Event::default()
                            .event(sse_event::USAGE)
                            .data(serde_json::json!({
                                "inputTokens": input_tokens,
                                "outputTokens": output_tokens,
                            }).to_string())
                    );
                }
                AgentEvent::Done { stop_reason } => {
                    yield Ok(
                        Event::default()
                            .event(sse_event::DONE)
                            .data(serde_json::json!({
                                "stopReason": stop_reason,
                                "sessionId": session_id_for_stream,
                            }).to_string())
                    );
                    break;
                }
                AgentEvent::Error(msg) => {
                    yield Ok(
                        Event::default()
                            .event(sse_event::ERROR)
                            .data(serde_json::json!({ "error": msg }).to_string())
                    );
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(AGENT_SSE_HEARTBEAT)
            .text("heartbeat"),
    ))
}

/// `GET /api/agent/sessions` — list active agent sessions.
pub async fn list_agent_sessions(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
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
