//! Per-session ledger — turn caps, per-tool budgets, and the grant set, owned by
//! the RUNTIME and keyed by `session_id` (moved off the host, constitution #2).
//!
//! In the dumb-pipe model the host POSTs the artifact once and pipes turns; it no
//! longer tracks per-conversation turn counts, tool budgets, or grants. The
//! runtime holds them here so a one-execution-per-session conversation enforces
//! caps across its re-armed turns without any host bookkeeping. The ledger is a
//! process-global registry keyed by session id; an `Arc<SessionLedger>` is also
//! threaded onto `ExecutionContext` (inherited by child contexts).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Per-session caps/budgets/grants.
#[derive(Debug)]
pub struct SessionLedger {
    /// Max substantive turns for the session (`None` = unbounded).
    turn_cap: Option<usize>,
    turns_used: AtomicUsize,
    /// Per-tool call budget for the whole session (`None` entry = unbounded).
    tool_budgets: HashMap<String, usize>,
    tool_consumed: Mutex<HashMap<String, usize>>,
    /// Capability grant set seeded from the start request's `admit_capabilities`.
    grants: HashSet<String>,
}

impl SessionLedger {
    pub fn new(
        turn_cap: Option<usize>,
        tool_budgets: HashMap<String, usize>,
        grants: HashSet<String>,
    ) -> Self {
        Self {
            turn_cap,
            turns_used: AtomicUsize::new(0),
            tool_budgets,
            tool_consumed: Mutex::new(HashMap::new()),
            grants,
        }
    }

    /// Charge one turn; returns the new turn count, or `Err` if it would exceed
    /// the cap (the turn must be refused — fail-closed, constitution #9 keeps the
    /// runtime from serving unbounded turns).
    pub fn charge_turn(&self) -> Result<usize, String> {
        let next = self.turns_used.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(cap) = self.turn_cap
            && next > cap
        {
            self.turns_used.fetch_sub(1, Ordering::SeqCst);
            return Err(format!("session turn cap reached ({cap})"));
        }
        Ok(next)
    }

    pub fn turns_used(&self) -> usize {
        self.turns_used.load(Ordering::SeqCst)
    }

    /// Charge one call against a tool's session budget. Returns `false` (without
    /// charging) when the budget is exhausted; `true` (and charges) otherwise.
    pub fn charge_tool(&self, tool: &str) -> bool {
        let mut consumed = self.tool_consumed.lock().expect("ledger poisoned");
        let used = consumed.entry(tool.to_string()).or_insert(0);
        if let Some(budget) = self.tool_budgets.get(tool)
            && *used >= *budget
        {
            return false;
        }
        *used += 1;
        true
    }

    /// Whether the session's grant set admits `capability`.
    pub fn admits(&self, capability: &str) -> bool {
        self.grants.contains(capability)
    }

    pub fn grants(&self) -> &HashSet<String> {
        &self.grants
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<SessionLedger>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<SessionLedger>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Seed (or return the existing) ledger for `session_id`. Idempotent: a session's
/// caps are fixed at first execution start and reused across re-armed turns.
pub fn seed(session_id: &str, ledger: SessionLedger) -> Arc<SessionLedger> {
    let mut guard = registry().lock().expect("session ledger registry poisoned");
    guard
        .entry(session_id.to_string())
        .or_insert_with(|| Arc::new(ledger))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_cap_is_enforced() {
        let l = SessionLedger::new(Some(2), HashMap::new(), HashSet::new());
        assert_eq!(l.charge_turn().unwrap(), 1);
        assert_eq!(l.charge_turn().unwrap(), 2);
        assert!(l.charge_turn().is_err(), "third turn exceeds cap of 2");
        assert_eq!(l.turns_used(), 2, "rejected turn is not counted");
    }

    #[test]
    fn unbounded_turns_when_no_cap() {
        let l = SessionLedger::new(None, HashMap::new(), HashSet::new());
        for _ in 0..100 {
            assert!(l.charge_turn().is_ok());
        }
    }

    #[test]
    fn tool_budget_is_enforced() {
        let mut budgets = HashMap::new();
        budgets.insert("lookup".to_string(), 2);
        let l = SessionLedger::new(None, budgets, HashSet::new());
        assert!(l.charge_tool("lookup"));
        assert!(l.charge_tool("lookup"));
        assert!(!l.charge_tool("lookup"), "third lookup exceeds budget of 2");
        assert!(l.charge_tool("other"), "untracked tool is unbounded");
    }

    #[test]
    fn grants_admit_set() {
        let mut grants = HashSet::new();
        grants.insert("write_file".to_string());
        let l = SessionLedger::new(None, HashMap::new(), grants);
        assert!(l.admits("write_file"));
        assert!(!l.admits("delete_file"));
    }

    #[test]
    fn registry_seed_is_idempotent() {
        let id = "sess-ledger-test-unique";
        let first = seed(id, SessionLedger::new(Some(5), HashMap::new(), HashSet::new()));
        let second = seed(id, SessionLedger::new(Some(99), HashMap::new(), HashSet::new()));
        // Idempotent: the second seed returns the first ledger (cap stays 5).
        assert!(Arc::ptr_eq(&first, &second));
        assert!(get(id).is_some());
        remove(id);
        assert!(get(id).is_none());
    }
}
