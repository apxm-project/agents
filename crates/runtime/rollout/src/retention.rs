//! Retention & compaction for agents-owned durable state.
//!
//! Scope, by state-layout.v1 vector:
//!   (a) expired rollout compaction — a rollout JSONL older than its max-age
//!       policy has its bulky body truncated (only the `SessionMeta` line
//!       survives); the corresponding `sessions/index.sqlite` row is kept
//!       and re-marked with `status = "archived"`. The index row is never
//!       deleted.
//!   (b) unreferenced blob GC — a spilled blob
//!       (`rollout-<thread>/blobs/<blake3>.<ext>`) with no `Spilled` payload
//!       anywhere in the rollout tree pointing at it is removed once it has
//!       aged past a grace period; a referenced blob is left alone.
//!
//! Vectors (c) acked-journal pruning and (d) dead-letter alerting live in
//! the `os` repo (journal/DLQ are os-owned primitives) — not applicable here.
//!
//! Hard invariant: this module only ever touches paths under
//! `<apxm_home>/sessions/rollouts/**` and `<apxm_home>/sessions/index.sqlite`
//! (both resolved through [`RolloutPaths`]). It has no dependency on, and
//! must never gain a dependency on, `apxm-memory` or `apxm-backends` — the
//! unified memory tier (STM/LTM/episodic, FTS5 search) is out of scope for
//! compaction and nothing in this crate may delete or rewrite it. This
//! crate's `Cargo.toml` does not list `apxm-memory`/`apxm-backends` as
//! dependencies, which makes that guarantee structural, not just a
//! convention.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use apxm_core::constants::env as apxm_env;
use chrono::{DateTime, Utc};
use tracing::{debug, warn};
use walkdir::WalkDir;

use crate::index::{IndexDb, IndexError};
use crate::line::RolloutPayload;
use crate::loader::load_rollout;
use crate::paths::RolloutPaths;

/// Status value written to `sessions/index.sqlite` for a compacted rollout.
/// The row is never deleted — only its `status` column changes.
pub const ARCHIVED_STATUS: &str = "archived";

/// Retention policy for agents-owned durable state.
///
/// `sessions/rollouts` and `sessions/index.sqlite` are `class = durable`;
/// retention here
/// means "shrink the bulky JSONL body and mark the index row archived",
/// never "delete the index row" or "delete memory stores".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// A rollout older than this (by `SessionMeta.started_at`, falling back
    /// to file mtime) is eligible for compaction.
    pub rollout_max_age: Duration,
    /// An unreferenced blob must have been idle at least this long before
    /// GC deletes it, so an in-flight `maybe_spill` write can never race
    /// collection.
    pub blob_gc_grace: Duration,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            rollout_max_age: Duration::from_secs(90 * 24 * 3600),
            blob_gc_grace: Duration::from_secs(24 * 3600),
        }
    }
}

impl RetentionPolicy {
    /// Build from env vars (`APXM_RETENTION_ROLLOUT_MAX_AGE_DAYS`,
    /// `APXM_RETENTION_BLOB_GC_GRACE_HOURS`), falling back to
    /// [`RetentionPolicy::default`] for any unset/invalid value.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let rollout_max_age = std::env::var(apxm_env::APXM_RETENTION_ROLLOUT_MAX_AGE_DAYS)
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .map(|days| Duration::from_secs(days * 24 * 3600))
            .unwrap_or(defaults.rollout_max_age);
        let blob_gc_grace = std::env::var(apxm_env::APXM_RETENTION_BLOB_GC_GRACE_HOURS)
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .map(|hours| Duration::from_secs(hours * 3600))
            .unwrap_or(defaults.blob_gc_grace);
        Self {
            rollout_max_age,
            blob_gc_grace,
        }
    }
}

/// Errors from the compaction task.
#[derive(Debug, thiserror::Error)]
pub enum CompactionError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("index error: {0}")]
    Index(#[from] IndexError),
}

/// Outcome of one [`compact_rollouts`] run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RolloutCompactionReport {
    pub scanned: usize,
    pub archived: usize,
    pub bytes_reclaimed: u64,
}

/// Outcome of one [`gc_blobs`] run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BlobGcReport {
    pub scanned: usize,
    pub deleted: usize,
    pub retained: usize,
    pub bytes_reclaimed: u64,
}

/// Combined report for a full compaction pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompactionReport {
    pub rollouts: RolloutCompactionReport,
    pub blobs: BlobGcReport,
}

/// Run the full agents-owned compaction pass: archive expired rollouts,
/// then GC blobs left unreferenced by the (now-archived) rollouts.
///
/// Order matters: compacting a rollout first (vector a) may drop the last
/// `Spilled` pointer to a blob, which is exactly the case vector (b) is
/// meant to reclaim on the *next* run once the blob clears its grace
/// period — GC always runs against whatever is on disk right now.
pub async fn compact(
    paths: &RolloutPaths,
    policy: &RetentionPolicy,
) -> Result<CompactionReport, CompactionError> {
    let rollouts = compact_rollouts(paths, policy).await?;
    let blobs = gc_blobs(paths, policy).await?;
    Ok(CompactionReport { rollouts, blobs })
}

/// Vector (a): archive rollout JSONL bodies older than `policy.rollout_max_age`.
///
/// For each eligible rollout file: truncate the file down to just its
/// `SessionMeta` line (seq=0) and re-write the corresponding
/// `sessions/index.sqlite` row with `status = "archived"`. The index row
/// is updated in place via `INSERT ... ON CONFLICT DO UPDATE` — it is never
/// deleted, and every other column (thread_id, session_id, started_at, ...)
/// is preserved.
///
/// A rollout already archived (single-line file) is skipped — compaction is
/// idempotent.
pub async fn compact_rollouts(
    paths: &RolloutPaths,
    policy: &RetentionPolicy,
) -> Result<RolloutCompactionReport, CompactionError> {
    let mut report = RolloutCompactionReport::default();
    let root = paths.sessions_root().join("rollouts");
    if !root.exists() {
        return Ok(report);
    }

    let db_path = paths.index_db_path();
    let db = IndexDb::open(&db_path)?;
    let now = SystemTime::now();

    for entry in WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !is_main_rollout_file(path) {
            continue;
        }
        report.scanned += 1;

        let (items, _) = match load_rollout(path).await {
            Ok(v) => v,
            Err(error) => {
                warn!(%error, path = %path.display(), "retention: failed to load rollout, skipping");
                continue;
            }
        };
        if items.len() <= 1 {
            // Already archived (or empty) — nothing to shrink.
            continue;
        }
        let Some(first) = items.first() else { continue };
        let RolloutPayload::SessionMeta(meta) = &first.payload else {
            continue;
        };

        let age = rollout_age(path, &meta.started_at, now);
        if age < policy.rollout_max_age {
            continue;
        }

        let old_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        truncate_to_first_line(path).await?;
        let new_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        // Preserve every existing column; only status + size/line_count
        // change. Fall back to deriving from SessionMeta if the row was
        // never indexed yet (first run before an index rebuild).
        let mut existing = db.get(&meta.thread_id)?;
        if existing.is_none() {
            existing = Some(crate::index::ThreadIndexEntry {
                thread_id: meta.thread_id.clone(),
                parent_thread_id: meta.parent_thread_id.clone(),
                session_id: meta.session_id.clone(),
                started_at: meta.started_at.clone(),
                completed_at: None,
                status: "unknown".to_string(),
                agent_role: meta.agent_role.clone(),
                agent_code: meta.agent_code.clone(),
                file_path: path.to_string_lossy().into_owned(),
                line_count: items.len() as i64,
                file_bytes: old_bytes as i64,
            });
        }
        let mut entry = existing.expect("populated above");
        entry.status = ARCHIVED_STATUS.to_string();
        entry.line_count = 1;
        entry.file_bytes = new_bytes as i64;
        db.insert_or_update(&entry)?;

        report.archived += 1;
        report.bytes_reclaimed += old_bytes.saturating_sub(new_bytes);
        debug!(
            thread_id = %meta.thread_id,
            old_bytes,
            new_bytes,
            "retention: archived expired rollout"
        );
    }

    Ok(report)
}

/// Vector (b): delete blobs with zero references, once they've aged past
/// `policy.blob_gc_grace`. A blob still pointed at by any `Spilled` payload
/// anywhere under `sessions/rollouts/**` is always retained.
pub async fn gc_blobs(
    paths: &RolloutPaths,
    policy: &RetentionPolicy,
) -> Result<BlobGcReport, CompactionError> {
    let mut report = BlobGcReport::default();
    let root = paths.sessions_root().join("rollouts");
    if !root.exists() {
        return Ok(report);
    }

    // Pass 1: collect every blob hash still referenced by a Spilled payload,
    // across every jsonl file in the tree (main threads + subagents).
    let mut referenced: HashSet<String> = HashSet::new();
    for entry in WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !is_jsonl_file(path) {
            continue;
        }
        let (items, _) = match load_rollout(path).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        for line in &items {
            if let RolloutPayload::Spilled(spilled) = &line.payload {
                referenced.insert(spilled.blob_ref.clone());
            }
        }
    }

    // Pass 2: walk every `blobs/` sidecar dir and delete unreferenced,
    // grace-period-expired files.
    let now = SystemTime::now();
    for entry in WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let in_blobs_dir = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some("blobs");
        if !in_blobs_dir {
            continue;
        }
        report.scanned += 1;

        let hash = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if referenced.contains(hash) {
            report.retained += 1;
            continue;
        }

        let age = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .unwrap_or(Duration::ZERO);
        if age < policy.blob_gc_grace {
            // Too young — could still be mid-write from a concurrent
            // recorder. Leave it for the next pass.
            report.retained += 1;
            continue;
        }

        let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        match std::fs::remove_file(path) {
            Ok(()) => {
                report.deleted += 1;
                report.bytes_reclaimed += bytes;
                debug!(path = %path.display(), "retention: collected unreferenced blob");
            }
            Err(error) => {
                warn!(%error, path = %path.display(), "retention: failed to remove unreferenced blob");
                report.retained += 1;
            }
        }
    }

    Ok(report)
}

fn is_main_rollout_file(path: &Path) -> bool {
    path.is_file()
        && path.extension().and_then(|s| s.to_str()) == Some("jsonl")
        && path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|n| n.starts_with("rollout-"))
}

fn is_jsonl_file(path: &Path) -> bool {
    path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl")
}

fn rollout_age(path: &Path, started_at: &str, now: SystemTime) -> Duration {
    if let Ok(started) = DateTime::parse_from_rfc3339(started_at) {
        let started: DateTime<Utc> = started.with_timezone(&Utc);
        let now_dt: DateTime<Utc> = now.into();
        if let Ok(delta) = (now_dt - started).to_std() {
            return delta;
        }
    }
    // Fall back to filesystem mtime when started_at fails to parse.
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| now.duration_since(m).ok())
        .unwrap_or(Duration::ZERO)
}

/// Rewrite `path` so only its first line survives, atomically (write to a
/// temp file in the same directory, then rename over the original).
async fn truncate_to_first_line(path: &Path) -> std::io::Result<()> {
    let contents = tokio::fs::read_to_string(path).await?;
    let Some(first_line) = contents.lines().next() else {
        return Ok(());
    };
    let mut out = first_line.to_string();
    out.push('\n');
    let tmp = tmp_path_for(path);
    tokio::fs::write(&tmp, out.as_bytes()).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".compact.tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::rebuild_index_from_disk;
    use crate::line::{ContentBlock, SessionMetaPayload, UserMessagePayload};
    use crate::recorder::{PartialMeta, RolloutRecorder, RolloutRecorderConfig};
    use std::sync::Arc;

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
            skill_id: "skill".to_string(),
            skill_version: "1.0.0".to_string(),
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

    async fn open_recorder(
        paths: &Arc<RolloutPaths>,
        thread_id: &str,
        started_at: DateTime<Utc>,
        spill_threshold_bytes: Option<u64>,
    ) -> RolloutRecorder {
        let config = RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: thread_id.to_string(),
            session_id: format!("session-{thread_id}"),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes,
            override_path: None,
        };
        RolloutRecorder::open(config, session_meta(thread_id, started_at))
            .await
            .expect("open recorder")
    }

    #[tokio::test]
    async fn expired_rollout_is_archived_and_index_row_survives() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(RolloutPaths::new(tmp.path().to_path_buf()));

        let old_started = Utc::now() - chrono::Duration::days(200);
        let recorder = open_recorder(&paths, "thread-old", old_started, None).await;
        for i in 0..5 {
            recorder
                .write_line(
                    RolloutPayload::UserMessage(UserMessagePayload {
                        content: vec![ContentBlock::Text {
                            text: format!("message {i}"),
                        }],
                    }),
                    PartialMeta::default(),
                )
                .await
                .unwrap();
        }
        recorder.close().await.unwrap();
        let file_path = recorder.file_path().clone();
        let bytes_before = std::fs::metadata(&file_path).unwrap().len();

        // Seed the index the way the server sink / `apxm rollout list` would.
        rebuild_index_from_disk(&paths).await.unwrap();

        let policy = RetentionPolicy {
            rollout_max_age: Duration::from_secs(90 * 24 * 3600),
            blob_gc_grace: Duration::from_secs(3600),
        };
        let report = compact_rollouts(&paths, &policy).await.unwrap();
        assert_eq!(report.archived, 1);

        let bytes_after = std::fs::metadata(&file_path).unwrap().len();
        assert!(
            bytes_after < bytes_before,
            "expected the rollout body to shrink: before={bytes_before} after={bytes_after}"
        );

        // Index row survives, with status flipped to archived — every other
        // identifying column (thread/session id) is preserved.
        let db = IndexDb::open(&paths.index_db_path()).unwrap();
        let entry = db
            .get("thread-old")
            .unwrap()
            .expect("index row must survive compaction");
        assert_eq!(entry.status, ARCHIVED_STATUS);
        assert_eq!(entry.session_id, "session-thread-old");
        assert_eq!(entry.line_count, 1);

        // Compaction is idempotent: a second pass is a no-op.
        let report2 = compact_rollouts(&paths, &policy).await.unwrap();
        assert_eq!(report2.archived, 0);
    }

    #[tokio::test]
    async fn fresh_rollout_is_not_archived() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(RolloutPaths::new(tmp.path().to_path_buf()));

        let recorder = open_recorder(&paths, "thread-fresh", Utc::now(), None).await;
        recorder
            .write_line(
                RolloutPayload::UserMessage(UserMessagePayload {
                    content: vec![ContentBlock::Text {
                        text: "hi".to_string(),
                    }],
                }),
                PartialMeta::default(),
            )
            .await
            .unwrap();
        recorder.close().await.unwrap();
        rebuild_index_from_disk(&paths).await.unwrap();

        let report = compact_rollouts(&paths, &RetentionPolicy::default())
            .await
            .unwrap();
        assert_eq!(report.archived, 0);

        let db = IndexDb::open(&paths.index_db_path()).unwrap();
        let entry = db.get("thread-fresh").unwrap().unwrap();
        assert_ne!(entry.status, ARCHIVED_STATUS);
    }

    #[tokio::test]
    async fn unreferenced_blob_is_collected_referenced_blob_survives() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(RolloutPaths::new(tmp.path().to_path_buf()));
        let started = Utc::now();

        // Force every payload to spill so we get a real Spilled pointer +
        // blob file out of the production write path.
        let recorder = open_recorder(&paths, "thread-blobs", started, Some(1)).await;
        recorder
            .write_line(
                RolloutPayload::UserMessage(UserMessagePayload {
                    content: vec![ContentBlock::Text {
                        text: "this payload is forced to spill".to_string(),
                    }],
                }),
                PartialMeta::default(),
            )
            .await
            .unwrap();
        recorder.close().await.unwrap();

        let blobs_dir = paths
            .rollout_sidecar_dir("thread-blobs", started)
            .join("blobs");
        let referenced_count_before = std::fs::read_dir(&blobs_dir).unwrap().count();
        assert_eq!(
            referenced_count_before, 1,
            "expected exactly one real spilled blob"
        );

        // Plant an orphan blob nobody points at.
        let orphan_path = blobs_dir.join("deadbeefdeadbeefdeadbeefdeadbeef.json");
        std::fs::write(&orphan_path, b"{\"orphan\":true}").unwrap();

        // Zero grace so the freshly-planted orphan is eligible immediately.
        let policy = RetentionPolicy {
            rollout_max_age: RetentionPolicy::default().rollout_max_age,
            blob_gc_grace: Duration::ZERO,
        };
        let report = gc_blobs(&paths, &policy).await.unwrap();
        assert_eq!(report.deleted, 1);
        assert_eq!(report.retained, 1);

        assert!(!orphan_path.exists(), "orphan blob must be collected");
        let remaining: Vec<_> = std::fs::read_dir(&blobs_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(remaining.len(), 1, "the referenced blob must survive");
    }

    /// Structural + behavioral proof that compaction never touches the
    /// memory tier: `apxm-rollout` has no dependency on `apxm-memory` or
    /// `apxm-backends` (see Cargo.toml), so there is no code path by which
    /// `compact`/`compact_rollouts`/`gc_blobs` can reach memory storage.
    /// This test additionally proves it empirically: a sibling `memory/`
    /// directory under the same `apxm_home` (STM/LTM/episodic sit at
    /// `<apxm_home>/memory/...`, per `apxm_core::paths::ApxmPaths`) is
    /// byte-for-byte unchanged after a full compaction pass, because
    /// `compact` only ever walks `<apxm_home>/sessions/rollouts/**` and
    /// touches `<apxm_home>/sessions/index.sqlite`.
    #[tokio::test]
    async fn compaction_never_touches_memory_tier_storage() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(RolloutPaths::new(tmp.path().to_path_buf()));

        // Simulate the memory tier's on-disk footprint (LTM sqlite + episodic
        // jsonl), sibling to `sessions/`, without depending on apxm-memory.
        let memory_dir = tmp.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        let ltm_path = memory_dir.join("ltm.sqlite");
        let episodes_path = memory_dir.join("episodes.jsonl");
        std::fs::write(&ltm_path, b"pretend-sqlite-bytes-for-ltm-memory-store").unwrap();
        std::fs::write(&episodes_path, b"{\"episode\":\"remember this\"}\n").unwrap();
        let ltm_before = std::fs::read(&ltm_path).unwrap();
        let episodes_before = std::fs::read(&episodes_path).unwrap();

        // Also create an old rollout so the compaction pass actually does
        // real work (archiving + blob GC), not a no-op.
        let old_started = Utc::now() - chrono::Duration::days(400);
        let recorder = open_recorder(&paths, "thread-mem-check", old_started, Some(1)).await;
        recorder
            .write_line(
                RolloutPayload::UserMessage(UserMessagePayload {
                    content: vec![ContentBlock::Text {
                        text: "some content that will spill and later be archived".to_string(),
                    }],
                }),
                PartialMeta::default(),
            )
            .await
            .unwrap();
        recorder.close().await.unwrap();
        rebuild_index_from_disk(&paths).await.unwrap();

        let policy = RetentionPolicy {
            rollout_max_age: Duration::from_secs(90 * 24 * 3600),
            blob_gc_grace: Duration::ZERO,
        };
        let report = compact(&paths, &policy).await.unwrap();
        assert_eq!(
            report.rollouts.archived, 1,
            "sanity: compaction did real work"
        );

        let ltm_after = std::fs::read(&ltm_path).unwrap();
        let episodes_after = std::fs::read(&episodes_path).unwrap();
        assert_eq!(ltm_before, ltm_after, "LTM memory store must be untouched");
        assert_eq!(
            episodes_before, episodes_after,
            "episodic memory store must be untouched"
        );
    }
}
