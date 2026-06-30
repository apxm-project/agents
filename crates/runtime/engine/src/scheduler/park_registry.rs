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

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use apxm_core::types::{Node, TokenId, Value};

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
    /// the cap the loop stops re-arming, so a session is bounded (CONV-1).
    pub(crate) session_id: String,
    pub(crate) max_turns: u64,
}

/// Resumes one parked node by making its output tokens ready in its scheduler.
pub struct ParkWaker {
    state: Arc<SchedulerState>,
    outputs: Vec<TokenId>,
    /// When set, the parked node is a re-arming session recv: after delivering
    /// the message, splice a fresh turn + recv to continue the loop.
    rearm: Option<RearmSpec>,
}

impl ParkWaker {
    pub(crate) fn new(state: Arc<SchedulerState>, outputs: Vec<TokenId>) -> Self {
        Self {
            state,
            outputs,
            rearm: None,
        }
    }

    /// A re-arming waker for a session conversation-loop recv node.
    pub(crate) fn new_rearming(
        state: Arc<SchedulerState>,
        outputs: Vec<TokenId>,
        rearm: RearmSpec,
    ) -> Self {
        Self {
            state,
            outputs,
            rearm: Some(rearm),
        }
    }

    fn fire(self, value: Value) {
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
                self.state.wake_parked_node(
                    &self.outputs,
                    Value::String(format!("[turn_denied: {msg}]")),
                );
                return;
            }
            // Bound the loop (CONV-1): count this delivered turn and only re-arm
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
        self.state.wake_parked_node(&self.outputs, value);
    }
}

enum Entry {
    /// Nodes parked on this key, awaiting a wake.
    Waiters(Vec<ParkWaker>),
    /// A wake arrived before any node registered; the value waits for one.
    Resolved(Value),
}

fn registry() -> &'static Mutex<HashMap<String, Entry>> {
    static R: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
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
    match guard.remove(&wait_key) {
        Some(Entry::Resolved(value)) => {
            drop(guard);
            waker.fire(value);
        }
        Some(Entry::Waiters(mut ws)) => {
            ws.push(waker);
            guard.insert(wait_key, Entry::Waiters(ws));
        }
        None => {
            guard.insert(wait_key, Entry::Waiters(vec![waker]));
        }
    }
}

/// Wake every node parked on `wait_key` with `value`. Returns how many were
/// woken. If none are registered yet (wake-before-register race), the value is
/// stored so the next `register` fires it.
pub fn wake(wait_key: &str, value: Value) -> usize {
    let wakers = {
        let mut guard = registry().lock().expect("park registry poisoned");
        match guard.remove(wait_key) {
            Some(Entry::Waiters(ws)) => ws,
            // No waiters (or a prior resolution): stash the value for a late register.
            _ => {
                guard.insert(wait_key.to_string(), Entry::Resolved(value));
                return 0;
            }
        }
    }; // lock dropped before firing
    let n = wakers.len();
    for w in wakers {
        w.fire(value.clone());
    }
    n
}
