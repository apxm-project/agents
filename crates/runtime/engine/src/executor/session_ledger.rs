//! Per-session ledger — turn caps and per-tool budgets, owned by
//! the RUNTIME and keyed by `session_id` (moved off the host, constitution #2).
//!
//! In the dumb-pipe model the host POSTs the artifact once and pipes turns; it no
//! longer tracks per-conversation turn counts or tool budgets. The runtime
//! holds them here so a one-execution-per-session conversation enforces
//! caps across its re-armed turns without any host bookkeeping. The ledger is a
//! process-global registry keyed by session id; an `Arc<SessionLedger>` is also
//! threaded onto `ExecutionContext` (inherited by child contexts).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Per-session caps/budgets.
#[derive(Debug)]
pub struct SessionLedger {
    /// Set once by [`seed`] so `charge_turn`/`charge_tool` can self-persist
    /// (and, on first seed after a restart, self-rehydrate) without every
    /// caller threading `session_id` through. `OnceLock` because a ledger's
    /// key is fixed for its lifetime.
    session_id: OnceLock<String>,
    /// Max substantive turns for the session (`None` = unbounded).
    turn_cap: Option<usize>,
    turns_used: AtomicUsize,
    /// Per-tool call budget for the whole session (`None` entry = unbounded).
    tool_budgets: HashMap<String, usize>,
    tool_consumed: Mutex<HashMap<String, usize>>,
    /// Fail-closed flag: set when a durable rehydration found a persisted row
    /// for this session that could not be parsed with confidence. A poisoned
    /// ledger denies every further turn/tool charge rather than silently
    /// resuming from zero (constitution: a restart must never be a free
    /// cap refill — see the restart-state reconstruction invariant).
    poisoned: AtomicBool,
}

impl SessionLedger {
    pub fn new(turn_cap: Option<usize>, tool_budgets: HashMap<String, usize>) -> Self {
        Self {
            session_id: OnceLock::new(),
            turn_cap,
            turns_used: AtomicUsize::new(0),
            tool_budgets,
            tool_consumed: Mutex::new(HashMap::new()),
            poisoned: AtomicBool::new(false),
        }
    }

    /// Charge one turn; returns the new turn count, or `Err` if it would exceed
    /// the cap (the turn must be refused — fail-closed, constitution #9 keeps the
    /// runtime from serving unbounded turns).
    pub fn charge_turn(&self) -> Result<usize, String> {
        if self.poisoned.load(Ordering::SeqCst) {
            return Err(
                "session ledger could not be durably reconstructed after restart; denying turn (fail-closed)"
                    .to_string(),
            );
        }
        let next = self.turns_used.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(cap) = self.turn_cap
            && next > cap
        {
            self.turns_used.fetch_sub(1, Ordering::SeqCst);
            return Err(format!("session turn cap reached ({cap})"));
        }
        self.persist_if_durable();
        Ok(next)
    }

    pub fn turns_used(&self) -> usize {
        self.turns_used.load(Ordering::SeqCst)
    }

    /// Whether this ledger's durable state could not be reconstructed with
    /// confidence (fail-closed: every further turn/tool charge is denied).
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::SeqCst)
    }

    /// Charge one call against a tool's session budget. Returns `false` (without
    /// charging) when the budget is exhausted or the ledger is poisoned;
    /// `true` (and charges) otherwise.
    pub fn charge_tool(&self, tool: &str) -> bool {
        if self.poisoned.load(Ordering::SeqCst) {
            return false;
        }
        let charged = {
            let mut consumed = self.tool_consumed.lock().expect("ledger poisoned");
            let used = consumed.entry(tool.to_string()).or_insert(0);
            if let Some(budget) = self.tool_budgets.get(tool)
                && *used >= *budget
            {
                false
            } else {
                *used += 1;
                true
            }
        };
        if charged {
            self.persist_if_durable();
        }
        charged
    }

    /// Persist current `turns_used`/`tool_consumed` durably, if both this
    /// ledger has been seeded with a `session_id` and a durable store is
    /// configured (`durable::init`). No-op otherwise (volatile mode).
    fn persist_if_durable(&self) {
        let Some(session_id) = self.session_id.get() else {
            return;
        };
        let consumed = self.tool_consumed.lock().expect("ledger poisoned").clone();
        durable::persist(session_id, self.turns_used(), &consumed);
    }

    /// Remaining per-tool call budget for the session (`None` entry omitted).
    pub fn tool_budgets_remaining(&self) -> HashMap<String, usize> {
        let consumed = self.tool_consumed.lock().expect("ledger poisoned");
        self.tool_budgets
            .iter()
            .map(|(tool, budget)| {
                let used = consumed.get(tool).copied().unwrap_or(0);
                (tool.clone(), budget.saturating_sub(used))
            })
            .collect()
    }

    /// Whether the next substantive turn would exceed the session turn cap.
    pub fn would_exceed_turn_cap(&self) -> bool {
        self.turn_cap.is_some_and(|cap| self.turns_used() >= cap)
    }

    pub fn turn_cap(&self) -> Option<usize> {
        self.turn_cap
    }

    /// Remaining substantive turns before the cap, if bounded.
    pub fn turns_remaining(&self) -> Option<usize> {
        self.turn_cap
            .map(|cap| cap.saturating_sub(self.turns_used()))
    }
}

/// Outcome of charging a substantive turn at the session recv wake / re-arm seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnChargeOutcome {
    /// No ledger was seeded for this session (non-session or host omitted caps).
    NoLedger,
    /// Turn charged; returns the new running turn count.
    Charged(usize),
    /// Session turn cap would be exceeded — the turn must not proceed (fail-closed).
    CapExceeded(String),
}

/// Charge one substantive turn when a session recv wakes, before re-arm delivers
/// the user message. The recv re-arm path calls this so turn caps are enforced
/// server-side without any host-side counting.
pub fn charge_turn_for_wake(session_id: &str) -> TurnChargeOutcome {
    match get(session_id) {
        None => TurnChargeOutcome::NoLedger,
        Some(ledger) => match ledger.charge_turn() {
            Ok(n) => TurnChargeOutcome::Charged(n),
            Err(msg) => TurnChargeOutcome::CapExceeded(msg),
        },
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<SessionLedger>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<SessionLedger>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Seed (or return the existing) ledger for `session_id`. Idempotent: a session's
/// caps are fixed at first execution start and reused across re-armed turns.
///
/// On first seed (no in-memory entry yet — which is also true right after a
/// process restart, since the registry is a fresh, empty process-global map),
/// rehydrates `turns_used`/`tool_consumed` from the durable store if one is
/// configured ([`durable::init`]) and a row exists for `session_id`. A
/// persisted row that fails to parse poisons the ledger (fail-closed) instead
/// of silently starting the count over at zero.
pub fn seed(session_id: &str, ledger: SessionLedger) -> Arc<SessionLedger> {
    let mut guard = registry().lock().expect("session ledger registry poisoned");
    guard
        .entry(session_id.to_string())
        .or_insert_with(|| {
            let _ = ledger.session_id.set(session_id.to_string());
            match durable::load(session_id) {
                durable::LoadOutcome::None => {}
                durable::LoadOutcome::Found {
                    turns_used,
                    tool_consumed,
                } => {
                    ledger.turns_used.store(turns_used, Ordering::SeqCst);
                    *ledger.tool_consumed.lock().expect("ledger poisoned") = tool_consumed;
                }
                durable::LoadOutcome::Corrupt => {
                    tracing::error!(
                        session_id,
                        "durable session-ledger row could not be parsed; denying further turns/tools for this session (fail-closed)"
                    );
                    ledger.poisoned.store(true, Ordering::SeqCst);
                }
            }
            Arc::new(ledger)
        })
        .clone()
}

/// Look up the ledger for `session_id`, if any.
pub fn get(session_id: &str) -> Option<Arc<SessionLedger>> {
    registry()
        .lock()
        .expect("session ledger registry poisoned")
        .get(session_id)
        .cloned()
}

/// Drop a session's ledger (call when the session's execution ends).
pub fn remove(session_id: &str) {
    registry()
        .lock()
        .expect("session ledger registry poisoned")
        .remove(session_id);
}

/// Durable session-ledger backing (turn caps / tool budgets survive a
/// restart), mirroring `apxm-server`'s `CheckpointStore` pattern: a SQLite
/// row per session, upserted on every successful charge, boot-loaded lazily
/// (on first [`seed`] for a session, not eagerly for the whole table — the
/// runtime doesn't know which sessions matter until a caller re-seeds them).
pub mod durable {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    #[cfg(feature = "sqlite")]
    use rusqlite::{Connection, params};

    #[cfg(feature = "sqlite")]
    fn slot() -> &'static Mutex<Option<Connection>> {
        static SLOT: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(None))
    }

    /// Outcome of a durable lookup for one session.
    pub(super) enum LoadOutcome {
        /// No durable store configured, or no row for this session (a
        /// genuinely new session — not an ambiguous restart case).
        None,
        Found {
            turns_used: usize,
            tool_consumed: HashMap<String, usize>,
        },
        /// A row exists but could not be parsed with confidence.
        Corrupt,
    }

    /// Open (creating if needed) the durable session-ledger store at `path`.
    /// Idempotent — safe to call again after a simulated restart to reopen
    /// the same on-disk file (tests use this to model "the process died and
    /// came back").
    #[cfg(feature = "sqlite")]
    pub fn init(path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_ledger (
                session_id TEXT PRIMARY KEY,
                turns_used INTEGER NOT NULL,
                tool_consumed_json TEXT NOT NULL
            );",
        )
        .map_err(|e| e.to_string())?;
        *slot().lock().expect("session ledger durable slot poisoned") = Some(conn);
        Ok(())
    }

    #[cfg(not(feature = "sqlite"))]
    pub fn init(_path: &Path) -> Result<(), String> {
        Err("durable session-ledger persistence requires the 'sqlite' feature".to_string())
    }

    /// Drop the durable connection (test-only: simulates the process dying —
    /// the on-disk file survives, only the in-process handle is dropped).
    #[cfg(feature = "sqlite")]
    pub fn close_for_test() {
        *slot().lock().expect("session ledger durable slot poisoned") = None;
    }

    /// Test-only direct access to the open connection (used to write a
    /// deliberately-corrupt row and exercise the fail-closed poison path).
    #[cfg(all(test, feature = "sqlite"))]
    pub(super) fn slot_for_test() -> std::sync::MutexGuard<'static, Option<Connection>> {
        slot().lock().expect("session ledger durable slot poisoned")
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn persist(
        session_id: &str,
        turns_used: usize,
        tool_consumed: &HashMap<String, usize>,
    ) {
        let guard = slot().lock().expect("session ledger durable slot poisoned");
        let Some(conn) = guard.as_ref() else { return };
        let Ok(json) = serde_json::to_string(tool_consumed) else {
            return;
        };
        let _ = conn.execute(
            "INSERT INTO session_ledger (session_id, turns_used, tool_consumed_json) VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET turns_used = excluded.turns_used, tool_consumed_json = excluded.tool_consumed_json",
            params![session_id, turns_used.min(i64::MAX as usize) as i64, json],
        );
    }

    #[cfg(not(feature = "sqlite"))]
    pub(super) fn persist(
        _session_id: &str,
        _turns_used: usize,
        _tool_consumed: &HashMap<String, usize>,
    ) {
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn load(session_id: &str) -> LoadOutcome {
        let guard = slot().lock().expect("session ledger durable slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return LoadOutcome::None;
        };
        let row: Option<(i64, String)> = conn
            .query_row(
                "SELECT turns_used, tool_consumed_json FROM session_ledger WHERE session_id = ?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let Some((turns_used, tool_consumed_json)) = row else {
            return LoadOutcome::None;
        };
        match serde_json::from_str::<HashMap<String, usize>>(&tool_consumed_json) {
            Ok(tool_consumed) => LoadOutcome::Found {
                turns_used: usize::try_from(turns_used).unwrap_or(0),
                tool_consumed,
            },
            Err(_) => LoadOutcome::Corrupt,
        }
    }

    #[cfg(not(feature = "sqlite"))]
    pub(super) fn load(_session_id: &str) -> LoadOutcome {
        LoadOutcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_cap_is_enforced() {
        let l = SessionLedger::new(Some(2), HashMap::new());
        assert_eq!(l.charge_turn().unwrap(), 1);
        assert_eq!(l.charge_turn().unwrap(), 2);
        assert!(l.charge_turn().is_err(), "third turn exceeds cap of 2");
        assert_eq!(l.turns_used(), 2, "rejected turn is not counted");
    }

    #[test]
    fn unbounded_turns_when_no_cap() {
        let l = SessionLedger::new(None, HashMap::new());
        for _ in 0..100 {
            assert!(l.charge_turn().is_ok());
        }
    }

    #[test]
    fn tool_budget_is_enforced() {
        let mut budgets = HashMap::new();
        budgets.insert("lookup".to_string(), 2);
        let l = SessionLedger::new(None, budgets);
        assert!(l.charge_tool("lookup"));
        assert!(l.charge_tool("lookup"));
        assert!(!l.charge_tool("lookup"), "third lookup exceeds budget of 2");
        assert!(l.charge_tool("other"), "untracked tool is unbounded");
    }

    #[test]
    fn registry_seed_is_idempotent() {
        let id = "sess-ledger-test-unique";
        let first = seed(id, SessionLedger::new(Some(5), HashMap::new()));
        let second = seed(id, SessionLedger::new(Some(99), HashMap::new()));
        // Idempotent: the second seed returns the first ledger (cap stays 5).
        assert!(Arc::ptr_eq(&first, &second));
        assert!(get(id).is_some());
        remove(id);
        assert!(get(id).is_none());
    }

    #[test]
    fn charge_turn_for_wake_enforces_cap_via_registry() {
        let id = "sess-charge-wake-cap";
        seed(id, SessionLedger::new(Some(1), HashMap::new()));
        assert_eq!(
            charge_turn_for_wake(id),
            TurnChargeOutcome::Charged(1),
            "first wake charges turn 1"
        );
        assert!(
            matches!(charge_turn_for_wake(id), TurnChargeOutcome::CapExceeded(_)),
            "second wake exceeds cap of 1"
        );
        remove(id);
    }

    #[test]
    fn charge_turn_for_wake_without_ledger() {
        assert_eq!(
            charge_turn_for_wake("sess-no-ledger-xyz"),
            TurnChargeOutcome::NoLedger
        );
    }

    #[test]
    fn turns_remaining_tracks_cap() {
        let l = SessionLedger::new(Some(3), HashMap::new());
        assert_eq!(l.turns_remaining(), Some(3));
        assert_eq!(l.charge_turn().unwrap(), 1);
        assert_eq!(l.turns_remaining(), Some(2));
    }

    /// Positive + negative/regression: charge N of M turns, simulate a
    /// process restart (drop the durable connection and the in-memory
    /// registry, then reopen the same on-disk file), rehydrate via `seed`,
    /// and assert `turns_used() == N` with the cap still enforced — not
    /// reset. Also proves the negative: a naive from-scratch `SessionLedger`
    /// for the same session (ignoring durable state) would incorrectly
    /// report 0 and let the cap be exceeded — the exact fail-open regression
    /// this durability layer exists to prevent.
    ///
    /// Single test (not split across several `#[test]` fns) because the
    /// durable backing is one process-global slot; `cargo test` runs test
    /// functions concurrently by default; splitting this across tests would
    /// race on that shared slot.
    #[test]
    fn restart_preserves_turn_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("session_ledger.sqlite");
        let session_id = "sess-restart-preserves-cap";

        durable::init(&db_path).expect("open durable session-ledger store");
        let ledger = seed(session_id, SessionLedger::new(Some(3), HashMap::new()));
        assert_eq!(ledger.charge_turn().unwrap(), 1);
        assert_eq!(ledger.charge_turn().unwrap(), 2);
        assert_eq!(ledger.turns_used(), 2);

        // Simulate a process restart: drop the durable connection AND the
        // in-memory registry entry (a fresh process has neither), then
        // reopen the same on-disk file as boot would.
        durable::close_for_test();
        remove(session_id);
        durable::init(&db_path).expect("reopen durable session-ledger store after restart");

        // Regression guard: a naive rebuild (constructing a fresh ledger and
        // reading it back WITHOUT going through `seed`'s rehydration) would
        // report 0 turns used, not 2 — prove that's what "naive" looks like
        // so the contrast with the real rehydration path below is explicit.
        let naive = SessionLedger::new(Some(3), HashMap::new());
        assert_eq!(
            naive.turns_used(),
            0,
            "sanity: an unrehydrated fresh ledger must NOT already show 2 turns used"
        );

        // Real rehydration path: `seed` seeing no in-memory entry for this
        // session pulls the persisted row before handing back the ledger.
        let rehydrated = seed(session_id, SessionLedger::new(Some(3), HashMap::new()));
        assert_eq!(
            rehydrated.turns_used(),
            2,
            "turn count survives restart via durable rehydration, not reset to 0"
        );
        assert!(!rehydrated.is_poisoned());

        // Cap is still enforced against the RESTORED count, not a fresh cap:
        // only one more turn (3rd of 3) should be allowed before it denies.
        assert_eq!(rehydrated.charge_turn().unwrap(), 3);
        assert!(
            rehydrated.charge_turn().is_err(),
            "4th turn must be denied: restart is not a free cap refill"
        );

        // Negative: a persisted row that fails to parse poisons the ledger
        // (fail-closed) instead of silently starting the count over at zero.
        // Kept in this same test (not a separate `#[test]` fn) because the
        // durable backing is one process-global slot and `cargo test` runs
        // test fns concurrently by default; a second fn would race on it.
        let session_id = "sess-restart-corrupt-row";
        // Write a row with an unparseable `tool_consumed_json` directly,
        // simulating a truncated/corrupt durable write.
        {
            let guard = durable::slot_for_test();
            let conn = guard.as_ref().unwrap();
            conn.execute(
                "INSERT INTO session_ledger (session_id, turns_used, tool_consumed_json) VALUES (?1, ?2, ?3)",
                rusqlite::params![session_id, 1i64, "{not valid json"],
            )
            .unwrap();
        }

        let rehydrated = seed(session_id, SessionLedger::new(Some(5), HashMap::new()));
        assert!(
            rehydrated.is_poisoned(),
            "corrupt durable row must poison the ledger, not silently reset to 0"
        );
        assert!(
            rehydrated.charge_turn().is_err(),
            "poisoned ledger denies every further turn (fail-closed)"
        );
        assert!(
            !rehydrated.charge_tool("any-tool"),
            "poisoned ledger denies every further tool charge (fail-closed)"
        );

        durable::close_for_test();
        remove(session_id);
    }
}
