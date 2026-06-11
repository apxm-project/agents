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
