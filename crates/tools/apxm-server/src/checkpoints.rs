use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::info;

use crate::error::ApiError;
use crate::helpers::now_ms;
use crate::routes;
use crate::state::AppState;
use crate::types::responses::{
    CheckpointCreatedResponse, CheckpointResumedResponse, CheckpointWebhookPayload,
};

/// Status of a PAUSE/RESUME checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CheckpointStatus {
    Pending,
    Resumed,
    Expired,
}

/// A PAUSE checkpoint waiting for human input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Checkpoint {
    pub(crate) id: String,
    pub(crate) message: String,
    pub(crate) display_data: JsonValue,
    pub(crate) status: CheckpointStatus,
    #[serde(default)]
    pub(crate) human_input: Option<JsonValue>,
    #[serde(default)]
    pub(crate) notification_url: Option<String>,
    pub(crate) created_at_ms: u64,
    #[serde(default)]
    pub(crate) resumed_at_ms: Option<u64>,
}

/// In-memory checkpoint store for PAUSE/RESUME HITL workflow.
#[derive(Clone)]
pub(crate) struct CheckpointStore {
    inner: Arc<DashMap<String, Checkpoint>>,
}

impl CheckpointStore {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn create(&self, checkpoint: Checkpoint) {
        self.inner.insert(checkpoint.id.clone(), checkpoint);
    }

    pub(crate) fn get(&self, id: &str) -> Option<Checkpoint> {
        self.inner.get(id).map(|c| c.clone())
    }

    pub(crate) fn resume(&self, id: &str, human_input: JsonValue) -> Result<Checkpoint, String> {
        let mut entry = self
            .inner
            .get_mut(id)
            .ok_or_else(|| format!("Checkpoint '{}' not found", id))?;
        if entry.status != CheckpointStatus::Pending {
            return Err(format!("Checkpoint '{}' is not in pending state", id));
        }
        entry.status = CheckpointStatus::Resumed;
        entry.human_input = Some(human_input);
        entry.resumed_at_ms = Some(now_ms());
        Ok(entry.clone())
    }
}

// ─── Checkpoint Handlers (Plan 07 — PAUSE/RESUME HITL) ───────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct CreateCheckpointRequest {
    checkpoint_id: String,
    message: String,
    #[serde(default)]
    display_data: JsonValue,
    /// Optional webhook URL — server POSTs a notification when checkpoint is created.
    #[serde(default)]
    notification_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResumeCheckpointRequest {
    human_input: JsonValue,
}

pub(crate) async fn create_checkpoint(
    State(state): State<AppState>,
    Json(req): Json<CreateCheckpointRequest>,
) -> Json<CheckpointCreatedResponse> {
    let id = req.checkpoint_id.clone();
    let notification_url = req.notification_url.clone();
    let checkpoint = Checkpoint {
        id: id.clone(),
        message: req.message.clone(),
        display_data: req.display_data,
        status: CheckpointStatus::Pending,
        human_input: None,
        notification_url: notification_url.clone(),
        created_at_ms: now_ms(),
        resumed_at_ms: None,
    };
    state.checkpoint_store.create(checkpoint);
    info!(id = %id, "Checkpoint created — awaiting human input");

    // Fire-and-forget webhook notification
    if let Some(notify_url) = notification_url {
        let payload = CheckpointWebhookPayload {
            kind: "checkpoint_created",
            checkpoint_id: id.clone(),
            message: req.message,
            review_url: routes::checkpoint_detail_path(&id),
        };
        tokio::spawn(async move {
            if let Ok(client) = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
            {
                let _ = client.post(&notify_url).json(&payload).send().await;
            }
        });
    }

    Json(CheckpointCreatedResponse {
        ok: true,
        checkpoint_id: id.clone(),
        status: apxm_core::types::SessionStatus::Pending,
        resume_url: routes::checkpoint_resume_path(&id),
    })
}

pub(crate) async fn get_checkpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JsonValue>, ApiError> {
    match state.checkpoint_store.get(&id) {
        Some(cp) => Ok(Json(serde_json::to_value(cp).unwrap_or_default())),
        None => Err(ApiError {
            status: axum::http::StatusCode::NOT_FOUND,
            message: format!("Checkpoint '{}' not found", id),
        }),
    }
}

pub(crate) async fn resume_checkpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ResumeCheckpointRequest>,
) -> Result<Json<CheckpointResumedResponse>, ApiError> {
    let cp = state
        .checkpoint_store
        .resume(&id, req.human_input)
        .map_err(|e| ApiError {
            status: axum::http::StatusCode::BAD_REQUEST,
            message: e,
        })?;
    info!(id = %id, "Checkpoint resumed with human input");
    Ok(Json(CheckpointResumedResponse {
        ok: true,
        checkpoint_id: cp.id,
        status: apxm_core::types::SessionStatus::Resumed,
        human_input: cp.human_input,
        resumed_at_ms: cp.resumed_at_ms,
    }))
}
