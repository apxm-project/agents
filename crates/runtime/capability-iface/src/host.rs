//! The narrow wake-notification interface capability's builtin `schedule`
//! tool needs from the scheduler.
//!
//! Deliberately **not** the whole `park_registry` module (register/state/
//! lost-wakeup bookkeeping stay scheduler-only) — just the one operation a
//! capability ever needs to perform on the scheduler: deliver a value to
//! whatever is parked on a correlation key.

use apxm_core::types::values::Value;

/// A host capable of waking scheduler-parked work waiting on a correlation
/// key.
///
/// `apxm-runtime`'s scheduler implements this over its process-global
/// park/wake registry; capability's `schedule` builtin tool calls through
/// this trait instead of naming `scheduler::park_registry::wake` directly.
pub trait CapabilityHost: Send + Sync {
    /// Wake every waiter parked on `wait_key`, delivering `value`.
    ///
    /// Returns how many waiters were woken. Implementations may choose to
    /// stash the value for a not-yet-registered waiter (wake-before-register
    /// race) and return `0` in that case, as the scheduler's park registry
    /// does.
    fn wake(&self, wait_key: &str, value: Value) -> usize;
}
