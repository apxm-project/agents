//! Filesystem layout helpers.
//!
//! Layout (respects the existing `.apxm/sessions/skills/<id>/<execution_id>/`
//! per-node workspace — rollout sits ALONGSIDE, never replacing it):
//!
//! ```text
//! <apxm_home>/sessions/
//!   rollouts/YYYY/MM/DD/
//!     rollout-<thread_id>.jsonl              ← main thread
//!     rollout-<thread_id>/                   ← sidecar dir
//!       subagents/agent-<agent_id>.jsonl     ← subagent threads
//!       blobs/<blake3>.<ext>                 ← spilled large payloads
//!   index.sqlite                             ← thread index
//! ```

use chrono::{DateTime, Datelike, Utc};
use std::path::{Path, PathBuf};

const SESSIONS_DIR: &str = "sessions";
const ROLLOUTS_DIR: &str = "rollouts";
const SUBAGENTS_DIR: &str = "subagents";
const BLOBS_DIR: &str = "blobs";
const INDEX_FILE: &str = "index.sqlite";
const ROLLOUT_PREFIX: &str = "rollout-";
const AGENT_PREFIX: &str = "agent-";

/// Resolver for rollout filesystem layout. Built once at process start and
/// cloned (cheap — only a PathBuf).
#[derive(Debug, Clone)]
pub struct RolloutPaths {
    pub apxm_home: PathBuf,
}

impl RolloutPaths {
    /// Build from `APXM_ROLLOUT_HOME` env var when set, otherwise from the
    /// read-write state root ([`apxm_core::env::state_home`], i.e.
    /// `APXM_STATE_HOME → APXM_HOME → ~/.apxm`). Rollouts are mutable per-run
    /// state, so they follow the same state root as sessions and memory.
    pub fn from_env() -> Self {
        if let Ok(custom) = std::env::var("APXM_ROLLOUT_HOME") {
            return Self {
                apxm_home: PathBuf::from(custom),
            };
        }
        Self {
            apxm_home: apxm_core::env::state_home(),
        }
    }

    pub fn new(apxm_home: PathBuf) -> Self {
        Self { apxm_home }
    }

    /// `<apxm_home>/sessions`
    pub fn sessions_root(&self) -> PathBuf {
        self.apxm_home.join(SESSIONS_DIR)
    }

    /// `<apxm_home>/sessions/rollouts/YYYY/MM/DD`
    pub fn rollouts_date_dir(&self, started_at: DateTime<Utc>) -> PathBuf {
        self.sessions_root()
            .join(ROLLOUTS_DIR)
            .join(year_month_day(started_at))
    }

    /// Main thread rollout file path.
    pub fn rollout_path(&self, thread_id: &str, started_at: DateTime<Utc>) -> PathBuf {
        self.rollouts_date_dir(started_at)
            .join(format!("{ROLLOUT_PREFIX}{thread_id}.jsonl"))
    }

    /// Sidecar directory for a thread (subagents + blobs).
    pub fn rollout_sidecar_dir(&self, thread_id: &str, started_at: DateTime<Utc>) -> PathBuf {
        self.rollouts_date_dir(started_at)
            .join(format!("{ROLLOUT_PREFIX}{thread_id}"))
    }

    /// Subagent rollout file path.
    pub fn subagent_path(
        &self,
        parent_thread_id: &str,
        parent_started_at: DateTime<Utc>,
        agent_id: &str,
    ) -> PathBuf {
        self.rollout_sidecar_dir(parent_thread_id, parent_started_at)
            .join(SUBAGENTS_DIR)
            .join(format!("{AGENT_PREFIX}{agent_id}.jsonl"))
    }

    /// Spilled blob path. `ext` should not include the leading `.`.
    pub fn blob_path(
        &self,
        parent_thread_id: &str,
        parent_started_at: DateTime<Utc>,
        blake3_hex: &str,
        ext: &str,
    ) -> PathBuf {
        self.rollout_sidecar_dir(parent_thread_id, parent_started_at)
            .join(BLOBS_DIR)
            .join(format!("{blake3_hex}.{ext}"))
    }

    /// SQLite index db path.
    pub fn index_db_path(&self) -> PathBuf {
        self.sessions_root().join(INDEX_FILE)
    }
}

fn year_month_day(ts: DateTime<Utc>) -> String {
    format!("{:04}/{:02}/{:02}", ts.year(), ts.month(), ts.day())
}

/// Convenience wrapper for callers that don't want to instantiate a
/// [`RolloutPaths`] just to resolve one file.
pub fn rollout_path_for(apxm_home: &Path, thread_id: &str, started_at: DateTime<Utc>) -> PathBuf {
    RolloutPaths::new(apxm_home.to_path_buf()).rollout_path(thread_id, started_at)
}

pub fn subagent_path_for(
    apxm_home: &Path,
    parent_thread_id: &str,
    parent_started_at: DateTime<Utc>,
    agent_id: &str,
) -> PathBuf {
    RolloutPaths::new(apxm_home.to_path_buf()).subagent_path(
        parent_thread_id,
        parent_started_at,
        agent_id,
    )
}

pub fn blob_path_for(
    apxm_home: &Path,
    parent_thread_id: &str,
    parent_started_at: DateTime<Utc>,
    blake3_hex: &str,
    ext: &str,
) -> PathBuf {
    RolloutPaths::new(apxm_home.to_path_buf()).blob_path(
        parent_thread_id,
        parent_started_at,
        blake3_hex,
        ext,
    )
}
