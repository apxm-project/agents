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
use std::sync::{Arc, Mutex, OnceLock};

use thiserror::Error;

/// Typed session-ledger failures surfaced to runtime and Server callers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionLedgerError {
    /// The next turn would exceed the configured session cap.
    #[error("session turn cap reached ({cap})")]
    TurnCapReached { cap: usize },
    /// The next tool call would exceed the configured session budget.
    #[error("session tool budget exhausted for '{tool}' ({budget})")]
    ToolBudgetExhausted { tool: String, budget: usize },
    /// The durable store could not be initialized.
    #[error("failed to initialize session ledger at {path}: {message}")]
    DurableInitialization { path: String, message: String },
    /// A durable session row could not be read.
    #[error("failed to read durable session ledger for '{session_id}': {message}")]
    DurableRead { session_id: String, message: String },
    /// A durable session row could not be written.
    #[error("failed to write durable session ledger for '{session_id}': {message}")]
    DurableWrite { session_id: String, message: String },
    /// A durable session row exists but is not valid for the current typed schema.
    #[error("invalid durable session ledger for '{session_id}': {message}")]
    CorruptRecord { session_id: String, message: String },
}

#[derive(Debug, Default)]
struct LedgerState {
    turns_used: usize,
    tool_consumed: HashMap<String, usize>,
}

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
    /// Per-tool call budget for the whole session (`None` entry = unbounded).
    tool_budgets: HashMap<String, usize>,
    /// Serializes each proposed charge with its durable write. The in-memory
    /// counters move only after SQLite accepts the corresponding next state.
    state: Mutex<LedgerState>,
}

impl SessionLedger {
    pub fn new(turn_cap: Option<usize>, tool_budgets: HashMap<String, usize>) -> Self {
        Self {
            session_id: OnceLock::new(),
            turn_cap,
            tool_budgets,
            state: Mutex::new(LedgerState::default()),
        }
    }

    /// Charge one turn; returns the new turn count, or `Err` if it would exceed
    /// the cap (the turn must be refused — fail-closed, constitution #9 keeps the
    /// runtime from serving unbounded turns).
    pub fn charge_turn(&self) -> Result<usize, SessionLedgerError> {
        let mut state = self.state.lock().expect("session ledger poisoned");
        let next = state.turns_used + 1;
        if let Some(cap) = self.turn_cap
            && next > cap
        {
            return Err(SessionLedgerError::TurnCapReached { cap });
        }
        self.persist_if_durable(next, &state.tool_consumed)?;
        state.turns_used = next;
        Ok(next)
    }

    pub fn turns_used(&self) -> usize {
        self.state
            .lock()
            .expect("session ledger poisoned")
            .turns_used
    }

    /// Charge one call against a tool's session budget and return its new count.
    pub fn charge_tool(&self, tool: &str) -> Result<usize, SessionLedgerError> {
        let mut state = self.state.lock().expect("session ledger poisoned");
        let used = state.tool_consumed.get(tool).copied().unwrap_or(0);
        if let Some(&budget) = self.tool_budgets.get(tool)
            && used >= budget
        {
            return Err(SessionLedgerError::ToolBudgetExhausted {
                tool: tool.to_string(),
                budget,
            });
        }
        let next = used + 1;
        let mut next_consumed = state.tool_consumed.clone();
        next_consumed.insert(tool.to_string(), next);
        self.persist_if_durable(state.turns_used, &next_consumed)?;
        state.tool_consumed = next_consumed;
        Ok(next)
    }

    /// Persist current `turns_used`/`tool_consumed` durably, if both this
    /// ledger has been seeded with a `session_id` and a durable store is
    /// configured (`durable::init`). No-op otherwise (volatile mode).
    fn persist_if_durable(
        &self,
        turns_used: usize,
        tool_consumed: &HashMap<String, usize>,
    ) -> Result<(), SessionLedgerError> {
        let Some(session_id) = self.session_id.get() else {
            return Ok(());
        };
        durable::persist(session_id, turns_used, tool_consumed)
    }

    /// Remaining per-tool call budget for the session (`None` entry omitted).
    pub fn tool_budgets_remaining(&self) -> HashMap<String, usize> {
        let state = self.state.lock().expect("session ledger poisoned");
        self.tool_budgets
            .iter()
            .map(|(tool, budget)| {
                let used = state.tool_consumed.get(tool).copied().unwrap_or(0);
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
}

/// Charge one substantive turn when a session recv wakes, before re-arm delivers
/// the user message. The recv re-arm path calls this so turn caps are enforced
/// server-side without any host-side counting.
pub fn charge_turn_for_wake(session_id: &str) -> Result<TurnChargeOutcome, SessionLedgerError> {
    match get(session_id) {
        None => Ok(TurnChargeOutcome::NoLedger),
        Some(ledger) => ledger.charge_turn().map(TurnChargeOutcome::Charged),
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
/// durable read or typed decode failure is returned without inserting a fresh
/// in-memory ledger, so restart cannot silently reset counters to zero.
pub fn seed(
    session_id: &str,
    ledger: SessionLedger,
) -> Result<Arc<SessionLedger>, SessionLedgerError> {
    let mut guard = registry().lock().expect("session ledger registry poisoned");
    if let Some(existing) = guard.get(session_id) {
        return Ok(Arc::clone(existing));
    }

    let _ = ledger.session_id.set(session_id.to_string());
    if let Some(loaded) = durable::load(session_id)? {
        *ledger.state.lock().expect("session ledger poisoned") = loaded;
    }
    let ledger = Arc::new(ledger);
    guard.insert(session_id.to_string(), Arc::clone(&ledger));
    Ok(ledger)
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
    use rusqlite::{Connection, OptionalExtension, params};

    use super::{LedgerState, SessionLedgerError};

    #[cfg(feature = "sqlite")]
    fn slot() -> &'static Mutex<Option<Connection>> {
        static SLOT: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(None))
    }

    #[cfg(test)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub(super) enum FaultOperation {
        Read,
        Write,
    }

    #[cfg(test)]
    fn faults() -> &'static Mutex<std::collections::HashSet<(FaultOperation, String)>> {
        static FAULTS: OnceLock<Mutex<std::collections::HashSet<(FaultOperation, String)>>> =
            OnceLock::new();
        FAULTS.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
    }

    #[cfg(test)]
    pub(super) fn fail_next_for_test(operation: FaultOperation, session_id: &str) {
        faults()
            .lock()
            .expect("session ledger fault injector poisoned")
            .insert((operation, session_id.to_string()));
    }

    #[cfg(test)]
    fn inject_failure(operation: FaultOperation, session_id: &str) -> bool {
        faults()
            .lock()
            .expect("session ledger fault injector poisoned")
            .remove(&(operation, session_id.to_string()))
    }

    /// Open (creating if needed) the durable session-ledger store at `path`.
    /// Idempotent — safe to call again after a simulated restart to reopen
    /// the same on-disk file (tests use this to model "the process died and
    /// came back").
    #[cfg(feature = "sqlite")]
    pub fn init(path: &Path) -> Result<(), SessionLedgerError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                SessionLedgerError::DurableInitialization {
                    path: path.display().to_string(),
                    message: error.to_string(),
                }
            })?;
        }
        let conn =
            Connection::open(path).map_err(|error| SessionLedgerError::DurableInitialization {
                path: path.display().to_string(),
                message: error.to_string(),
            })?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| SessionLedgerError::DurableInitialization {
                path: path.display().to_string(),
                message: error.to_string(),
            })?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_ledger (
                session_id TEXT PRIMARY KEY,
                turns_used INTEGER NOT NULL,
                tool_consumed_json TEXT NOT NULL
            );",
        )
        .map_err(|error| SessionLedgerError::DurableInitialization {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        *slot().lock().expect("session ledger durable slot poisoned") = Some(conn);
        Ok(())
    }

    #[cfg(not(feature = "sqlite"))]
    pub fn init(path: &Path) -> Result<(), SessionLedgerError> {
        Err(SessionLedgerError::DurableInitialization {
            path: path.display().to_string(),
            message: "durable session-ledger persistence requires the 'sqlite' feature".to_string(),
        })
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
    ) -> Result<(), SessionLedgerError> {
        let guard = slot().lock().expect("session ledger durable slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return Ok(());
        };
        #[cfg(test)]
        if inject_failure(FaultOperation::Write, session_id) {
            return Err(SessionLedgerError::DurableWrite {
                session_id: session_id.to_string(),
                message: "injected write failure".to_string(),
            });
        }
        let json = serde_json::to_string(tool_consumed).map_err(|error| {
            SessionLedgerError::DurableWrite {
                session_id: session_id.to_string(),
                message: error.to_string(),
            }
        })?;
        conn.execute(
            "INSERT INTO session_ledger (session_id, turns_used, tool_consumed_json) VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET turns_used = excluded.turns_used, tool_consumed_json = excluded.tool_consumed_json",
            params![session_id, turns_used.min(i64::MAX as usize) as i64, json],
        )
        .map_err(|error| SessionLedgerError::DurableWrite {
            session_id: session_id.to_string(),
            message: error.to_string(),
        })?;
        Ok(())
    }

    #[cfg(not(feature = "sqlite"))]
    pub(super) fn persist(
        _session_id: &str,
        _turns_used: usize,
        _tool_consumed: &HashMap<String, usize>,
    ) -> Result<(), SessionLedgerError> {
        Ok(())
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn load(session_id: &str) -> Result<Option<LedgerState>, SessionLedgerError> {
        let guard = slot().lock().expect("session ledger durable slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return Ok(None);
        };
        #[cfg(test)]
        if inject_failure(FaultOperation::Read, session_id) {
            return Err(SessionLedgerError::DurableRead {
                session_id: session_id.to_string(),
                message: "injected read failure".to_string(),
            });
        }
        let row = conn
            .query_row(
                "SELECT turns_used, tool_consumed_json FROM session_ledger WHERE session_id = ?1",
                params![session_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| SessionLedgerError::DurableRead {
                session_id: session_id.to_string(),
                message: error.to_string(),
            })?;
        let Some((turns_used, tool_consumed_json)) = row else {
            return Ok(None);
        };
        let turns_used =
            usize::try_from(turns_used).map_err(|_| SessionLedgerError::CorruptRecord {
                session_id: session_id.to_string(),
                message: "turns_used is negative or out of range".to_string(),
            })?;
        let tool_consumed = serde_json::from_str::<HashMap<String, usize>>(&tool_consumed_json)
            .map_err(|error| SessionLedgerError::CorruptRecord {
                session_id: session_id.to_string(),
                message: error.to_string(),
            })?;
        Ok(Some(LedgerState {
            turns_used,
            tool_consumed,
        }))
    }

    #[cfg(not(feature = "sqlite"))]
    pub(super) fn load(_session_id: &str) -> Result<Option<LedgerState>, SessionLedgerError> {
        Ok(None)
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
        assert_eq!(l.charge_tool("lookup").unwrap(), 1);
        assert_eq!(l.charge_tool("lookup").unwrap(), 2);
        assert_eq!(
            l.charge_tool("lookup"),
            Err(SessionLedgerError::ToolBudgetExhausted {
                tool: "lookup".to_string(),
                budget: 2,
            })
        );
        assert_eq!(l.charge_tool("other").unwrap(), 1);
    }

    #[test]
    fn registry_seed_is_idempotent() {
        let id = "sess-ledger-test-unique";
        let first = seed(id, SessionLedger::new(Some(5), HashMap::new())).unwrap();
        let second = seed(id, SessionLedger::new(Some(99), HashMap::new())).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(get(id).is_some());
        remove(id);
        assert!(get(id).is_none());
    }

    #[test]
    fn charge_turn_for_wake_enforces_cap_via_registry() {
        let id = "sess-charge-wake-cap";
        seed(id, SessionLedger::new(Some(1), HashMap::new())).unwrap();
        assert_eq!(
            charge_turn_for_wake(id),
            Ok(TurnChargeOutcome::Charged(1)),
            "first wake charges turn 1"
        );
        assert_eq!(
            charge_turn_for_wake(id),
            Err(SessionLedgerError::TurnCapReached { cap: 1 })
        );
        remove(id);
    }

    #[test]
    fn charge_turn_for_wake_without_ledger() {
        assert_eq!(
            charge_turn_for_wake("sess-no-ledger-xyz"),
            Ok(TurnChargeOutcome::NoLedger)
        );
    }

    #[test]
    fn turns_remaining_tracks_cap() {
        let l = SessionLedger::new(Some(3), HashMap::new());
        assert_eq!(l.turns_remaining(), Some(3));
        assert_eq!(l.charge_turn().unwrap(), 1);
        assert_eq!(l.turns_remaining(), Some(2));
    }

    #[test]
    fn durable_failures_and_concurrent_restart_are_failure_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("session_ledger.sqlite");
        let session_id = "sess-durable-failure-atomic";

        durable::init(&db_path).expect("open durable session-ledger store");
        let ledger = seed(session_id, SessionLedger::new(Some(16), HashMap::new())).unwrap();

        durable::fail_next_for_test(durable::FaultOperation::Write, session_id);
        assert!(matches!(
            ledger.charge_turn(),
            Err(SessionLedgerError::DurableWrite { .. })
        ));
        assert_eq!(ledger.turns_used(), 0);

        let tool_session_id = "sess-tool-write-failure-atomic";
        let mut budgets = HashMap::new();
        budgets.insert("lookup".to_string(), 1);
        let tool_ledger = seed(tool_session_id, SessionLedger::new(None, budgets)).unwrap();
        durable::fail_next_for_test(durable::FaultOperation::Write, tool_session_id);
        assert!(matches!(
            tool_ledger.charge_tool("lookup"),
            Err(SessionLedgerError::DurableWrite { .. })
        ));
        assert_eq!(tool_ledger.tool_budgets_remaining()["lookup"], 1);
        assert_eq!(tool_ledger.charge_tool("lookup").unwrap(), 1);

        let workers: Vec<_> = (0..8)
            .map(|_| {
                let ledger = Arc::clone(&ledger);
                std::thread::spawn(move || ledger.charge_turn().unwrap())
            })
            .collect();
        let mut charged: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        charged.sort_unstable();
        assert_eq!(charged, (1..=8).collect::<Vec<_>>());
        assert_eq!(ledger.turns_used(), 8);

        durable::close_for_test();
        remove(session_id);
        durable::init(&db_path).expect("reopen durable session-ledger store after restart");
        let rehydrated = seed(session_id, SessionLedger::new(Some(16), HashMap::new())).unwrap();
        assert_eq!(rehydrated.turns_used(), 8);

        remove(session_id);
        durable::fail_next_for_test(durable::FaultOperation::Read, session_id);
        assert!(matches!(
            seed(session_id, SessionLedger::new(Some(16), HashMap::new())),
            Err(SessionLedgerError::DurableRead { .. })
        ));
        assert!(get(session_id).is_none());

        let corrupt_session_id = "sess-durable-corrupt-row";
        {
            let guard = durable::slot_for_test();
            let conn = guard.as_ref().unwrap();
            conn.execute(
                "INSERT INTO session_ledger (session_id, turns_used, tool_consumed_json) VALUES (?1, ?2, ?3)",
                rusqlite::params![corrupt_session_id, 1i64, "{not valid json"],
            )
            .unwrap();
        }
        assert!(matches!(
            seed(
                corrupt_session_id,
                SessionLedger::new(Some(5), HashMap::new())
            ),
            Err(SessionLedgerError::CorruptRecord { .. })
        ));
        assert!(get(corrupt_session_id).is_none());

        durable::close_for_test();
        remove(session_id);
        remove(tool_session_id);
        remove(corrupt_session_id);
    }
}
