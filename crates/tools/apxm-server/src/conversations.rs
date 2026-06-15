//! Turn-input seam — the dumb-pipe contract (constitution #2, FR-002/FR-003).
//!
//! `POST /v1/conversations/{session_id}/message` delivers one user turn to the
//! parked recv node of that session's long-lived `/v1/execute/stream`
//! execution. It mirrors the checkpoint-resume route: it wakes the parked node
//! via the runtime park registry (wake-before-register safe), then the reply
//! streams over the session's already-open output — not this response. The host
//! adds no conversational behavior; it only delivers input and renders output.
//!
//! The session→execution registry below lets the endpoint find a session's
//! running execution. Driving one long-lived execution per session over a single
//! SSE is wired with US1/US2; this module is the substrate (registry + endpoint
//! + wake) the loop builds on.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use tracing::info;

use crate::error::ApiError;
use crate::state::AppState;

/// One conversation session's running execution.
#[derive(Clone, Debug)]
pub(crate) struct SessionRecord {
    pub(crate) execution_id: String,
}

/// `session_id` → running execution. Lets the turn-input endpoint find a
/// session's long-lived execution (and, for US1/US2, hold one execution per
/// session across parks).
#[derive(Clone, Default)]
pub(crate) struct SessionRegistry {
    inner: Arc<DashMap<String, SessionRecord>>,
}

impl SessionRegistry {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn register(&self, session_id: impl Into<String>, execution_id: impl Into<String>) {
        let session_id = session_id.into();
        self.inner.insert(
            session_id,
            SessionRecord {
                execution_id: execution_id.into(),
            },
        );
    }

    pub(crate) fn get(&self, session_id: &str) -> Option<SessionRecord> {
        self.inner.get(session_id).map(|r| r.clone())
    }

    /// Drop a session's record when its execution settles (avoids leaks). Only
    /// removes if the record still points at `execution_id` (a newer turn's
    /// execution may have re-registered the session).
    pub(crate) fn remove_if_execution(&self, session_id: &str, execution_id: &str) {
        self.inner
            .remove_if(session_id, |_, rec| rec.execution_id == execution_id);
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConversationMessageRequest {
    /// The user message text for this turn.
    pub(crate) message: String,
}

/// `POST /v1/conversations/{session_id}/message` — deliver one user turn.
pub(crate) async fn post_conversation_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<ConversationMessageRequest>,
) -> Result<(StatusCode, Json<JsonValue>), ApiError> {
    // Record the user message into session memory so the transcript is full
    // session memory (T052) — not just the assistant answer the conversation
    // middleware records. Ordered under `conversation:user:<n>` so a recency
    // window (`qmem recall_mode=recent`) can read the user side too. Best-effort.
    {
        let mem = state.runtime.memory();
        let count_key = "conversation:user_count";
        let n = mem
            .read_scoped(apxm_runtime::memory::MemorySpace::Stm, &session_id, count_key)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            + 1;
        let _ = mem
            .write_scoped(
                apxm_runtime::memory::MemorySpace::Stm,
                &session_id,
                count_key.to_string(),
                apxm_core::types::Value::Number(apxm_core::types::values::Number::Integer(n)),
            )
            .await;
        let _ = mem
            .write_scoped(
                apxm_runtime::memory::MemorySpace::Stm,
                &session_id,
                format!("conversation:user:{n}"),
                apxm_core::types::Value::String(req.message.clone()),
            )
            .await;
    }

    // Find this session's long-lived execution (registered on the stream path).
    let record = state.session_registry.get(&session_id);
    let execution_id = record.as_ref().map(|r| r.execution_id.clone());
    let known = record.is_some();

    let wait_key = apxm_runtime::scheduler::park_registry::session_recv_key(&session_id);
    let value = apxm_core::types::Value::String(req.message);
    let woken = apxm_runtime::scheduler::park_registry::wake(&wait_key, value);
    info!(
        session_id = %session_id,
        woken,
        known,
        execution_id = execution_id.as_deref().unwrap_or(""),
        "delivered turn input to parked recv node"
    );
    // 202 Accepted: the reply streams over the session's open output, not here.
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "ok": true,
            "session_id": session_id,
            "execution_id": execution_id,
            "known_session": known,
            "woken": woken,
        })),
    ))
}
