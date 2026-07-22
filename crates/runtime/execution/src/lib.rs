//! The canonical Agent Program runtime driver.
//!
//! It executes a compiled five-operation AIR node graph through the kernel's
//! injected ports and commits atomically through the one Execution Commit port.
//! It is generic over the effects: it selects no implementation, holds no
//! router or registry, and never falls back or picks a first-available target.
//!
//! Beyond the single-shot [`execute`], it provides durable park/resume
//! ([`execute_resumable`]/[`resume`]) for conversational sessions that suspend
//! at an `await.event` and continue on delivered input, plus the plane-correct
//! session-family port surface (the [`resume::ContinuationPort`], the pure
//! [`session_ledger::SessionLedger`], and the [`ports::ScopedMemoryPort`]) whose
//! durable implementations are owned by the lifecycle/persistence plane.

pub mod driver;
pub mod ports;
pub mod resume;
pub mod session_ledger;
pub mod structural;

pub use driver::{
    ExecutionError, ExecutionPorts, ExecutionRequest, NodeOutcome, NoopStaticHookHandler,
    RunReport, StaticHookHandlerPort, StaticHookResult, execute, execute_resumable, resume,
    resume_event,
};
pub use ports::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionReceiver, CompositionRequest, EventAwait, EventOutcome, EventPort, EventRef,
    EventRefError, MemoryError, MemorySpace, ScopedMemoryPort,
};
pub use resume::{Continuation, ContinuationError, DurableLoopFrame, RunOutcome};
pub use session_ledger::{LedgerState, SessionLedger, SessionLedgerError, SessionLedgerStore};
