//! Run-history storage backend abstraction and migration notes (spec 0021).
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

/// Identifies the backing store for the run-history index.
///
/// The active backend is selected at startup from the server config.
/// Changing it requires applying the numbered sqlx migrations and a reindex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunHistoryBackend {
    /// Scan-on-read over JSON files in `$APXM_HOME/sessions/`.  The v0 default.
    JsonFiles,
    /// Derived SQLite index at `$APXM_HOME/sessions/runs.sqlite`.
    Sqlite,
    /// Postgres run-history DB (requires numbered sqlx migrations).
    Postgres(String),
}

impl RunHistoryBackend {
    /// Return a human-readable label for logging and diagnostics.
    pub fn label(&self) -> &str {
        match self {
            Self::JsonFiles => "json-files",
            Self::Sqlite => "sqlite",
            Self::Postgres(_) => "postgres",
        }
    }
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
    NotFound { id: String },
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
            Self::ArtifactExpired { artifact_ref, expired_at, retention_class } => write!(
                f,
                "artifact_expired artifact_ref={artifact_ref} expired_at={expired_at} \
                 retention_class={retention_class}"
            ),
            Self::Backend(msg) => write!(f, "run history backend error: {msg}"),
        }
    }
}

impl std::error::Error for RunHistoryError {}
