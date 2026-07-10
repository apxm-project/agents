//! Tolerant reader for rollout JSONL files.
//!
//! Aborting on a single malformed line would defeat the durability goal.
//! Unparseable lines bump `LoadStats.parse_errors` and the rest of the
//! file is returned to the caller.

use std::path::Path;

use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::warn;

use crate::line::{CompactedPayload, RolloutLine, RolloutPayload};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LoadStats {
    pub total_lines: usize,
    pub parse_errors: usize,
}

/// Load a rollout file from disk in arrival order. Unparseable lines are
/// counted and skipped, never propagated as an error.
pub async fn load_rollout(path: &Path) -> Result<(Vec<RolloutLine>, LoadStats), LoadError> {
    let file = File::open(path).await?;
    let mut reader = BufReader::new(file).lines();
    let mut out = Vec::new();
    let mut stats = LoadStats::default();

    while let Some(raw) = reader.next_line().await? {
        if raw.trim().is_empty() {
            continue;
        }
        stats.total_lines += 1;
        match serde_json::from_str::<RolloutLine>(&raw) {
            Ok(line) => out.push(line),
            Err(error) => {
                warn!(%error, "unparseable rollout line skipped");
                stats.parse_errors += 1;
            }
        }
    }
    Ok((out, stats))
}

/// Reconstructed history baseline + suffix replay (Codex pattern).
#[derive(Debug, Clone, Default)]
pub struct ConversationTree {
    /// Replacement history from the most recent Compacted marker, if any.
    pub baseline: Vec<RolloutPayload>,
    /// All lines from after the baseline cutoff forward.
    pub suffix: Vec<RolloutLine>,
    /// Tree edges keyed by uuid → parent_uuid for downstream graph rendering.
    pub parent_of: std::collections::HashMap<String, String>,
}

/// Walk the rollout in reverse to find the most recent
/// [`RolloutPayload::Compacted`] marker, then forward-replay the suffix.
/// When no compaction marker exists, returns the whole rollout as the suffix.
pub fn reconstruct_history(items: &[RolloutLine]) -> ConversationTree {
    let mut cutoff: Option<usize> = None;
    let mut baseline: Vec<RolloutPayload> = Vec::new();
    for (idx, line) in items.iter().enumerate().rev() {
        if let RolloutPayload::Compacted(CompactedPayload {
            replacement_history,
            ..
        }) = &line.payload
        {
            baseline = replacement_history
                .iter()
                .map(|boxed| (**boxed).clone())
                .collect();
            cutoff = Some(idx);
            break;
        }
    }
    let suffix: Vec<RolloutLine> = match cutoff {
        Some(idx) => items.iter().skip(idx + 1).cloned().collect(),
        None => items.to_vec(),
    };
    let mut parent_of = std::collections::HashMap::new();
    for line in items {
        if let Some(parent) = &line.meta.parent_uuid {
            parent_of.insert(line.meta.uuid.clone(), parent.clone());
        }
    }
    ConversationTree {
        baseline,
        suffix,
        parent_of,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use apxm_core::events::payload::TokenPayload;
    use apxm_core::events::{ApxmEvent, EventSource};
    use chrono::Utc;

    use super::*;
    use crate::line::SessionMetaPayload;
    use crate::paths::RolloutPaths;
    use crate::recorder::{PartialMeta, RolloutRecorder, RolloutRecorderConfig};

    fn session_meta(thread_id: &str, started_at: chrono::DateTime<Utc>) -> SessionMetaPayload {
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

    /// Recovery: a crash mid-`write_all` (before or after `flush`, see
    /// `recorder.rs`'s durability doc) can leave a rollout file's last line
    /// truncated to non-JSON. `load_rollout` must discard exactly that
    /// truncated tail line (skip + count, never propagated as an error) and
    /// `reconstruct_history` must produce the identical suffix it would if
    /// the truncated line had never been written at all — the single
    /// "last complete line wins" rule applied uniformly, not an ad hoc
    /// per-reader heuristic.
    #[tokio::test]
    async fn truncated_tail_line_is_discarded_consistently() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(RolloutPaths::new(tmp.path().to_path_buf()));
        let thread_id = "thread-truncated-tail";
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

        // Two complete lines: SessionMeta (seq=0) + one event (seq=1).
        recorder
            .write_event(
                ApxmEvent::root(
                    TokenPayload {
                        text: "first".to_string(),
                    },
                    EventSource::Runtime,
                    thread_id,
                ),
                PartialMeta::default(),
            )
            .await
            .expect("write first event");
        recorder.close().await.unwrap();
        let complete_len = tokio::fs::metadata(recorder.file_path())
            .await
            .unwrap()
            .len();

        // A third line, then simulate a crash mid-`write_all`: truncate the
        // file partway through the line instead of leaving it complete.
        recorder
            .write_event(
                ApxmEvent::root(
                    TokenPayload {
                        text: "second".to_string(),
                    },
                    EventSource::Runtime,
                    thread_id,
                ),
                PartialMeta::default(),
            )
            .await
            .expect("write second event");
        recorder.close().await.unwrap();
        let full_len = tokio::fs::metadata(recorder.file_path())
            .await
            .unwrap()
            .len();
        assert!(
            full_len > complete_len + 10,
            "third line must add a meaningful number of bytes to truncate mid-line"
        );
        // Cut off the last ~40% of the third line's bytes — a torn write,
        // not a clean removal, so the remaining tail is non-JSON.
        let third_line_len = full_len - complete_len;
        let truncated_len = complete_len + (third_line_len * 3 / 5);
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(recorder.file_path())
            .await
            .unwrap();
        file.set_len(truncated_len).await.unwrap();
        drop(file);

        // The truncated file: exactly one unparseable trailing line.
        let (truncated_items, truncated_stats) = load_rollout(recorder.file_path())
            .await
            .expect("load truncated rollout");
        assert_eq!(
            truncated_stats.parse_errors, 1,
            "the torn third line is counted as exactly one parse error"
        );
        assert_eq!(
            truncated_items.len(),
            2,
            "only the two complete lines (SessionMeta + first event) are returned"
        );

        // A file with the third line entirely absent (truncated to exactly
        // the end of the second complete line) must be indistinguishable
        // downstream: same item count, same reconstructed suffix.
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(recorder.file_path())
            .await
            .unwrap();
        file.set_len(complete_len).await.unwrap();
        drop(file);
        let (absent_items, absent_stats) = load_rollout(recorder.file_path())
            .await
            .expect("load as-if-absent rollout");
        assert_eq!(
            absent_stats.parse_errors, 0,
            "no partial line remains at all"
        );
        assert_eq!(absent_items.len(), 2);

        // `RolloutLine`/`RolloutPayload` don't derive `PartialEq` (they carry
        // `serde_json::Value` payloads); compare via their JSON projection,
        // which is exactly the byte-for-byte content identity that matters
        // here.
        let to_json = |items: &[RolloutLine]| serde_json::to_value(items).unwrap();
        assert_eq!(
            to_json(&truncated_items),
            to_json(&absent_items),
            "a torn tail line is discarded exactly as if it had never been written"
        );

        let truncated_tree = reconstruct_history(&truncated_items);
        let absent_tree = reconstruct_history(&absent_items);
        assert_eq!(
            to_json(&truncated_tree.suffix),
            to_json(&absent_tree.suffix),
            "reconstruct_history produces the identical suffix in both cases"
        );
    }
}
