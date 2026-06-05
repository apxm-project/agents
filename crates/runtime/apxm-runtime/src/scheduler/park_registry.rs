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

use apxm_core::types::{TokenId, Value};

use crate::scheduler::state::SchedulerState;

/// Resumes one parked node by making its output tokens ready in its scheduler.
pub struct ParkWaker {
    state: Arc<SchedulerState>,
    outputs: Vec<TokenId>,
}

impl ParkWaker {
    pub(crate) fn new(state: Arc<SchedulerState>, outputs: Vec<TokenId>) -> Self {
        Self { state, outputs }
    }

    fn fire(self, value: Value) {
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
