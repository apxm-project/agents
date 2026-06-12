use std::sync::Arc;

use apxm_runtime::capability::builtins::guard_url_ssrf;
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

/// Durable store for PAUSE/RESUME HITL checkpoints.
///
/// The in-memory `DashMap` is the hot path; an optional SQLite backing makes a
/// checkpoint's state (and its resumed `human_input`) survive a server restart.
/// Waking a parked PAUSE/RESUME node is NOT this store's job — that goes through
/// the runtime's `park_registry` (the server's resume handler fires it). This
/// store just records state.
#[derive(Clone)]
pub(crate) struct CheckpointStore {
    inner: Arc<DashMap<String, Checkpoint>>,
    /// Optional durable backing (SQLite). `None` = volatile (tests). When set,
    /// every create/resume is persisted; `open` boot-loads rows into the cache.
    db: Option<Arc<std::sync::Mutex<rusqlite::Connection>>>,
}

impl CheckpointStore {
    /// Volatile in-memory store (tests, and the fallback when durability can't be
    /// opened). State is lost on restart.
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            db: None,
        }
    }

    /// Durable store backed by a SQLite file. Creates the table if needed and
    /// boot-loads existing checkpoints into the cache so a checkpoint (and any
    /// resumed input) survives a server restart. Only the `json` blob is stored;
    /// it is the full `Checkpoint` (status/timestamps included).
    pub(crate) fn open(path: &std::path::Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS checkpoints (id TEXT PRIMARY KEY, json TEXT NOT NULL);",
        )
        .map_err(|e| e.to_string())?;

        let inner: DashMap<String, Checkpoint> = DashMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT json FROM checkpoints")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            for js in rows.flatten() {
                if let Ok(cp) = serde_json::from_str::<Checkpoint>(&js) {
                    inner.insert(cp.id.clone(), cp);
                }
            }
        }
        let recovered = inner.len();
        if recovered > 0 {
            info!(count = recovered, "Recovered durable checkpoints from disk");
        }
        Ok(Self {
            inner: Arc::new(inner),
            db: Some(Arc::new(std::sync::Mutex::new(conn))),
        })
    }

    /// Persist a checkpoint row (idempotent upsert of the full JSON). No-op in
    /// volatile mode.
    fn persist(&self, cp: &Checkpoint) {
        let Some(db) = &self.db else { return };
        let Ok(json) = serde_json::to_string(cp) else {
            return;
        };
        if let Ok(conn) = db.lock() {
            let _ = conn.execute(
                "INSERT INTO checkpoints (id, json) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json",
                rusqlite::params![cp.id, json],
            );
        }
    }

    pub(crate) fn create(&self, checkpoint: Checkpoint) {
        self.persist(&checkpoint);
        self.inner.insert(checkpoint.id.clone(), checkpoint);
    }

    pub(crate) fn get(&self, id: &str) -> Option<Checkpoint> {
        self.inner.get(id).map(|c| c.clone())
    }

    pub(crate) fn resume(&self, id: &str, human_input: JsonValue) -> Result<Checkpoint, String> {
        let cp = {
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
            entry.clone()
        };
        // Durably record the resume (human_input) before the caller wakes any
        // parked node, so a crash in between still has the resolved input on
        // restart. The wake itself is driven by the resume handler via
        // `park_registry`, not this store.
        self.persist(&cp);
        Ok(cp)
    }
}

// ─── Checkpoint Handlers (PAUSE/RESUME HITL) ─────────────────────────────────

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
) -> Result<Json<CheckpointCreatedResponse>, ApiError> {
    let id = req.checkpoint_id.clone();
    let notification_url = req.notification_url.clone();
    if let Some(notify_url) = notification_url.as_deref() {
        guard_url_ssrf("checkpoint.notification_url", notify_url)
            .await
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
    }
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
                .redirect(reqwest::redirect::Policy::none())
                .build()
            {
                let _ = client.post(&notify_url).json(&payload).send().await;
            }
        });
    }

    Ok(Json(CheckpointCreatedResponse {
        ok: true,
        checkpoint_id: id.clone(),
        status: apxm_core::types::SessionStatus::Pending,
        resume_url: routes::checkpoint_resume_path(&id),
    }))
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

    // Push-wake any in-process PAUSE/RESUME node parked on this checkpoint: it
    // yielded its worker and is re-injected here with the human_input as its
    // output. The runtime runs in-process with the server, so this is the wake path.
    if let Some(value) = cp
        .human_input
        .clone()
        .and_then(|hi| apxm_core::types::Value::try_from(hi).ok())
    {
        let woken = apxm_runtime::scheduler::park_registry::wake(&cp.id, value);
        if woken > 0 {
            info!(id = %cp.id, woken, "woke parked node(s) via park registry");
        }
    }
    Ok(Json(CheckpointResumedResponse {
        ok: true,
        checkpoint_id: cp.id,
        status: apxm_core::types::SessionStatus::Resumed,
        human_input: cp.human_input,
        resumed_at_ms: cp.resumed_at_ms,
    }))
}

