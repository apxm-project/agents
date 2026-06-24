//! Run-history storage backend abstraction and migration notes.
//!
//! ## Current state (v0)
//!
//! Run records are held in an in-memory `DashMap` keyed by `execution_id` and
//! serialized to `<session>/executions/<id>.json` on disk.  The server rebuilds
//! from disk on startup (`rebuilt-from-disk` path).  The workflow-scoped list
//! route (`GET /v1/workflows/{id}/runs`) scans the in-memory map — not cheap at
//! scale but correct for a single-host v0 deployment.
//!
//! ## Production migration path
//!
//! | Backing | v0 | Production target |
//! |---------|----|--------------------|
//! | run records | DashMap + JSON files | SQLite index → Postgres run-history DB |
//! | cost/token aggregates | none (missing) | columns on the run record |
//! | artifact references | file-path strings | object-store URIs with typed expiry |
//!
//! The migration introduces a `runs` table keyed by `(workflow_id, execution_id)`
//! with indexed columns for `started_at`, `status`, `duration_ms`, and
//! `cost_tokens`.  The table is a derived, rebuildable index over the rollout JSONL
//! (the immutable source of truth); a `POST /v1/runs/reindex` endpoint rebuilds it
//! from scratch.
//!
//! ## Migration gates
//!
//! 1. **Schema roundtrip** — `sqlite3 sessions/index.sqlite .dump | sqlite3 /tmp/test.db`
//!    must produce identical `.tables` output.  Gate command:
//!    ```text
//!    sqlite3 $APXM_HOME/sessions/index.sqlite .dump | sqlite3 /tmp/test.db \
//!        && sqlite3 /tmp/test.db ".tables"
//!    ```
//! 2. **Retention annotations** — run record rows carry `retention_class = standard`
//!    (90-day TTL) before migration.  Checkpoint rows carry `retention_class = permanent`.
//! 3. **Cost/token columns** — add `cost_tokens INTEGER` and `cost_usd REAL` to the
//!    `runs` table before migration so aggregation queries work without a schema change.
//! 4. **Rollback plan** — a restore from Postgres back to SQLite passes
//!    `tools/backup-restore-fixture.sh restore` and `tests/storage_restore.rs`.
//! 5. **Reindex proof** — `POST /v1/runs/reindex` rebuilds the derived index from
//!    the JSONL and the result matches the pre-migration scan.
//!
//! ## Object-store artifact references
//!
//! Large run blobs (>1 MiB runner artifacts, rollout JSONL chunks) are stored as
//! object references, not embedded in the run record.  Each run record row holds:
//!
//! ```text
//! artifact_refs  -- JSON array of opaque URIs (s3://, gs://, file:// for v0)
//! ```
//!
//! Resolution goes through `src/artifacts/object_store.rs`, never via direct
//! volume access from the client.  Expired references return a typed error
//! (`artifact_expired`) rather than a silent 404 or null.
//!
//! ## Retention/GC policy
//!
//! | Data | Class | TTL | GC trigger |
//! |------|-------|-----|------------|
//! | run record rows | standard | 90 days | nightly GC |
//! | rollout JSONL | standard | 90 days | nightly GC |
//! | blob references | standard | 90 days | nightly GC (follows run record) |
//! | checkpoint rows | permanent | none | operator action |
//! | sandbox scratch | ephemeral | 1 hour | post-run cleanup |

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::DateTime;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::executions::{ExecutionRecord, ExecutionStatus};
use crate::helpers::now_ms;

const RUNS_TABLE_SQL: &str = r"
CREATE TABLE IF NOT EXISTS runs (
  execution_id TEXT PRIMARY KEY,
  workflow_id TEXT,
  skill_id TEXT,
  skill_version TEXT,
  session_id TEXT,
  session_dir TEXT,
  run_root TEXT,
  trace_id TEXT,
  status TEXT NOT NULL,
  started_at_ms INTEGER NOT NULL,
  finished_at_ms INTEGER,
  duration_ms INTEGER,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  total_tokens INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL,
  retention_class TEXT NOT NULL DEFAULT 'standard',
  hidden_at_ms INTEGER,
  updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_runs_workflow_started
  ON runs(workflow_id, started_at_ms DESC, execution_id);
CREATE INDEX IF NOT EXISTS idx_runs_status_started
  ON runs(status, started_at_ms DESC, execution_id);
";

/// Derived run-history row persisted in the v0 SQLite index.
///
/// The immutable execution evidence remains the session snapshot, rollout JSONL,
/// and `/workspace/runs/<workflow>/<execution>/` artifact tree. This row is the
/// query model Studio and API clients use for cheap workflow/run lists.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StoredRunRecord {
    pub(crate) run_id: String,
    pub(crate) workflow_id: Option<String>,
    pub(crate) skill_id: Option<String>,
    pub(crate) skill_version: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) session_dir: Option<String>,
    pub(crate) run_root: Option<String>,
    pub(crate) trace_id: Option<String>,
    pub(crate) status: ExecutionStatus,
    pub(crate) started_at: u64,
    pub(crate) finished_at: Option<u64>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cost_usd: Option<f64>,
    pub(crate) retention_class: String,
}

impl StoredRunRecord {
    pub(crate) fn from_execution_record(record: &ExecutionRecord) -> Self {
        let (input_tokens, output_tokens, total_tokens) = record
            .result
            .as_ref()
            .map_or((0, 0, 0), |result| {
                let input = result.llm_usage.input_tokens as u64;
                let output = result.llm_usage.output_tokens as u64;
                (input, output, input.saturating_add(output))
            });
        let finished_at = record.completed_at_ms;
        Self {
            run_id: record.execution_id.clone(),
            workflow_id: record.workflow_id.clone(),
            skill_id: Some(record.skill_id.clone()),
            skill_version: Some(record.skill_version.clone()),
            session_id: Some(record.session_id.clone()),
            session_dir: Some(record.session_dir.clone()),
            run_root: record.run_root.clone(),
            trace_id: record.trace_id.clone(),
            status: record.status.clone(),
            started_at: record.started_at_ms,
            finished_at,
            duration_ms: finished_at.and_then(|end| end.checked_sub(record.started_at_ms)),
            input_tokens,
            output_tokens,
            total_tokens,
            cost_usd: None,
            retention_class: RetentionClass::Standard.as_str().to_string(),
        }
    }
}

/// SQLite-backed workflow run-history index.
///
/// `None` is reserved for explicit in-memory tests. Production startup opens
/// `runs.sqlite` under the state sessions root and fails closed if the index
/// cannot be created, so workflow lists survive process restarts and do not rely
/// on scan-on-read.
#[derive(Clone)]
pub(crate) struct RunHistoryIndex {
    db: Option<Arc<Mutex<Connection>>>,
}

/// Minimal identity for a rollout thread row when inserting a hidden tombstone
/// (threads that appear in `GET /v1/runs` via the index fallback but have no
/// prior `runs` row).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RolloutThreadHideRow<'a> {
    pub execution_id: &'a str,
    pub session_id: &'a str,
    pub status: &'a str,
    pub started_at: &'a str,
}

impl RunHistoryIndex {
    pub(crate) fn disabled() -> Self {
        Self { db: None }
    }

    pub(crate) fn open(path: &Path) -> Result<Self, RunHistoryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| RunHistoryError::Backend(error.to_string()))?;
        }
        let conn =
            Connection::open(path).map_err(|error| RunHistoryError::Backend(error.to_string()))?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.execute_batch(RUNS_TABLE_SQL)
            .map_err(|error| RunHistoryError::Backend(error.to_string()))?;
        ensure_hidden_at_column(&conn)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_runs_visible_started
               ON runs(hidden_at_ms, started_at_ms DESC, execution_id)",
            [],
        )
        .map_err(|error| RunHistoryError::Backend(error.to_string()))?;
        Ok(Self {
            db: Some(Arc::new(Mutex::new(conn))),
        })
    }

    pub(crate) fn upsert_from_record(&self, record: &ExecutionRecord) {
        self.upsert_row(&StoredRunRecord::from_execution_record(record));
    }

    pub(crate) fn is_hidden(&self, execution_id: &str) -> bool {
        let Some(db) = &self.db else { return false };
        let Ok(conn) = db.lock() else { return false };
        conn.query_row(
            "SELECT hidden_at_ms IS NOT NULL FROM runs WHERE execution_id = ?1",
            [execution_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or(false)
    }

    pub(crate) fn hide_settled_visible(&self, hidden_at_ms: u64) -> Vec<String> {
        let Some(db) = &self.db else {
            return Vec::new();
        };
        let Ok(mut conn) = db.lock() else {
            return Vec::new();
        };
        let Ok(tx) = conn.transaction() else {
            return Vec::new();
        };
        let ids: Vec<String> = {
            let Ok(mut stmt) = tx.prepare(
                "SELECT execution_id
                   FROM runs
                  WHERE status != 'running' AND hidden_at_ms IS NULL",
            ) else {
                return Vec::new();
            };
            let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
                return Vec::new();
            };
            rows.filter_map(Result::ok).collect()
        };
        if ids.is_empty() {
            let _ = tx.commit();
            return ids;
        }
        let _ = tx.execute(
            "UPDATE runs
                SET hidden_at_ms = ?1,
                    updated_at_ms = ?1
              WHERE status != 'running' AND hidden_at_ms IS NULL",
            params![hidden_at_ms as i64],
        );
        let _ = tx.commit();
        ids
    }

    /// Upsert hidden tombstones for settled rollout-index threads so
    /// [`crate::executions::ExecutionStore::is_hidden`] returns true for
    /// index-only listings after `POST /v1/runs/clear`.
    pub(crate) fn hide_rollout_index_threads(
        &self,
        threads: &[RolloutThreadHideRow<'_>],
        hidden_at_ms: u64,
    ) -> usize {
        let Some(db) = &self.db else {
            return 0;
        };
        let Ok(conn) = db.lock() else {
            return 0;
        };
        let mut applied = 0usize;
        for row in threads {
            if row.status.eq_ignore_ascii_case("running") {
                continue;
            }
            let started_ms = DateTime::parse_from_rfc3339(row.started_at)
                .map_or(0, |ts| ts.timestamp_millis());
            let status_db = if row.status.eq_ignore_ascii_case("failed") {
                "failed"
            } else {
                "succeeded"
            };
            let hidden = hidden_at_ms as i64;
            if let Ok(n) = conn.execute(
                r"INSERT INTO runs (
                    execution_id, workflow_id, skill_id, skill_version, session_id, session_dir,
                    run_root, trace_id, status, started_at_ms, finished_at_ms, duration_ms,
                    input_tokens, output_tokens, total_tokens, cost_usd, retention_class,
                    hidden_at_ms, updated_at_ms
                 ) VALUES (
                    ?1, NULL, '', '', ?2, '', NULL, NULL, ?3, ?4, NULL, NULL,
                    0, 0, 0, NULL, 'standard', ?5, ?5
                 )
                 ON CONFLICT(execution_id) DO UPDATE SET
                    hidden_at_ms = excluded.hidden_at_ms,
                    updated_at_ms = excluded.updated_at_ms
                  WHERE runs.hidden_at_ms IS NULL",
                params![
                    row.execution_id,
                    row.session_id,
                    status_db,
                    started_ms,
                    hidden
                ],
            ) {
                applied += n;
            }
        }
        applied
    }

    pub(crate) fn upsert_row(&self, row: &StoredRunRecord) {
        let Some(db) = &self.db else { return };
        let Ok(conn) = db.lock() else { return };
        let _ = conn.execute(
            "INSERT INTO runs (
                execution_id, workflow_id, skill_id, skill_version, session_id, session_dir,
                run_root, trace_id, status, started_at_ms, finished_at_ms, duration_ms,
                input_tokens, output_tokens, total_tokens, cost_usd, retention_class, updated_at_ms
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
             )
             ON CONFLICT(execution_id) DO UPDATE SET
                workflow_id=excluded.workflow_id,
                skill_id=excluded.skill_id,
                skill_version=excluded.skill_version,
                session_id=excluded.session_id,
                session_dir=excluded.session_dir,
                run_root=excluded.run_root,
                trace_id=excluded.trace_id,
                status=excluded.status,
                started_at_ms=excluded.started_at_ms,
                finished_at_ms=excluded.finished_at_ms,
                duration_ms=excluded.duration_ms,
                input_tokens=excluded.input_tokens,
                output_tokens=excluded.output_tokens,
                total_tokens=excluded.total_tokens,
                cost_usd=excluded.cost_usd,
                retention_class=excluded.retention_class,
                updated_at_ms=excluded.updated_at_ms",
            params![
                row.run_id,
                row.workflow_id,
                row.skill_id,
                row.skill_version,
                row.session_id,
                row.session_dir,
                row.run_root,
                row.trace_id,
                status_to_str(&row.status),
                row.started_at as i64,
                row.finished_at.map(|value| value as i64),
                row.duration_ms.map(|value| value as i64),
                row.input_tokens as i64,
                row.output_tokens as i64,
                row.total_tokens as i64,
                row.cost_usd,
                row.retention_class,
                now_ms() as i64,
            ],
        );
    }

    pub(crate) fn list_workflow(&self, workflow_id: &str) -> Option<Vec<StoredRunRecord>> {
        let db = self.db.as_ref()?;
        let conn = db.lock().ok()?;
        let mut stmt = conn
            .prepare(
                "SELECT execution_id, workflow_id, skill_id, skill_version, session_id, session_dir,
                        run_root, trace_id, status, started_at_ms, finished_at_ms, duration_ms,
                        input_tokens, output_tokens, total_tokens, cost_usd, retention_class
                  FROM runs
                  WHERE workflow_id = ?1 AND hidden_at_ms IS NULL
                  ORDER BY started_at_ms DESC, execution_id ASC",
            )
            .ok()?;
        let rows = stmt
            .query_map([workflow_id], stored_run_from_row)
            .ok()?;
        Some(rows.filter_map(Result::ok).collect())
    }

    pub(crate) fn get(&self, execution_id: &str) -> Option<StoredRunRecord> {
        let db = self.db.as_ref()?;
        let conn = db.lock().ok()?;
        conn.query_row(
            "SELECT execution_id, workflow_id, skill_id, skill_version, session_id, session_dir,
                    run_root, trace_id, status, started_at_ms, finished_at_ms, duration_ms,
                    input_tokens, output_tokens, total_tokens, cost_usd, retention_class
               FROM runs
              WHERE execution_id = ?1",
            [execution_id],
            stored_run_from_row,
        )
        .optional()
        .ok()
        .flatten()
    }
}

fn ensure_hidden_at_column(conn: &Connection) -> Result<(), RunHistoryError> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(runs)")
        .map_err(|error| RunHistoryError::Backend(error.to_string()))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| RunHistoryError::Backend(error.to_string()))?;
    let has_hidden_at = columns
        .filter_map(Result::ok)
        .any(|name| name == "hidden_at_ms");
    if !has_hidden_at {
        conn.execute("ALTER TABLE runs ADD COLUMN hidden_at_ms INTEGER", [])
            .map_err(|error| RunHistoryError::Backend(error.to_string()))?;
    }
    Ok(())
}

/// Retention class for a run record or artifact reference.
///
/// Every row written to the run-history store carries one of these tags so GC
/// policies transfer to Postgres without a second migration pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionClass {
    Ephemeral,
    Short,
    Standard,
    Permanent,
}

impl RetentionClass {
    /// Default TTL in seconds (`None` means no expiry).
    pub fn default_ttl_secs(&self) -> Option<u64> {
        match self {
            Self::Ephemeral => Some(3_600),
            Self::Short => Some(7 * 86_400),
            Self::Standard => Some(90 * 86_400),
            Self::Permanent => None,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Ephemeral => "ephemeral",
            Self::Short => "short",
            Self::Standard => "standard",
            Self::Permanent => "permanent",
        }
    }
}

/// Typed error returned when a run record or artifact has expired.
///
/// Callers must distinguish `ArtifactExpired` (valid ID, TTL elapsed) from
/// `NotFound` (unknown ID).  Returning a bare 404 or null for an expired
/// artifact is a contract violation.
#[derive(Debug)]
pub enum RunHistoryError {
    NotFound {
        id: String,
    },
    ArtifactExpired {
        artifact_ref: String,
        expired_at: String,
        retention_class: String,
    },
    Backend(String),
}

impl std::fmt::Display for RunHistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { id } => write!(f, "run record not found: {id}"),
            Self::ArtifactExpired {
                artifact_ref,
                expired_at,
                retention_class,
            } => write!(
                f,
                "artifact_expired artifact_ref={artifact_ref} expired_at={expired_at} \
                 retention_class={retention_class}"
            ),
            Self::Backend(msg) => write!(f, "run history backend error: {msg}"),
        }
    }
}

impl std::error::Error for RunHistoryError {}

fn stored_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRunRecord> {
    let status: String = row.get(8)?;
    Ok(StoredRunRecord {
        run_id: row.get(0)?,
        workflow_id: row.get(1)?,
        skill_id: row.get(2)?,
        skill_version: row.get(3)?,
        session_id: row.get(4)?,
        session_dir: row.get(5)?,
        run_root: row.get(6)?,
        trace_id: row.get(7)?,
        status: status_from_str(&status),
        started_at: row.get::<_, i64>(9)?.max(0) as u64,
        finished_at: row
            .get::<_, Option<i64>>(10)?
            .map(|value| value.max(0) as u64),
        duration_ms: row
            .get::<_, Option<i64>>(11)?
            .map(|value| value.max(0) as u64),
        input_tokens: row.get::<_, i64>(12)?.max(0) as u64,
        output_tokens: row.get::<_, i64>(13)?.max(0) as u64,
        total_tokens: row.get::<_, i64>(14)?.max(0) as u64,
        cost_usd: row.get(15)?,
        retention_class: row.get(16)?,
    })
}

fn status_to_str(status: &ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Running => "running",
        ExecutionStatus::Succeeded => "succeeded",
        ExecutionStatus::Failed => "failed",
    }
}

fn status_from_str(status: &str) -> ExecutionStatus {
    match status {
        "succeeded" => ExecutionStatus::Succeeded,
        "failed" => ExecutionStatus::Failed,
        _ => ExecutionStatus::Running,
    }
}
