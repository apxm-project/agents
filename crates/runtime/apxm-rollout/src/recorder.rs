//! [`RolloutRecorder`] — append-only JSONL writer per rollout thread.
//!
//! Crash-safe: every line is `write_all` + `flush().await` (Codex pattern;
//! fsync per line trades throughput for replay determinism). Spills any
//! payload larger than [`crate::SPILL_THRESHOLD_BYTES`] to a content-hashed
//! sidecar blob and emits a [`RolloutPayload::Spilled`] pointer in its place.
//!
//! The recorder is the single mount point the apxm event sink fan-out talks
//! to: every event that lands on the in-memory [`crate::IndexDb`]/`RunEventBus`
//! is mirrored here for durable replay.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use apxm_core::events::ApxmEvent;
use chrono::{DateTime, Utc};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

use crate::SCHEMA_VERSION;
use crate::line::{
    EventMsgPayload, RolloutLine, RolloutMeta, RolloutPayload, SessionMetaPayload, SpilledPayload,
    Usage,
};
use crate::paths::RolloutPaths;

/// Errors from the writer path.
#[derive(Debug, thiserror::Error)]
pub enum RolloutWriteError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Construction-time configuration for the recorder.
#[derive(Debug, Clone)]
pub struct RolloutRecorderConfig {
    pub paths: Arc<RolloutPaths>,
    pub thread_id: String,
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    pub is_sidechain: bool,
    /// Override spill threshold. None ⇒ default from env or
    /// [`crate::SPILL_THRESHOLD_BYTES`].
    pub spill_threshold_bytes: Option<u64>,
    /// Override the SessionMeta line path: when set, write to this exact
    /// path instead of resolving from `paths`. Used for subagent files.
    pub override_path: Option<PathBuf>,
}

/// Per-line meta fields the writer can't derive from an event/payload.
#[derive(Debug, Default, Clone)]
pub struct PartialMeta {
    pub parent_uuid: Option<String>,
    pub tool_use_id: Option<String>,
    pub node_id: Option<String>,
    pub usage: Option<Usage>,
}

/// Append-only JSONL writer for a single rollout thread.
pub struct RolloutRecorder {
    paths: Arc<RolloutPaths>,
    thread_id: String,
    session_id: String,
    started_at: DateTime<Utc>,
    is_sidechain: bool,
    file_path: PathBuf,
    file: Mutex<BufWriter<File>>,
    next_seq: AtomicU64,
    spill_threshold: u64,
    last_uuid: Mutex<Option<String>>,
}

impl RolloutRecorder {
    /// Open a recorder and write the SessionMeta line (seq=0). Creates any
    /// missing parent directories.
    pub async fn open(
        config: RolloutRecorderConfig,
        session_meta: SessionMetaPayload,
    ) -> Result<Self, RolloutWriteError> {
        let file_path = match config.override_path.clone() {
            Some(path) => path,
            None => config.paths.rollout_path(&config.thread_id, config.started_at),
        };
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let raw = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await?;
        let file = BufWriter::new(raw);

        let spill_threshold = config
            .spill_threshold_bytes
            .or_else(spill_threshold_from_env)
            .unwrap_or(crate::SPILL_THRESHOLD_BYTES);

        let recorder = Self {
            paths: config.paths,
            thread_id: config.thread_id.clone(),
            session_id: config.session_id.clone(),
            started_at: config.started_at,
            is_sidechain: config.is_sidechain,
            file_path,
            file: Mutex::new(file),
            next_seq: AtomicU64::new(0),
            spill_threshold,
            last_uuid: Mutex::new(None),
        };

        // SessionMeta MUST be the first line of every rollout file; this
        // pins the reproducibility envelope (artifact_hash/source_hash/air_hash)
        // before any UserMessage/ToolUse can show up.
        let session_payload = RolloutPayload::SessionMeta(Box::new(session_meta));
        recorder
            .write_line(session_payload, PartialMeta::default())
            .await?;
        Ok(recorder)
    }

    /// Resolved file path — exposed for the index sink.
    pub fn file_path(&self) -> &PathBuf {
        &self.file_path
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    /// Write a payload as a new rollout line. Computes the seq, fills in
    /// the metadata, spills large payloads to a blob.
    pub async fn write_line(
        &self,
        mut payload: RolloutPayload,
        partial: PartialMeta,
    ) -> Result<RolloutLine, RolloutWriteError> {
        // Spill BEFORE serializing the line itself so the line stays small.
        // We never spill the SessionMeta line — that's the reproducibility
        // anchor and must be inline at seq=0.
        if !matches!(payload, RolloutPayload::SessionMeta(_)) {
            payload = self.maybe_spill(payload).await?;
        }

        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let uuid = Uuid::new_v4().to_string();
        let mut last = self.last_uuid.lock().await;
        let parent_uuid = partial.parent_uuid.clone().or_else(|| last.clone());
        let meta = RolloutMeta {
            seq,
            timestamp: Utc::now().to_rfc3339(),
            trace_id: self.thread_id.clone(),
            span_id: Uuid::new_v4().to_string(),
            parent_span_id: None,
            scope_id: None,
            source: apxm_core::events::EventSource::Runtime,
            skill: None,
            uuid: uuid.clone(),
            parent_uuid,
            session_id: self.session_id.clone(),
            thread_id: self.thread_id.clone(),
            is_sidechain: self.is_sidechain,
            tool_use_id: partial.tool_use_id,
            schema_version: SCHEMA_VERSION.to_string(),
            usage: partial.usage,
            node_id: partial.node_id,
        };
        let line = RolloutLine { meta, payload };
        let serialized = serde_json::to_string(&line)?;
        let mut file = self.file.lock().await;
        file.write_all(serialized.as_bytes()).await?;
        file.write_all(b"\n").await?;
        // fsync-per-line trade is intentional (Codex pattern): a crash
        // mid-turn must leave a complete prefix of valid lines.
        file.flush().await?;
        *last = Some(uuid);
        Ok(line)
    }

    /// Convenience: turn an ApxmEvent into a rollout Event line.
    pub async fn write_event(
        &self,
        event: ApxmEvent,
        partial: PartialMeta,
    ) -> Result<RolloutLine, RolloutWriteError> {
        let event_kind = event.kind().name().to_string();
        let event_json = serde_json::to_value(&event)?;
        self.write_line(
            RolloutPayload::Event(EventMsgPayload {
                event_kind,
                event: event_json,
            }),
            partial,
        )
        .await
    }

    /// Final flush — call before dropping. Required for `last_event_id`
    /// replay determinism on a clean shutdown.
    pub async fn close(&self) -> Result<(), RolloutWriteError> {
        let mut file = self.file.lock().await;
        file.flush().await?;
        Ok(())
    }

    async fn maybe_spill(
        &self,
        payload: RolloutPayload,
    ) -> Result<RolloutPayload, RolloutWriteError> {
        let serialized = serde_json::to_vec(&payload)?;
        let len = serialized.len() as u64;
        if len <= self.spill_threshold {
            return Ok(payload);
        }
        // Blob is content-addressed: blake3 of the serialized payload so
        // replay can prove byte-identity.
        let hash = blake3::hash(&serialized).to_hex().to_string();
        let original_kind = payload_kind_name(&payload).to_string();
        let blob_path = self
            .paths
            .blob_path(&self.thread_id, self.started_at, &hash, "json");
        if let Some(parent) = blob_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if !blob_path.exists() {
            // Atomic-ish: write to a tmp file then rename so partial blobs
            // never appear after a crash. Skip when the blob already exists
            // (content-addressed; duplicate writes are no-ops).
            let tmp = blob_path.with_extension("json.tmp");
            tokio::fs::write(&tmp, &serialized).await?;
            tokio::fs::rename(&tmp, &blob_path).await?;
        }
        Ok(RolloutPayload::Spilled(SpilledPayload {
            blob_ref: hash,
            bytes_estimate: len,
            mime: Some("application/json".to_string()),
            original_kind: Some(original_kind),
        }))
    }
}

fn payload_kind_name(payload: &RolloutPayload) -> &'static str {
    match payload {
        RolloutPayload::SessionMeta(_) => "session_meta",
        RolloutPayload::TurnContext(_) => "turn_context",
        RolloutPayload::UserMessage(_) => "user_message",
        RolloutPayload::AssistantMessage(_) => "assistant_message",
        RolloutPayload::ToolUse(_) => "tool_use",
        RolloutPayload::ToolResult(_) => "tool_result",
        RolloutPayload::Compacted(_) => "compacted",
        RolloutPayload::Event(_) => "event",
        RolloutPayload::Spilled(_) => "spilled",
    }
}

fn spill_threshold_from_env() -> Option<u64> {
    match std::env::var("APXM_ROLLOUT_SPILL_THRESHOLD_BYTES") {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(value) => Some(value),
            Err(error) => {
                warn!(%error, value = %raw, "ignoring invalid APXM_ROLLOUT_SPILL_THRESHOLD_BYTES");
                None
            }
        },
        Err(_) => None,
    }
}

/// Helper: a UTC timestamp string suitable for [`SessionMetaPayload::started_at`].
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    DateTime::<Utc>::from_timestamp(secs as i64, 0)
        .map(|ts| ts.to_rfc3339())
        .unwrap_or_else(|| Utc::now().to_rfc3339())
}
