//! rollout writer wiring.
//!
//! Lives alongside [`crate::runs`]: every event that lands on `RunEventBus`
//! also gets persisted via a parallel JSONL line per execution. The recorder
//! is constructed lazily — the first event for an execution_id opens its
//! file with a synthesized SessionMeta. Subsequent events append. This keeps
//! the integration surface minimal: nothing changes for callers that don't
//! know about rollouts; observers gain durable replay across restarts.

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use apxm_core::events::{ApxmEvent, EventEmitter};
use apxm_driver::ServerRolloutConfig;
use apxm_rollout::{
    IndexDb, PartialMeta, RolloutPaths, RolloutRecorder, RolloutRecorderConfig, SessionMetaPayload,
    ThreadIndexEntry, now_rfc3339,
};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tracing::warn;

const MIN_ROLLOUT_EVENT_BUFFER: usize = 128;
const MAX_ROLLOUT_EVENT_BUFFER: usize = 65_536;

/// Holds open recorders keyed by trace_id (which is the execution_id for
/// runtime events). The map is small — one entry per in-flight run.
#[derive(Clone)]
pub(crate) struct RolloutRegistry {
    inner: Arc<DashMap<String, Arc<RolloutWriter>>>,
    event_buffer: usize,
    spill_threshold_bytes: Option<u64>,
}

impl RolloutRegistry {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_config(&ServerRolloutConfig::default())
    }

    pub(crate) fn with_config(config: &ServerRolloutConfig) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            event_buffer: config
                .event_buffer
                .clamp(MIN_ROLLOUT_EVENT_BUFFER, MAX_ROLLOUT_EVENT_BUFFER),
            spill_threshold_bytes: config.spill_threshold_bytes,
        }
    }

    pub(crate) async fn open_for_run(
        &self,
        paths: Arc<RolloutPaths>,
        index: Option<Arc<Mutex<IndexDb>>>,
        execution_id: &str,
        session_id: &str,
        session_meta: SessionMetaPayload,
    ) -> Option<Arc<RolloutRecorder>> {
        if let Some(existing) = self.inner.get(execution_id) {
            return Some(existing.recorder.clone());
        }
        let started_at = Utc::now();
        let cfg = RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: execution_id.to_string(),
            session_id: session_id.to_string(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: self.spill_threshold_bytes,
            override_path: None,
        };
        match RolloutRecorder::open(cfg, session_meta.clone()).await {
            Ok(recorder) => {
                let recorder = Arc::new(recorder);
                let writer = Arc::new(RolloutWriter::start(
                    execution_id.to_string(),
                    recorder.clone(),
                    self.event_buffer,
                ));
                self.inner.insert(execution_id.to_string(), writer);
                if let Some(index) = index {
                    insert_index_row(&index, recorder.file_path(), &session_meta, started_at).await;
                }
                Some(recorder)
            }
            Err(error) => {
                warn!(%error, execution_id, "failed to open rollout recorder");
                None
            }
        }
    }

    /// Convenience: close + drop a recorder. Idempotent.
    pub(crate) async fn close(&self, execution_id: &str) {
        if let Some((_, writer)) = self.inner.remove(execution_id) {
            writer.shutdown(execution_id).await;
        }
    }

    pub(crate) fn try_record(&self, execution_id: &str, event: ApxmEvent) {
        let Some(writer) = self.inner.get(execution_id) else {
            return;
        };
        if let Err(error) = writer.try_send(event) {
            warn!(%error, execution_id, "rollout event queue rejected event");
        }
    }
}

struct RolloutWriter {
    recorder: Arc<RolloutRecorder>,
    tx: StdMutex<Option<mpsc::Sender<ApxmEvent>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl RolloutWriter {
    fn start(
        execution_id: String,
        recorder: Arc<RolloutRecorder>,
        event_buffer: usize,
    ) -> RolloutWriter {
        let (tx, mut rx) = mpsc::channel(event_buffer);
        let worker_recorder = recorder.clone();
        let worker_execution_id = execution_id.clone();
        let handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(error) = worker_recorder
                    .write_event(event, PartialMeta::default())
                    .await
                {
                    warn!(%error, execution_id = %worker_execution_id, "rollout write_event failed");
                }
            }
        });

        RolloutWriter {
            recorder,
            tx: StdMutex::new(Some(tx)),
            handle: Mutex::new(Some(handle)),
        }
    }

    fn try_send(&self, event: ApxmEvent) -> Result<(), mpsc::error::TrySendError<ApxmEvent>> {
        let Ok(guard) = self.tx.lock() else {
            return Err(mpsc::error::TrySendError::Closed(event));
        };
        match guard.as_ref() {
            Some(tx) => tx.try_send(event),
            None => Err(mpsc::error::TrySendError::Closed(event)),
        }
    }

    async fn shutdown(&self, execution_id: &str) {
        if let Ok(mut guard) = self.tx.lock() {
            guard.take();
        }

        if let Some(handle) = self.handle.lock().await.take()
            && let Err(error) = handle.await
        {
            warn!(%error, execution_id, "rollout writer task failed");
        }

        if let Err(error) = self.recorder.close().await {
            warn!(%error, execution_id, "failed to close rollout recorder");
        }
    }
}

async fn insert_index_row(
    index: &Mutex<IndexDb>,
    file_path: &Path,
    session_meta: &SessionMetaPayload,
    started_at: DateTime<Utc>,
) {
    let entry = ThreadIndexEntry {
        thread_id: session_meta.thread_id.clone(),
        parent_thread_id: session_meta.parent_thread_id.clone(),
        session_id: session_meta.session_id.clone(),
        started_at: started_at.to_rfc3339(),
        completed_at: None,
        status: "running".to_string(),
        agent_role: session_meta.agent_role.clone(),
        agent_code: session_meta.agent_code.clone(),
        file_path: file_path.to_string_lossy().into_owned(),
        line_count: 1,
        file_bytes: 0,
    };
    let guard = index.lock().await;
    if let Err(error) = guard.insert_or_update(&entry) {
        warn!(%error, "failed to insert rollout index row");
    }
}

/// EventEmitter that mirrors each ApxmEvent into the active rollout
/// recorder for `execution_id`. The recorder must have been opened before
/// the first emit lands.
pub(crate) struct RolloutEmitter {
    registry: RolloutRegistry,
    execution_id: String,
}

impl RolloutEmitter {
    pub(crate) fn new(registry: RolloutRegistry, execution_id: impl Into<String>) -> Self {
        Self {
            registry,
            execution_id: execution_id.into(),
        }
    }
}

impl EventEmitter for RolloutEmitter {
    fn emit(&self, event: ApxmEvent) {
        self.registry.try_record(&self.execution_id, event);
    }
}


/// Build a synthetic SessionMeta from skill execution context. Used by
/// the server when the caller hasn't supplied a richer one (the
/// reproducibility-pin hashes come from the skill manifest).
#[allow(clippy::too_many_arguments)]
pub(crate) fn session_meta_from_skill(
    execution_id: &str,
    session_id: &str,
    skill_id: &str,
    skill_version: &str,
    artifact_hash: Option<&str>,
    source_hash: Option<&str>,
    air_hash: Option<&str>,
    args: Vec<String>,
) -> SessionMetaPayload {
    SessionMetaPayload {
        thread_id: execution_id.to_string(),
        parent_thread_id: None,
        session_id: session_id.to_string(),
        started_at: now_rfc3339(),
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        apxm_version: env!("CARGO_PKG_VERSION").to_string(),
        agent_role: "coordinator".to_string(),
        agent_code: None,
        skill_id: skill_id.to_string(),
        skill_version: skill_version.to_string(),
        // Missing hashes (prompt-only skills don't compile to a .apxmobj)
        // still need a valid pin field; we record the empty marker so the
        // line shape is uniform across paths.
        artifact_hash: artifact_hash.unwrap_or("").to_string(),
        source_hash: source_hash.unwrap_or("").to_string(),
        air_hash: air_hash.unwrap_or("").to_string(),
        compiler_version: None,
        runtime_version: None,
        args,
        model_provider: None,
        model_id: None,
        backend_endpoint: None,
        tool_use_id_in_parent: None,
    }
}

/// Build a synthetic SessionMeta for a raw `/v1/execute/stream` turn (the
/// path the conversational `apxm chat` CLI and the studio Chat both POST to).
///
/// Each chat turn is a fresh execution, so `thread_id` is the per-turn
/// `execution_id`; the durable, cross-turn key is `session_id`, which lets
/// `IndexDb::list_by_session` gather every turn of one conversation. There is
/// no skill manifest behind a raw execute, so the reproducibility-pin hashes
/// are left empty (the line shape stays uniform with the skill path).
pub(crate) fn session_meta_from_chat(
    execution_id: &str,
    session_id: &str,
    args: Vec<String>,
) -> SessionMetaPayload {
    SessionMetaPayload {
        thread_id: execution_id.to_string(),
        parent_thread_id: None,
        session_id: session_id.to_string(),
        started_at: now_rfc3339(),
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        apxm_version: env!("CARGO_PKG_VERSION").to_string(),
        agent_role: "chat".to_string(),
        agent_code: None,
        skill_id: String::new(),
        skill_version: String::new(),
        artifact_hash: String::new(),
        source_hash: String::new(),
        air_hash: String::new(),
        compiler_version: None,
        runtime_version: None,
        args,
        model_provider: None,
        model_id: None,
        backend_endpoint: None,
        tool_use_id_in_parent: None,
    }
}
