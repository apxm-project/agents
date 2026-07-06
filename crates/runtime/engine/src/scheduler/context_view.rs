//! Narrow, read-only view of [`ExecutionContext`] for scheduler worker-loop
//! bookkeeping (event emission, episodic recording, session re-arm lookups).
//!
//! `ExecutionContext` (`crate::executor::ExecutionContext`) is an
//! executor-owned god-struct that transitively pulls in `CapabilitySystem`,
//! `ModelRouter`, `AgentPool`, `ContextStack`, `ProcessTable`,
//! `SandboxRegistry`, `SessionLedger`, and ~20 other executor-internal types.
//! The scheduler's worker loop (`crate::scheduler::worker`) only ever reads
//! four of those fields directly for its own logic: `execution_id`,
//! `session_id`, `event_emitter`, and the memory subsystem (via
//! `ExecutionContext::memory()`).
//!
//! [`SchedulerCtx`] captures exactly that slice. Purely scheduler-side
//! bookkeeping functions (episodic event recording, scheduler-decision /
//! head-of-line-block event emission, session-loop re-arm detection) take
//! `&dyn SchedulerCtx` instead of `&ExecutionContext`, so they no longer name
//! the full executor type.
//!
//! This does **not** eliminate `ExecutionContext` from the scheduler module
//! entirely — see `worker::worker_loop` / `dataflow::DataflowScheduler`,
//! which hold and mutate the concrete struct because they:
//! - call `ExecutionContext::child()` / `Clone::clone()` (whole-value
//!   semantics inherent to the type),
//! - write the `dag_splicer` field (the scheduler installs
//!   `splicing::SchedulerDagSplicer` before handing the context to workers),
//! - read `aam` / `cancellation_token` / `metadata` to seed scheduler state,
//!   and
//! - must ultimately hand the *full* context to
//!   `ExecutorEngine::execute_with_context`, which dispatches operations that
//!   need the entire struct (capability system, AAM, agent pool, ...).
//!
//! Narrowing those call sites would require a trait-ified dispatch boundary
//! between the scheduler and executor — that's the actual  crate-split
//! work, out of scope here.

use std::sync::Arc;

use crate::ExecutionEventEmitter;
use crate::executor::ExecutionContext;
use crate::memory::MemorySystem;

/// Read-only slice of [`ExecutionContext`] that the scheduler's own
/// bookkeeping (not the executor dispatch hand-off) actually touches.
pub trait SchedulerCtx: Send + Sync {
    /// The execution this context belongs to, for episodic-event tagging.
    fn execution_id(&self) -> &str;

    /// The session id, if this execution is session-scoped (used to detect
    /// session conversation-loop `recv` nodes that need re-arming on wake).
    fn session_id(&self) -> Option<&str>;

    /// The event emitter, if observability is wired up for this execution.
    fn event_emitter(&self) -> Option<&Arc<dyn ExecutionEventEmitter>>;

    /// The memory subsystem, for recording per-node episodic events.
    fn memory(&self) -> &MemorySystem;
}

impl SchedulerCtx for ExecutionContext {
    fn execution_id(&self) -> &str {
        &self.execution_id
    }

    fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    fn event_emitter(&self) -> Option<&Arc<dyn ExecutionEventEmitter>> {
        self.event_emitter.as_ref()
    }

    fn memory(&self) -> &MemorySystem {
        ExecutionContext::memory(self)
    }
}
