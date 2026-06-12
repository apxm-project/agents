//! Process-global admission registry — lets a PARKED execution release its
//! cross-execution admission slot (e.g. the server's inference limiter) while it
//! waits on an event, and best-effort reacquire it on wake.
//!
//! The runtime is decoupled from what "admission" means: when an execution
//! becomes parked it calls [`on_park`] with its admission key; when it resumes it
//! calls [`on_unpark`]. The host (apxm-server) registers a [`ParkAdmission`] impl
//! that owns the real permit and knows how to drop/reacquire it. Keyed by an
//! execution-scoped id the host stamps into `metadata` (so the host, which mints
//! the key, can manage the permit it registered).
//!
//! Without a registered handle (tests, non-server runs) every call is a no-op, so
//! the behaviour is unchanged.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Host control over an execution's cross-execution admission slot.
pub trait ParkAdmission: Send + Sync {
    /// The execution parked (is waiting on an event) — release its admission slot
    /// so other work can be admitted in its place.
    fn on_park(&self);
    /// The execution resumed — best-effort reacquire the slot. MUST NOT block; if
    /// the slot is unavailable the execution proceeds un-admitted (a bounded,
    /// transient overshoot is preferred to stalling a resumed agent).
    fn on_unpark(&self);
}

fn registry() -> &'static Mutex<HashMap<String, Arc<dyn ParkAdmission>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<dyn ParkAdmission>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a host admission handle under an execution-scoped `id`.
pub fn register(id: String, handle: Arc<dyn ParkAdmission>) {
    registry()
        .lock()
        .expect("admission registry poisoned")
        .insert(id, handle);
}

/// Remove the handle (call on execution completion; dropping the host's handle
/// finalizes the slot). No-op if absent.
pub fn unregister(id: &str) {
    registry()
        .lock()
        .expect("admission registry poisoned")
        .remove(id);
}

/// Release the admission slot for `id` (the execution parked). No-op if absent.
pub fn on_park(id: &str) {
    // Clone the handle out before invoking it so the registry lock is never held
    // across the host callback (which may take its own lock).
    let handle = registry()
        .lock()
        .expect("admission registry poisoned")
        .get(id)
        .cloned();
    if let Some(h) = handle {
        h.on_park();
    }
}

/// Best-effort reacquire the admission slot for `id` (the execution resumed).
/// No-op if absent.
pub fn on_unpark(id: &str) {
    let handle = registry()
        .lock()
        .expect("admission registry poisoned")
        .get(id)
        .cloned();
    if let Some(h) = handle {
        h.on_unpark();
    }
}

