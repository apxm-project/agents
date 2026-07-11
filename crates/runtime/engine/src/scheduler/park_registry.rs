//! Process-global park/wake registry — the cross-frame wake bridge for
//! artifact-resident event waits (the no-poll keystone).
//!
//! When a handler PARKS (returns [`apxm_core::error::RuntimeError::OperationParked`]),
//! the worker yields its lane + permit and registers a [`ParkWaker`] here keyed by
//! the wait's correlation id. An external event source (e.g. the server's
//! `POST /v1/checkpoints/{id}/resume`) later calls [`wake`] with that id and the
//! resolved value; the waker makes the parked node's output token(s) ready and
//! does the single compensating completion, so the node finishes exactly as a
//! normal publish would — no polling, no held worker.
//!
//! Lost-wakeup safe: a [`wake`] that arrives before [`register`] stores a
//! `Resolved` sentinel that the next `register` fires immediately.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use apxm_core::types::{Node, NodeId, TokenId, Value};

use crate::scheduler::state::SchedulerState;

/// Re-arm spec for a session conversation loop: on wake, after delivering the
/// user message, splice a fresh turn flow-call + a fresh recv (the native loop
/// keystone). Carried by the recv's [`ParkWaker`].
pub(crate) struct RearmSpec {
    pub(crate) recv_node: Arc<Node>,
    pub(crate) turn_agent: String,
    pub(crate) turn_flow: String,
    pub(crate) turn_param: String,
    /// Session id keying the per-session turn counter, and the max turns to
    /// re-arm (the recv node's `max_iterations`). Once the running count reaches
    /// the cap the loop stops re-arming, so a session is bounded.
    pub(crate) session_id: String,
    pub(crate) max_turns: u64,
}

/// Resumes one parked node by making its output tokens ready in its scheduler.
pub struct ParkWaker {
    state: Arc<SchedulerState>,
    node_id: NodeId,
    outputs: Vec<TokenId>,
    attempts: u32,
    /// When set, the parked node is a re-arming session recv: after delivering
    /// the message, splice a fresh turn + recv to continue the loop.
    rearm: Option<RearmSpec>,
}

impl ParkWaker {
    pub(crate) fn for_node(
        state: Arc<SchedulerState>,
        node_id: NodeId,
        outputs: Vec<TokenId>,
        attempts: u32,
    ) -> Self {
        Self {
            state,
            node_id,
            outputs,
            attempts,
            rearm: None,
        }
    }

    /// A re-arming waker for a session conversation-loop recv node.
    pub(crate) fn for_rearming_node(
        state: Arc<SchedulerState>,
        node_id: NodeId,
        outputs: Vec<TokenId>,
        attempts: u32,
        rearm: RearmSpec,
    ) -> Self {
        Self {
            state,
            node_id,
            outputs,
            attempts,
            rearm: Some(rearm),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(state: Arc<SchedulerState>, outputs: Vec<TokenId>) -> Self {
        let node_id = producer_for_outputs(&state, &outputs);
        Self::for_node(state, node_id, outputs, 1)
    }

    #[cfg(test)]
    pub(crate) fn new_rearming(
        state: Arc<SchedulerState>,
        outputs: Vec<TokenId>,
        rearm: RearmSpec,
    ) -> Self {
        let node_id = producer_for_outputs(&state, &outputs);
        Self::for_rearming_node(state, node_id, outputs, 1, rearm)
    }

    fn belongs_to(&self, state: &SchedulerState) -> bool {
        std::ptr::eq(self.state.as_ref(), state)
    }

    fn abandon(self) {
        self.state.exit_parked();
        self.state.record_progress();
    }

    fn finish(&self, wait_key: &str, value: Value) {
        self.state
            .wake_parked_node(self.node_id, &self.outputs, value, self.attempts);
        if self.state.is_terminal() {
            close(wait_key);
        }
    }

    fn fire(self, wait_key: &str, value: Value) {
        // SPLICE-THEN-WAKE (robust by construction). For a session loop, splice
        // the fresh turn flow-call (binding the message token) + a fresh recv
        // FIRST, raising `remaining` before any decrement; only then wake the
        // recv to deliver the message and do the single compensating completion.
        // The inverse order (wake-then-splice) momentarily drives `remaining` to
        // 0 — firing `notify_done` — between the decrement and the re-raise,
        // which the scheduler's completion check could race; splicing first never
        // opens that zero-`remaining` window (constitution #9/#10). Matches the
        // proven `recv_wake_splice_rearm_keystone` ordering.
        if let Some(spec) = &self.rearm
            && let Some(message_token) = self.outputs.first().copied()
        {
            // Session ledger turn cap: charge before re-arm so
            // turn N+1 is denied without host-side counting (fail-closed).
            if let crate::executor::session_ledger::TurnChargeOutcome::CapExceeded(msg) =
                crate::executor::session_ledger::charge_turn_for_wake(&spec.session_id)
            {
                tracing::info!(
                 session_id = %spec.session_id,
                 %msg,
                 "session turn cap exceeded; denying turn at recv re-arm"
                );
                self.finish(wait_key, Value::String(format!("[turn_denied: {msg}]")));
                return;
            }
            // Bound the loop: count this delivered turn and only re-arm
            // while under the recv node's max_iterations cap. At the cap we skip
            // the re-arm so the recv completes and the session loop ends, instead
            // of splicing fresh turn+recv nodes forever.
            let turn = self.state.next_rearm_turn(&spec.session_id);
            if turn < spec.max_turns {
                if let Err(error) = self.state.rearm_session_turn(
                    message_token,
                    &spec.recv_node,
                    &spec.turn_agent,
                    &spec.turn_flow,
                    &spec.turn_param,
                ) {
                    tracing::error!(%error, "failed to re-arm session turn loop before recv wake");
                }
            } else {
                tracing::info!(
                 session_id = %spec.session_id,
                 turn,
                 max_turns = spec.max_turns,
                 "session turn cap reached; not re-arming (in-graph loop ends)"
                );
            }
        }
        self.finish(wait_key, value);
    }
}

#[cfg(test)]
fn producer_for_outputs(state: &SchedulerState, outputs: &[TokenId]) -> NodeId {
    state
        .nodes
        .iter()
        .find(|entry| {
            outputs
                .iter()
                .any(|token| entry.output_tokens.contains(token))
        })
        .map(|entry| *entry.key())
        .expect("parked output must have a producer node")
}

enum Entry {
    /// Nodes parked on this key, awaiting a wake.
    Waiters(Vec<ParkWaker>),
    /// A wake arrived before any node registered; the value waits for one.
    Resolved(Value),
}

#[derive(Default)]
struct Registry {
    entries: HashMap<String, Entry>,
    closed: HashSet<String>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default()))
}

/// Canonical park `wait_key` for a conversation session's turn-input recv node.
///
/// The in-graph conversation loop's `recv` node parks under this key; the
/// server's `POST /v1/conversations/{session_id}/message` endpoint wakes it with
/// the user message. Both sides MUST derive the key the same way, so it lives
/// here next to [`wake`]. Wake-before-register is handled by the registry.
pub fn session_recv_key(session_id: &str) -> String {
    format!("session_recv:{session_id}")
}

/// Register a parked node's waker under `wait_key`. If a wake already arrived
/// (resolved-before-register race), fire immediately.
pub(crate) fn register(wait_key: String, waker: ParkWaker) {
    // Each match arm consumes `waker` exactly once; the lock is dropped before
    // firing so a wake never runs under the registry mutex.
    let mut guard = registry().lock().expect("park registry poisoned");
    if waker.state.is_cancelled() {
        let key_is_idle =
            !matches!(guard.entries.get(&wait_key), Some(Entry::Waiters(ws)) if !ws.is_empty());
        if key_is_idle {
            guard.entries.remove(&wait_key);
            guard.closed.insert(wait_key.clone());
        }
        drop(guard);
        if key_is_idle {
            durable::clear(&wait_key);
        }
        waker.abandon();
        return;
    }

    guard.closed.remove(&wait_key);
    match guard.entries.remove(&wait_key) {
        Some(Entry::Resolved(value)) => {
            drop(guard);
            // Delivered: no need to keep the durable resolved-marker around —
            // a restart with nothing left to redeliver has nothing to lose.
            durable::clear(&wait_key);
            waker.fire(&wait_key, value);
        }
        Some(Entry::Waiters(mut ws)) => {
            ws.push(waker);
            guard.entries.insert(wait_key.clone(), Entry::Waiters(ws));
            drop(guard);
            durable::record_pending(&wait_key);
        }
        None => {
            guard
                .entries
                .insert(wait_key.clone(), Entry::Waiters(vec![waker]));
            drop(guard);
            durable::record_pending(&wait_key);
        }
    }
}

/// Wake every node parked on `wait_key` with `value`. Returns how many were
/// woken. If none are registered yet (wake-before-register race), the value is
/// stored so the next `register` fires it.
pub fn wake(wait_key: &str, value: Value) -> usize {
    let wakers = {
        let mut guard = registry().lock().expect("park registry poisoned");
        if guard.closed.contains(wait_key) {
            return 0;
        }
        match guard.entries.remove(wait_key) {
            Some(Entry::Waiters(ws)) => ws,
            // No waiters (or a prior resolution): stash the value for a late
            // register — durably too, so a value that arrives while nobody is
            // parked (or while the process is mid-restart) is not lost: a
            // real process restart wipes this in-memory map, but the durable
            // journal survives and `rebuild_from_durable` reloads it.
            _ => {
                guard
                    .entries
                    .insert(wait_key.to_string(), Entry::Resolved(value.clone()));
                drop(guard);
                durable::record_resolved(wait_key, &value);
                return 0;
            }
        }
    }; // lock dropped before firing
    durable::clear(wait_key);
    let n = wakers.len();
    for w in wakers {
        w.fire(wait_key, value.clone());
    }
    n
}

/// Remove every live waiter owned by `state`. Keys with no remaining owners are
/// closed so late wakes return zero instead of becoming wake-before-register
/// values for an execution that has already terminated.
pub(crate) fn remove_for_state(state: &SchedulerState) -> usize {
    let mut guard = registry().lock().expect("park registry poisoned");
    let keys: Vec<String> = guard.entries.keys().cloned().collect();
    let mut removed = 0;
    let mut cleared = Vec::new();

    for wait_key in keys {
        let mut empty = false;
        if let Some(Entry::Waiters(waiters)) = guard.entries.get_mut(&wait_key) {
            let before = waiters.len();
            waiters.retain(|waker| !waker.belongs_to(state));
            removed += before - waiters.len();
            empty = waiters.is_empty();
        }
        if empty {
            guard.entries.remove(&wait_key);
            guard.closed.insert(wait_key.clone());
            cleared.push(wait_key);
        }
    }
    drop(guard);

    for wait_key in cleared {
        durable::clear(&wait_key);
    }
    removed
}

fn close(wait_key: &str) {
    let mut guard = registry().lock().expect("park registry poisoned");
    if matches!(guard.entries.get(wait_key), Some(Entry::Waiters(ws)) if !ws.is_empty()) {
        return;
    }
    guard.entries.remove(wait_key);
    guard.closed.insert(wait_key.to_string());
    drop(guard);
    durable::clear(wait_key);
}

/// [`apxm_capability_iface::CapabilityHost`] implementation over this
/// process-global park registry.
///
/// Capability's builtin `schedule` tool calls through
/// [`CapabilityHost::wake`] instead of naming `park_registry::wake`
/// directly — the narrow seam that lets capability depend on
/// `apxm-capability-iface` instead of on `apxm-runtime`'s scheduler module
/// concretely. Zero-sized: the registry itself is the process-global
/// [`registry()`] `OnceLock`, not per-instance state.
#[derive(Debug, Default, Clone, Copy)]
pub struct ParkRegistryHost;

impl apxm_capability_iface::CapabilityHost for ParkRegistryHost {
    fn wake(&self, wait_key: &str, value: Value) -> usize {
        wake(wait_key, value)
    }
}

/// Durable park-checkpoint journal: there is otherwise no on-disk
/// representation of "node X is parked on wait_key Y" at all, so a `kill -9`
/// loses that bookkeeping even though the wait_key itself (a pure function of
/// session/execution id, see [`session_recv_key`]) is a perfectly stable
/// resumption handle. This journal records two things durably:
///
/// - **pending**: a `register` happened (someone is parked here) — read at
///   boot via [`pending_wait_keys`] so scheduler/session restore knows which
///   logical waits need a freshly-registered waker after the DAG is
///   rehydrated (see `the restart-state reconstruction invariant`); the actual [`ParkWaker`]
///   can never be durable (it closes over a live `Arc<SchedulerState>`).
/// - **resolved**: a `wake` arrived with nobody registered yet — durably, not
///   just in the in-memory `Resolved` stash, so a wake that lands in the
///   narrow window around a restart is not lost. [`rebuild_from_durable`]
///   reloads these into the in-memory stash so the next `register` for that
///   wait_key fires immediately, exactly as it would have pre-restart.
pub mod durable {
    use std::sync::{Mutex, OnceLock};

    #[cfg(feature = "sqlite")]
    use apxm_core::types::Value;
    #[cfg(feature = "sqlite")]
    use rusqlite::{Connection, params};

    #[cfg(feature = "sqlite")]
    fn slot() -> &'static Mutex<Option<Connection>> {
        static SLOT: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(None))
    }

    /// Open (creating if needed) the durable park journal at `path`.
    /// Idempotent — safe to call again after a simulated restart to reopen
    /// the same on-disk file.
    #[cfg(feature = "sqlite")]
    pub fn init(path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS park_journal (
                wait_key TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                value_json TEXT
            );",
        )
        .map_err(|e| e.to_string())?;
        *slot().lock().expect("park journal slot poisoned") = Some(conn);
        Ok(())
    }

    #[cfg(not(feature = "sqlite"))]
    pub fn init(_path: &std::path::Path) -> Result<(), String> {
        Err("durable park-registry persistence requires the 'sqlite' feature".to_string())
    }

    /// Drop the durable connection (test-only: simulates the process dying —
    /// the on-disk file survives, only the in-process handle is dropped).
    #[cfg(feature = "sqlite")]
    pub fn close_for_test() {
        *slot().lock().expect("park journal slot poisoned") = None;
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn record_pending(wait_key: &str) {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else { return };
        let _ = conn.execute(
            "INSERT INTO park_journal (wait_key, state, value_json) VALUES (?1, 'pending', NULL)
             ON CONFLICT(wait_key) DO UPDATE SET state = 'pending', value_json = NULL",
            params![wait_key],
        );
    }
    #[cfg(not(feature = "sqlite"))]
    pub(super) fn record_pending(_wait_key: &str) {}

    #[cfg(feature = "sqlite")]
    pub(super) fn record_resolved(wait_key: &str, value: &Value) {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else { return };
        let Ok(json) = serde_json::to_string(value) else {
            return;
        };
        let _ = conn.execute(
            "INSERT INTO park_journal (wait_key, state, value_json) VALUES (?1, 'resolved', ?2)
             ON CONFLICT(wait_key) DO UPDATE SET state = 'resolved', value_json = excluded.value_json",
            params![wait_key, json],
        );
    }
    #[cfg(not(feature = "sqlite"))]
    pub(super) fn record_resolved(_wait_key: &str, _value: &apxm_core::types::Value) {}

    #[cfg(feature = "sqlite")]
    pub(super) fn clear(wait_key: &str) {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else { return };
        let _ = conn.execute(
            "DELETE FROM park_journal WHERE wait_key = ?1",
            params![wait_key],
        );
    }
    #[cfg(not(feature = "sqlite"))]
    pub(super) fn clear(_wait_key: &str) {}

    /// `(wait_key, state, resolved_value)` rows currently in the journal.
    /// `state` is `"pending"` or `"resolved"`; `resolved_value` is `Some` only
    /// for `"resolved"` rows whose value parsed.
    #[cfg(feature = "sqlite")]
    pub(super) fn load_all() -> Vec<(String, String, Option<Value>)> {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return Vec::new();
        };
        let Ok(mut stmt) = conn.prepare("SELECT wait_key, state, value_json FROM park_journal")
        else {
            return Vec::new();
        };
        let rows = stmt.query_map([], |row| {
            let wait_key: String = row.get(0)?;
            let state: String = row.get(1)?;
            let value_json: Option<String> = row.get(2)?;
            Ok((wait_key, state, value_json))
        });
        let Ok(rows) = rows else { return Vec::new() };
        rows.flatten()
            .map(|(wait_key, state, value_json)| {
                let value = value_json.and_then(|json| serde_json::from_str(&json).ok());
                (wait_key, state, value)
            })
            .collect()
    }
    #[cfg(not(feature = "sqlite"))]
    pub(super) fn load_all() -> Vec<(String, String, Option<apxm_core::types::Value>)> {
        Vec::new()
    }
}

/// Rebuild the in-process registry entries for `wait_keys` from the durable
/// journal, simulating what boot does after a restart for the specific
/// sessions/executions the caller is resuming (typically the set of
/// reconciled-execution boot pass hands back). Only those wait_keys' entries
/// are touched — any in-memory entry for exactly those keys is replaced by
/// what durably survived; every other wait_key (unrelated in-flight work in
/// the same process — at a real boot there is none yet, since the registry
/// starts empty) is left untouched. `"resolved"` rows become an in-memory
/// [`Entry::Resolved`] stash (so the next `register` for that wait_key fires
/// immediately, exactly as it would have pre-restart); `"pending"` rows have
/// no waker to attach (that requires live scheduler state, which restart must
/// rebuild first — see `SchedulerState::restore`) and are dropped from the
/// in-memory map, but remain visible via [`pending_wait_keys`] so a caller
/// knows which logical waits still need a fresh `register` after rehydration.
pub fn rebuild_from_durable(wait_keys: &[String]) {
    let rows: HashMap<String, (String, Option<Value>)> = durable::load_all()
        .into_iter()
        .map(|(wait_key, state, value)| (wait_key, (state, value)))
        .collect();
    let mut guard = registry().lock().expect("park registry poisoned");
    for wait_key in wait_keys {
        guard.entries.remove(wait_key);
        guard.closed.remove(wait_key);
        if let Some((state, value)) = rows.get(wait_key)
            && state == "resolved"
            && let Some(value) = value.clone()
        {
            guard
                .entries
                .insert(wait_key.clone(), Entry::Resolved(value));
        }
        // "pending" rows are intentionally not restored as `Entry::Waiters`:
        // there is no live `ParkWaker` to attach durably (see module docs).
    }
}

/// Wait_keys the durable journal has recorded as still `"pending"` (parked,
/// unresolved) as of the last [`durable::init`]/write — the sessions/executions
/// that need a fresh waker registered once their scheduler state is rebuilt.
pub fn pending_wait_keys() -> Vec<String> {
    durable::load_all()
        .into_iter()
        .filter(|(_, state, _)| state == "pending")
        .map(|(wait_key, _, _)| wait_key)
        .collect()
}
