//! [`RolloutRecorder`] — append-only JSONL writer per rollout thread.
//!
//! **Honest durability policy (not fsync):** every line is `write_all` +
//! `flush().await` on a `tokio::io::BufWriter<File>`. `flush()` only moves
//! bytes from the userspace `BufWriter` into the OS page cache — it does
//! **not** call `File::sync_all()`/`sync_data()`, and nothing in this crate
//! does. That is enough to survive this *process* dying (`kill -9`): the
//! kernel still holds the page-cache pages, so another process (or this one,
//! restarted) reading the file back sees every flushed line. It is **not**
//! enough to survive a *host* crash or power loss / VM eviction before the
//! kernel writes those pages back to disk — that window can lose the tail of
//! "durable" lines even though they were already flushed. If a write is torn
//! by such a crash mid-`write_all`, the file's last line may be incomplete
//! non-JSON; [`crate::loader::load_rollout`] treats it as the truncated-tail
//! case (skip + count, never propagated as an error — see its doc comment).
//! Callers that need disk-durability stronger than "survives a process kill"
//! must not assume this recorder provides it; see
//! the restart-state reconstruction invariant for the explicit sync/ack contract this
//! implies at park/checkpoint boundaries.
//!
//! Spills any payload larger than [`crate::SPILL_THRESHOLD_BYTES`] to a
//! content-hashed sidecar blob and emits a [`RolloutPayload::Spilled`]
//! pointer in its place.
//!
//! The recorder is the single mount point the apxm event sink fan-out talks
//! to: every event that lands on the in-memory [`crate::IndexDb`]/`RunEventBus`
//! is appended here for durable replay.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use apxm_core::constants::env as apxm_env;
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
            None => config
                .paths
                .rollout_path(&config.thread_id, config.started_at),
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
            program_package: None,
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
        // `flush()` is NOT `fsync`: it only guarantees the OS page cache has
        // the bytes (survives this process dying), not that they have
        // reached disk (does NOT survive a host crash/power loss before the
        // kernel writes back). See the module doc comment for the full
        // policy and why a `kill -9`-only test cannot detect this gap.
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
    match std::env::var(apxm_env::APXM_ROLLOUT_SPILL_THRESHOLD_BYTES) {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(value) => Some(value),
            Err(error) => {
                warn!(
                    %error,
                    value = %raw,
                    env = apxm_env::APXM_ROLLOUT_SPILL_THRESHOLD_BYTES,
                    "ignoring invalid rollout spill threshold env var"
                );
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use apxm_core::events::payload::GraphEdgePayload;
    use apxm_core::events::{ApxmEvent, EventSource};

    use super::*;
    use crate::line::{RolloutPayload, SessionMetaPayload};
    use crate::loader::load_rollout;

    fn session_meta(thread_id: &str, started_at: DateTime<Utc>) -> SessionMetaPayload {
        SessionMetaPayload {
            thread_id: thread_id.to_string(),
            parent_thread_id: None,
            session_id: format!("session-{thread_id}"),
            started_at: started_at.to_rfc3339(),
            cwd: "/tmp".to_string(),
            apxm_version: "0.1.0".to_string(),
            agent_role: "test-agent".to_string(),
            agent_code: None,
            program_package_id: "skill".to_string(),
            program_package_digest: "1.0.0".to_string(),
            artifact_hash: "artifact".to_string(),
            source_hash: "source".to_string(),
            air_hash: "air".to_string(),
            compiler_version: None,
            runtime_version: None,
            args: vec![],
            model_provider: None,
            model_id: None,
            backend_endpoint: None,
            tool_use_id_in_parent: None,
        }
    }

    /// Recovery: a `graph_edge` event written through the recorder and
    /// reloaded from disk keeps its `edge_kind` — pre-fix, the envelope
    /// serializer clobbered the (then-named) `kind` field before it ever
    /// hit the wire, so `boxed_payload_from_json` hard-failed on replay and
    /// every reader silently dropped the event via `.ok()`/`let-else`.
    #[tokio::test]
    async fn write_then_read_graph_edge_survives_rollout_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(RolloutPaths::new(tmp.path().to_path_buf()));
        let thread_id = "thread-graph-edge";
        let started_at = Utc::now();

        let config = RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: thread_id.to_string(),
            session_id: format!("session-{thread_id}"),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        };
        let recorder = RolloutRecorder::open(config, session_meta(thread_id, started_at))
            .await
            .expect("open recorder");

        let event = ApxmEvent::root(
            GraphEdgePayload {
                from_node_id: 3,
                to_node_id: 4,
                edge_kind: "tool_invocation".to_string(),
            },
            EventSource::Runtime,
            thread_id,
        );
        recorder
            .write_event(event, PartialMeta::default())
            .await
            .expect("write graph_edge event");
        recorder.close().await.expect("close recorder");

        let (items, stats) = load_rollout(recorder.file_path())
            .await
            .expect("load rollout");
        assert_eq!(
            stats.parse_errors, 0,
            "no rollout line should be unparseable"
        );

        let decoded = items
            .iter()
            .find_map(|line| match &line.payload {
                RolloutPayload::Event(event_payload) => {
                    serde_json::from_value::<ApxmEvent>(event_payload.event.clone()).ok()
                }
                _ => None,
            })
            .expect("graph_edge event decodes back from the rollout line");

        let payload = decoded
            .payload
            .downcast_ref::<GraphEdgePayload>()
            .expect("decoded payload is a GraphEdgePayload");
        assert_eq!(payload.from_node_id, 3);
        assert_eq!(payload.to_node_id, 4);
        assert_eq!(payload.edge_kind, "tool_invocation");
    }

    /// Doc/behavior conformance (not just prose): the module doc comment
    /// states the honest durability policy (`flush()` is not `fsync`, the
    /// real risk window is host crash/power-loss, not just `kill -9`), the
    /// old inflated "fsync per line" claim is gone, and the write path itself
    /// never calls `sync_all`/`sync_data` — then demonstrates, behaviorally,
    /// the byte range that can be lost: bytes staged in the `BufWriter` via
    /// `write_all` but not yet reached by `flush()` are discarded if the
    /// writer is dropped without flushing (exactly what a `kill -9` between
    /// `write_all` and `flush` does here). A true host power-loss AFTER
    /// `flush()` returns cannot be simulated in a unit test (the OS page
    /// cache survives a same-host process kill), so this proves the
    /// demonstrable half of the documented gap and states plainly, in this
    /// comment, why the other half (post-flush, pre-fsync) is not
    /// mechanically provable outside real hardware.
    #[tokio::test]
    async fn flush_is_not_fsync_documented_and_tested() {
        // Only the non-test portion of this file: the test module below
        // necessarily contains the literal strings "sync_all(" etc. in its
        // own assertions, which would otherwise make this check tautological.
        let source = include_str!("recorder.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("source has a test module marker")
            .to_string();
        let lowered = source.to_lowercase();
        assert!(
            lowered.contains("sync_all") && lowered.contains("sync_data"),
            "recorder.rs module doc must name sync_all/sync_data explicitly"
        );
        assert!(
            lowered.contains("not") && lowered.contains("fsync"),
            "recorder.rs module doc must state flush() is not fsync"
        );
        assert!(
            lowered.contains("power loss") || lowered.contains("power-loss"),
            "doc must name the real failure window: host crash/power-loss, not just kill -9"
        );
        assert!(
            !lowered.contains("fsync per line trades throughput"),
            "the old inflated 'fsync per line' claim must not remain in the docs"
        );
        // Structural proof the code matches the doc: no non-doc-comment line
        // in this file calls fsync/sync_all/sync_data (the doc comment
        // itself names them, in prose, as what is deliberately NOT called).
        let code_only: String = source
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("///") && !trimmed.starts_with("//!")
            })
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
        assert!(
            !code_only.contains("sync_all(")
                && !code_only.contains("sync_data(")
                && !code_only.contains(".sync()"),
            "recorder.rs must not itself call fsync — the doc's claim must match the code"
        );

        // Behavioral: bytes `write_all`-staged into a `BufWriter` but never
        // reached by `flush()` are lost if the writer is dropped without
        // flushing — the exact byte range the doc warns a crash can lose.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("crash.jsonl");
        let raw = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .await
            .unwrap();
        let mut buffered = BufWriter::new(raw);
        buffered.write_all(b"line-that-is-flushed\n").await.unwrap();
        buffered.flush().await.unwrap();
        buffered
            .write_all(b"line-that-is-never-flushed\n")
            .await
            .unwrap();
        // Simulate the crash: drop without flushing/shutting down — a
        // `kill -9` between this `write_all` and the next `flush()` has
        // exactly this effect on `RolloutRecorder::write_line`.
        drop(buffered);

        let on_disk = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(
            on_disk, "line-that-is-flushed\n",
            "only the flushed prefix survives; the staged-but-unflushed line is \
             lost — the exact byte range the recorder's doc comment warns about"
        );
    }
}
