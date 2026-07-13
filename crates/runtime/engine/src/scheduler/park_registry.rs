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
//! FIFO queue of resolved values that subsequent registers consume
//! one-by-one in arrival order.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use apxm_core::types::{Node, NodeId, TokenId, Value};
use thiserror::Error;

use crate::scheduler::state::SchedulerState;

/// Typed durable park-registry failures surfaced to runtime and Server callers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParkRegistryError {
    /// The durable journal could not be initialized.
    #[error("failed to initialize park journal at {path}: {message}")]
    DurableInitialization { path: String, message: String },
    /// A durable park row could not be read.
    #[error("failed to read durable park state for '{wait_key}': {message}")]
    DurableRead { wait_key: String, message: String },
    /// A durable park row could not be written or deleted.
    #[error("failed to write durable park state for '{wait_key}': {message}")]
    DurableWrite { wait_key: String, message: String },
    /// A durable park row is not valid for the current typed journal schema.
    #[error("invalid durable park state for '{wait_key}': {message}")]
    CorruptRecord { wait_key: String, message: String },
}

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
            if let Err(error) =
                crate::executor::session_ledger::charge_turn_for_wake(&spec.session_id)
            {
                tracing::info!(
                 session_id = %spec.session_id,
                 %error,
                 "session turn charge denied at recv re-arm"
                );
                self.finish(wait_key, Value::String(format!("[turn_denied: {error}]")));
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
    /// One or more wakes arrived before any node registered; each subsequent
    /// register consumes exactly one value in FIFO order.
    Resolved(VecDeque<Value>),
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

#[cfg(test)]
pub(crate) fn clear_in_memory_for_test(wait_key: &str) {
    let mut guard = registry().lock().expect("park registry poisoned");
    guard.entries.remove(wait_key);
    guard.closed.remove(wait_key);
}

#[cfg(test)]
pub(crate) fn contains_in_memory_for_test(wait_key: &str) -> bool {
    registry()
        .lock()
        .expect("park registry poisoned")
        .entries
        .contains_key(wait_key)
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
pub(crate) fn register(wait_key: String, waker: ParkWaker) -> Result<(), ParkRegistryError> {
    let mut guard = registry().lock().expect("park registry poisoned");
    if waker.state.is_cancelled() {
        let key_is_idle =
            !matches!(guard.entries.get(&wait_key), Some(Entry::Waiters(ws)) if !ws.is_empty());
        if key_is_idle {
            if let Err(error) = durable::clear(&wait_key) {
                drop(guard);
                waker.abandon();
                return Err(error);
            }
            guard.entries.remove(&wait_key);
            guard.closed.insert(wait_key.clone());
        }
        drop(guard);
        waker.abandon();
        return Ok(());
    }

    match guard.entries.get(&wait_key) {
        Some(Entry::Resolved(values)) => {
            let mut remaining = values.clone();
            let value = remaining
                .pop_front()
                .expect("resolved queue must contain at least one value");
            if remaining.is_empty() {
                if let Err(error) = durable::clear(&wait_key) {
                    drop(guard);
                    waker.abandon();
                    return Err(error);
                }
            } else {
                if let Err(error) = durable::record_resolved(&wait_key, &remaining) {
                    drop(guard);
                    waker.abandon();
                    return Err(error);
                }
            }
            guard.closed.remove(&wait_key);
            if remaining.is_empty() {
                guard.entries.remove(&wait_key);
            } else {
                guard
                    .entries
                    .insert(wait_key.clone(), Entry::Resolved(remaining));
            }
            drop(guard);
            waker.fire(&wait_key, value);
            Ok(())
        }
        Some(Entry::Waiters(_)) => {
            if let Err(error) = durable::record_pending(&wait_key) {
                drop(guard);
                waker.abandon();
                return Err(error);
            }
            guard.closed.remove(&wait_key);
            match guard.entries.get_mut(&wait_key) {
                Some(Entry::Waiters(waiters)) => waiters.push(waker),
                _ => unreachable!("park registry entry changed while locked"),
            }
            Ok(())
        }
        None => {
            if let Err(error) = durable::record_pending(&wait_key) {
                drop(guard);
                waker.abandon();
                return Err(error);
            }
            guard.closed.remove(&wait_key);
            guard.entries.insert(wait_key, Entry::Waiters(vec![waker]));
            Ok(())
        }
    }
}

/// Wake every node parked on `wait_key` with `value`. Returns how many were
/// woken. If none are registered yet (wake-before-register race), the value is
/// stored so the next `register` fires it.
pub fn wake(wait_key: &str, value: Value) -> Result<usize, ParkRegistryError> {
    let wakers = {
        let mut guard = registry().lock().expect("park registry poisoned");
        if guard.closed.contains(wait_key) {
            return Ok(0);
        }
        match guard.entries.get(wait_key) {
            Some(Entry::Waiters(_)) => {
                durable::clear(wait_key)?;
                match guard.entries.remove(wait_key) {
                    Some(Entry::Waiters(waiters)) => waiters,
                    _ => unreachable!("park registry entry changed while locked"),
                }
            }
            // No waiters (or a prior resolution): stash the value for a late
            // register — durably too, so values that arrive while nobody is
            // parked (or while the process is mid-restart) are not lost: a
            // real process restart wipes this in-memory map, but the durable
            // journal survives and `rebuild_from_durable` reloads the queue.
            Some(Entry::Resolved(values)) => {
                let mut next = values.clone();
                next.push_back(value.clone());
                durable::record_resolved(wait_key, &next)?;
                guard
                    .entries
                    .insert(wait_key.to_string(), Entry::Resolved(next));
                return Ok(0);
            }
            None => {
                let mut values = VecDeque::new();
                values.push_back(value.clone());
                durable::record_resolved(wait_key, &values)?;
                guard
                    .entries
                    .insert(wait_key.to_string(), Entry::Resolved(values));
                return Ok(0);
            }
        }
    }; // lock dropped before firing
    let n = wakers.len();
    for w in wakers {
        w.fire(wait_key, value.clone());
    }
    Ok(n)
}

/// Remove every live waiter owned by `state`. Keys with no remaining owners are
/// closed so late wakes return zero instead of becoming wake-before-register
/// values for an execution that has already terminated.
pub(crate) fn remove_for_state(state: &SchedulerState) -> Result<usize, ParkRegistryError> {
    let mut guard = registry().lock().expect("park registry poisoned");
    let keys: Vec<String> = guard.entries.keys().cloned().collect();
    let mut removed = 0;
    let mut cleared = Vec::new();

    for wait_key in &keys {
        if let Some(Entry::Waiters(waiters)) = guard.entries.get(wait_key) {
            let owned = waiters
                .iter()
                .filter(|waker| waker.belongs_to(state))
                .count();
            removed += owned;
            if owned == waiters.len() && owned > 0 {
                cleared.push(wait_key.clone());
            }
        }
    }
    durable::clear_many(&cleared)?;
    for wait_key in keys {
        if cleared.contains(&wait_key) {
            guard.entries.remove(&wait_key);
            guard.closed.insert(wait_key);
        } else if let Some(Entry::Waiters(waiters)) = guard.entries.get_mut(&wait_key) {
            waiters.retain(|waker| !waker.belongs_to(state));
        }
    }
    Ok(removed)
}

fn close(wait_key: &str) {
    let mut guard = registry().lock().expect("park registry poisoned");
    if matches!(guard.entries.get(wait_key), Some(Entry::Waiters(ws)) if !ws.is_empty()) {
        return;
    }
    guard.entries.remove(wait_key);
    guard.closed.insert(wait_key.to_string());
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
    fn wake(
        &self,
        wait_key: &str,
        value: Value,
    ) -> Result<usize, apxm_capability_iface::CapabilityHostError> {
        wake(wait_key, value).map_err(|error| apxm_capability_iface::CapabilityHostError::Wake {
            wait_key: wait_key.to_string(),
            message: error.to_string(),
        })
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
/// - **resolved**: one or more `wake`s arrived with nobody registered yet —
///   durably, not just in the in-memory `Resolved` stash, so queued values
///   that land in the narrow window around a restart are not lost.
///   [`rebuild_from_durable`] reloads these into the in-memory stash so
///   subsequent `register`s for that wait_key fire immediately, exactly as
///   they would have pre-restart.
pub mod durable {
    #[cfg(feature = "sqlite")]
    use std::collections::VecDeque;
    use std::sync::{Mutex, OnceLock};

    #[cfg(feature = "sqlite")]
    use apxm_core::types::Value;
    #[cfg(feature = "sqlite")]
    use rusqlite::{Connection, OptionalExtension, params};

    use super::ParkRegistryError;

    #[derive(Debug)]
    pub(super) enum DurableEntry {
        Pending,
        Resolved(std::collections::VecDeque<apxm_core::types::Value>),
    }

    #[cfg(feature = "sqlite")]
    fn slot() -> &'static Mutex<Option<Connection>> {
        static SLOT: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(None))
    }

    #[cfg(test)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub(crate) enum FaultOperation {
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
    pub(crate) fn fail_next_for_test(operation: FaultOperation, wait_key: &str) {
        faults()
            .lock()
            .expect("park journal fault injector poisoned")
            .insert((operation, wait_key.to_string()));
    }

    #[cfg(test)]
    fn inject_failure(operation: FaultOperation, wait_key: &str) -> bool {
        faults()
            .lock()
            .expect("park journal fault injector poisoned")
            .remove(&(operation, wait_key.to_string()))
    }

    /// Open (creating if needed) the durable park journal at `path`.
    /// Idempotent — safe to call again after a simulated restart to reopen
    /// the same on-disk file.
    #[cfg(feature = "sqlite")]
    pub fn init(path: &std::path::Path) -> Result<(), ParkRegistryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                ParkRegistryError::DurableInitialization {
                    path: path.display().to_string(),
                    message: error.to_string(),
                }
            })?;
        }
        let conn =
            Connection::open(path).map_err(|error| ParkRegistryError::DurableInitialization {
                path: path.display().to_string(),
                message: error.to_string(),
            })?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| ParkRegistryError::DurableInitialization {
                path: path.display().to_string(),
                message: error.to_string(),
            })?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS park_journal (
                wait_key TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                value_json TEXT
            );",
        )
        .map_err(|error| ParkRegistryError::DurableInitialization {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        *slot().lock().expect("park journal slot poisoned") = Some(conn);
        Ok(())
    }

    #[cfg(not(feature = "sqlite"))]
    pub fn init(path: &std::path::Path) -> Result<(), ParkRegistryError> {
        Err(ParkRegistryError::DurableInitialization {
            path: path.display().to_string(),
            message: "durable park-registry persistence requires the 'sqlite' feature".to_string(),
        })
    }

    /// Drop the durable connection (test-only: simulates the process dying —
    /// the on-disk file survives, only the in-process handle is dropped).
    #[cfg(feature = "sqlite")]
    pub fn close_for_test() {
        *slot().lock().expect("park journal slot poisoned") = None;
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn record_pending(wait_key: &str) -> Result<(), ParkRegistryError> {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return Ok(());
        };
        #[cfg(test)]
        if inject_failure(FaultOperation::Write, wait_key) {
            return Err(ParkRegistryError::DurableWrite {
                wait_key: wait_key.to_string(),
                message: "injected write failure".to_string(),
            });
        }
        conn.execute(
            "INSERT INTO park_journal (wait_key, state, value_json) VALUES (?1, 'pending', NULL)
             ON CONFLICT(wait_key) DO UPDATE SET state = 'pending', value_json = NULL",
            params![wait_key],
        )
        .map_err(|error| ParkRegistryError::DurableWrite {
            wait_key: wait_key.to_string(),
            message: error.to_string(),
        })?;
        Ok(())
    }
    #[cfg(not(feature = "sqlite"))]
    pub(super) fn record_pending(_wait_key: &str) -> Result<(), ParkRegistryError> {
        Ok(())
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn record_resolved(
        wait_key: &str,
        values: &VecDeque<Value>,
    ) -> Result<(), ParkRegistryError> {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return Ok(());
        };
        #[cfg(test)]
        if inject_failure(FaultOperation::Write, wait_key) {
            return Err(ParkRegistryError::DurableWrite {
                wait_key: wait_key.to_string(),
                message: "injected write failure".to_string(),
            });
        }
        let json =
            serde_json::to_string(values).map_err(|error| ParkRegistryError::DurableWrite {
                wait_key: wait_key.to_string(),
                message: error.to_string(),
            })?;
        conn.execute(
            "INSERT INTO park_journal (wait_key, state, value_json) VALUES (?1, 'resolved', ?2)
             ON CONFLICT(wait_key) DO UPDATE SET state = 'resolved', value_json = excluded.value_json",
            params![wait_key, json],
        )
        .map_err(|error| ParkRegistryError::DurableWrite {
            wait_key: wait_key.to_string(),
            message: error.to_string(),
        })?;
        Ok(())
    }
    #[cfg(not(feature = "sqlite"))]
    pub(super) fn record_resolved(
        _wait_key: &str,
        _values: &std::collections::VecDeque<apxm_core::types::Value>,
    ) -> Result<(), ParkRegistryError> {
        Ok(())
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn clear(wait_key: &str) -> Result<(), ParkRegistryError> {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return Ok(());
        };
        #[cfg(test)]
        if inject_failure(FaultOperation::Write, wait_key) {
            return Err(ParkRegistryError::DurableWrite {
                wait_key: wait_key.to_string(),
                message: "injected write failure".to_string(),
            });
        }
        conn.execute(
            "DELETE FROM park_journal WHERE wait_key = ?1",
            params![wait_key],
        )
        .map_err(|error| ParkRegistryError::DurableWrite {
            wait_key: wait_key.to_string(),
            message: error.to_string(),
        })?;
        Ok(())
    }
    #[cfg(not(feature = "sqlite"))]
    pub(super) fn clear(_wait_key: &str) -> Result<(), ParkRegistryError> {
        Ok(())
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn clear_many(wait_keys: &[String]) -> Result<(), ParkRegistryError> {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return Ok(());
        };
        if wait_keys.is_empty() {
            return Ok(());
        }
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| ParkRegistryError::DurableWrite {
                wait_key: wait_keys.join(","),
                message: error.to_string(),
            })?;
        for wait_key in wait_keys {
            #[cfg(test)]
            if inject_failure(FaultOperation::Write, wait_key) {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(ParkRegistryError::DurableWrite {
                    wait_key: wait_key.clone(),
                    message: "injected write failure".to_string(),
                });
            }
            if let Err(error) = conn.execute(
                "DELETE FROM park_journal WHERE wait_key = ?1",
                params![wait_key],
            ) {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(ParkRegistryError::DurableWrite {
                    wait_key: wait_key.clone(),
                    message: error.to_string(),
                });
            }
        }
        if let Err(error) = conn.execute_batch("COMMIT") {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(ParkRegistryError::DurableWrite {
                wait_key: wait_keys.join(","),
                message: error.to_string(),
            });
        }
        Ok(())
    }
    #[cfg(not(feature = "sqlite"))]
    pub(super) fn clear_many(_wait_keys: &[String]) -> Result<(), ParkRegistryError> {
        Ok(())
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn load(wait_key: &str) -> Result<Option<DurableEntry>, ParkRegistryError> {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return Ok(None);
        };
        #[cfg(test)]
        if inject_failure(FaultOperation::Read, wait_key) {
            return Err(ParkRegistryError::DurableRead {
                wait_key: wait_key.to_string(),
                message: "injected read failure".to_string(),
            });
        }
        let row = conn
            .query_row(
                "SELECT state, value_json FROM park_journal WHERE wait_key = ?1",
                params![wait_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(|error| ParkRegistryError::DurableRead {
                wait_key: wait_key.to_string(),
                message: error.to_string(),
            })?;
        let Some((state, value_json)) = row else {
            return Ok(None);
        };
        match state.as_str() {
            "pending" if value_json.is_none() => Ok(Some(DurableEntry::Pending)),
            "resolved" => {
                let json = value_json.ok_or_else(|| ParkRegistryError::CorruptRecord {
                    wait_key: wait_key.to_string(),
                    message: "resolved row is missing value_json".to_string(),
                })?;
                let values = serde_json::from_str::<VecDeque<Value>>(&json).map_err(|error| {
                    ParkRegistryError::CorruptRecord {
                        wait_key: wait_key.to_string(),
                        message: error.to_string(),
                    }
                })?;
                if values.is_empty() {
                    return Err(ParkRegistryError::CorruptRecord {
                        wait_key: wait_key.to_string(),
                        message: "resolved row contains an empty FIFO queue".to_string(),
                    });
                }
                Ok(Some(DurableEntry::Resolved(values)))
            }
            _ => Err(ParkRegistryError::CorruptRecord {
                wait_key: wait_key.to_string(),
                message: format!("unsupported state '{state}'"),
            }),
        }
    }

    #[cfg(not(feature = "sqlite"))]
    pub(super) fn load(_wait_key: &str) -> Result<Option<DurableEntry>, ParkRegistryError> {
        Ok(None)
    }

    #[cfg(feature = "sqlite")]
    pub(super) fn pending_wait_keys() -> Result<Vec<String>, ParkRegistryError> {
        let guard = slot().lock().expect("park journal slot poisoned");
        let Some(conn) = guard.as_ref() else {
            return Ok(Vec::new());
        };
        let mut stmt = conn
            .prepare("SELECT wait_key FROM park_journal WHERE state = 'pending' ORDER BY wait_key")
            .map_err(|error| ParkRegistryError::DurableRead {
                wait_key: "*".to_string(),
                message: error.to_string(),
            })?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| ParkRegistryError::DurableRead {
                wait_key: "*".to_string(),
                message: error.to_string(),
            })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| ParkRegistryError::DurableRead {
                wait_key: "*".to_string(),
                message: error.to_string(),
            })
    }

    #[cfg(not(feature = "sqlite"))]
    pub(super) fn pending_wait_keys() -> Result<Vec<String>, ParkRegistryError> {
        Ok(Vec::new())
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
/// [`Entry::Resolved`] queue (so subsequent `register`s for that wait_key fire
/// immediately in FIFO order, exactly as they would have pre-restart);
/// `"pending"` rows have no waker to attach (that requires live scheduler
/// state, which restart must rebuild first — see `SchedulerState::restore`)
/// and are dropped from the in-memory map, but remain visible via
/// [`pending_wait_keys`] so a caller knows which logical waits still need a
/// fresh `register` after rehydration.
pub fn rebuild_from_durable(wait_keys: &[String]) -> Result<(), ParkRegistryError> {
    let mut rows = HashMap::new();
    for wait_key in wait_keys {
        rows.insert(wait_key.clone(), durable::load(wait_key)?);
    }
    let mut guard = registry().lock().expect("park registry poisoned");
    for wait_key in wait_keys {
        guard.entries.remove(wait_key);
        guard.closed.remove(wait_key);
        if let Some(Some(durable::DurableEntry::Resolved(values))) = rows.get(wait_key) {
            guard
                .entries
                .insert(wait_key.clone(), Entry::Resolved(values.clone()));
        }
        // "pending" rows are intentionally not restored as `Entry::Waiters`:
        // there is no live `ParkWaker` to attach durably (see module docs).
    }
    Ok(())
}

/// Wait_keys the durable journal has recorded as still `"pending"` (parked,
/// unresolved) as of the last [`durable::init`]/write — the sessions/executions
/// that need a fresh waker registered once their scheduler state is rebuilt.
pub fn pending_wait_keys() -> Result<Vec<String>, ParkRegistryError> {
    durable::pending_wait_keys()
}
