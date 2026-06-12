//! Durable rusqlite backing for the `schedule` and `manage_task` native tools.
//!
//! One SQLite file (see `apxm_core::constants::agent_tools::STORE_FILENAME`
//! under the state home) holds two tables:
//! `schedules` (armed one-shot / recurring wakeups) and `tasks` (a flat mirror
//! of the AAM goal tree so tasks survive a process restart). The pattern mirrors
//! `apxm-server`'s `CheckpointStore`: a single `Connection` behind a `Mutex`,
//! `CREATE TABLE IF NOT EXISTS` at open, WAL journaling, and short synchronous
//! statements that are never held across an `.await`.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

/// A persisted schedule row.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleRow {
    pub id: String,
    /// `once`, `recurring` (fixed interval), or `cron`.
    pub kind: String,
    /// Interval in seconds for fixed-interval recurring schedules; `None` otherwise.
    pub every_secs: Option<i64>,
    /// 5-field cron expression for cron schedules; `None` otherwise.
    pub cron: Option<String>,
    /// Unix epoch milliseconds of the next (or only) fire.
    pub next_fire_ms: i64,
    pub recurring: bool,
    /// Optional prompt carried for the woken waiter / delivery callback.
    pub prompt: Option<String>,
    /// JSON payload delivered on fire (defaults to `{}`).
    pub payload: String,
    /// `armed`, `fired`, or `cancelled`.
    pub status: String,
    pub created_at_ms: i64,
    pub last_fired_ms: Option<i64>,
}

/// A persisted task row (the indexable columns plus the full `Goal` JSON).
#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub id: String,
    pub parent_id: Option<String>,
    pub description: String,
    pub priority: i64,
    pub status: String,
    /// Explicitly-set completion policy wire token, if any.
    pub policy: Option<String>,
    /// Full serde-serialized `Goal`.
    pub json: String,
    pub updated_at_ms: i64,
}

/// Durable store shared by the schedule/manage_task capabilities and the
/// schedule firer. Cloning shares the same underlying connection.
#[derive(Clone)]
pub struct ToolsStore {
    db: Arc<Mutex<Connection>>,
}

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS schedules (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,
    every_secs    INTEGER,
    cron          TEXT,
    next_fire_ms  INTEGER NOT NULL,
    recurring     INTEGER NOT NULL,
    prompt        TEXT,
    payload       TEXT NOT NULL DEFAULT '{}',
    status        TEXT NOT NULL DEFAULT 'armed',
    created_at_ms INTEGER NOT NULL,
    last_fired_ms INTEGER
);
CREATE INDEX IF NOT EXISTS idx_sched_next_fire ON schedules(next_fire_ms);
CREATE INDEX IF NOT EXISTS idx_sched_status    ON schedules(status);
CREATE TABLE IF NOT EXISTS tasks (
    id            TEXT PRIMARY KEY,
    parent_id     TEXT,
    description   TEXT NOT NULL,
    priority      INTEGER NOT NULL,
    status        TEXT NOT NULL,
    policy        TEXT,
    json          TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
";

impl ToolsStore {
    /// Open (creating if needed) the durable store at `path`. Parent directories
    /// are created; tables are ensured; WAL is enabled.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// In-memory store for tests (no durability across reopen).
    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.db.lock().map_err(|e| e.to_string())
    }

    // ── schedules ───────────────────────────────────────────────────────

    /// Insert or replace a schedule row.
    pub fn upsert_schedule(&self, row: &ScheduleRow) -> Result<(), String> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO schedules
                (id, kind, every_secs, cron, next_fire_ms, recurring, prompt, payload,
                 status, created_at_ms, last_fired_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                every_secs = excluded.every_secs,
                cron = excluded.cron,
                next_fire_ms = excluded.next_fire_ms,
                recurring = excluded.recurring,
                prompt = excluded.prompt,
                payload = excluded.payload,
                status = excluded.status,
                last_fired_ms = excluded.last_fired_ms",
            rusqlite::params![
                row.id,
                row.kind,
                row.every_secs,
                row.cron,
                row.next_fire_ms,
                row.recurring as i64,
                row.prompt,
                row.payload,
                row.status,
                row.created_at_ms,
                row.last_fired_ms,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Fetch a single schedule by id.
    pub fn get_schedule(&self, id: &str) -> Result<Option<ScheduleRow>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT * FROM schedules WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(rusqlite::params![id], map_schedule)
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(r) => Ok(Some(r.map_err(|e| e.to_string())?)),
            None => Ok(None),
        }
    }

    /// List schedules, newest first.
    pub fn list_schedules(&self) -> Result<Vec<ScheduleRow>, String> {
        self.query_schedules("SELECT * FROM schedules ORDER BY created_at_ms DESC", &[])
    }

    /// All armed schedules ordered by next fire time (used to re-arm at boot).
    pub fn armed_schedules(&self) -> Result<Vec<ScheduleRow>, String> {
        self.query_schedules(
            "SELECT * FROM schedules WHERE status = 'armed' ORDER BY next_fire_ms ASC",
            &[],
        )
    }

    /// Armed schedules whose fire time is at or before `now_ms`.
    pub fn due_schedules(&self, now_ms: i64) -> Result<Vec<ScheduleRow>, String> {
        self.query_schedules(
            "SELECT * FROM schedules WHERE status = 'armed' AND next_fire_ms <= ?1
             ORDER BY next_fire_ms ASC",
            &[&now_ms],
        )
    }

    /// Earliest next-fire time among armed schedules, if any.
    pub fn next_armed_fire_ms(&self) -> Result<Option<i64>, String> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT MIN(next_fire_ms) FROM schedules WHERE status = 'armed'",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(|e| e.to_string())
    }

    /// Mark a schedule cancelled. Returns true if a row was affected.
    pub fn cancel_schedule(&self, id: &str) -> Result<bool, String> {
        let conn = self.lock()?;
        let n = conn
            .execute(
                "UPDATE schedules SET status = 'cancelled' WHERE id = ?1 AND status = 'armed'",
                rusqlite::params![id],
            )
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    /// Record a fire. For recurring schedules pass the next fire time to keep the
    /// row armed; pass `None` to mark a one-shot as fired.
    pub fn record_fire(
        &self,
        id: &str,
        fired_at_ms: i64,
        next_fire_ms: Option<i64>,
    ) -> Result<(), String> {
        let conn = self.lock()?;
        match next_fire_ms {
            Some(next) => conn.execute(
                "UPDATE schedules SET last_fired_ms = ?2, next_fire_ms = ?3, status = 'armed'
                 WHERE id = ?1",
                rusqlite::params![id, fired_at_ms, next],
            ),
            None => conn.execute(
                "UPDATE schedules SET last_fired_ms = ?2, status = 'fired' WHERE id = ?1",
                rusqlite::params![id, fired_at_ms],
            ),
        }
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn query_schedules(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<ScheduleRow>, String> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params, map_schedule)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    // ── tasks ───────────────────────────────────────────────────────────

    /// Insert or replace a task row.
    pub fn upsert_task(&self, row: &TaskRow) -> Result<(), String> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO tasks (id, parent_id, description, priority, status, policy, json, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                parent_id = excluded.parent_id,
                description = excluded.description,
                priority = excluded.priority,
                status = excluded.status,
                policy = excluded.policy,
                json = excluded.json,
                updated_at_ms = excluded.updated_at_ms",
            rusqlite::params![
                row.id,
                row.parent_id,
                row.description,
                row.priority,
                row.status,
                row.policy,
                row.json,
                row.updated_at_ms,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Delete a task row. Returns true if a row was removed.
    pub fn delete_task(&self, id: &str) -> Result<bool, String> {
        let conn = self.lock()?;
        let n = conn
            .execute("DELETE FROM tasks WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    /// Load all task rows (used to rehydrate the AAM goal tree at boot).
    pub fn load_tasks(&self) -> Result<Vec<TaskRow>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT id, parent_id, description, priority, status, policy, json, updated_at_ms FROM tasks")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(TaskRow {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    description: row.get(2)?,
                    priority: row.get(3)?,
                    status: row.get(4)?,
                    policy: row.get(5)?,
                    json: row.get(6)?,
                    updated_at_ms: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }
}

fn map_schedule(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleRow> {
    Ok(ScheduleRow {
        id: row.get("id")?,
        kind: row.get("kind")?,
        every_secs: row.get("every_secs")?,
        cron: row.get("cron")?,
        next_fire_ms: row.get("next_fire_ms")?,
        recurring: row.get::<_, i64>("recurring")? != 0,
        prompt: row.get("prompt")?,
        payload: row.get("payload")?,
        status: row.get("status")?,
        created_at_ms: row.get("created_at_ms")?,
        last_fired_ms: row.get("last_fired_ms")?,
    })
}

/// Current unix epoch in milliseconds.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

