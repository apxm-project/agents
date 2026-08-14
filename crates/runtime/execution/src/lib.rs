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
//! session-family port surface (the [`resume::Continuation`] read back through
//! the one Execution Commit port, the pure [`session_ledger::SessionLedger`],
//! and the [`ports::ScopedMemoryPort`]) whose durable implementations are owned
//! by the lifecycle/persistence plane.
//!
//! Dependency readiness for one leased activation lives in [`readiness`]. It is
//! the single decision point that makes a node occurrence runnable; nothing else
//! in this crate may synthesize that transition.

mod bundle;
pub mod driver;
pub mod operational_usage;
pub mod ports;
pub mod profile;
pub mod readiness;
pub mod resume;
pub mod session_ledger;
pub mod structural;

pub use bundle::ExecutionPortBundle;
pub use driver::{
    CapabilityGrantOrigin, CapabilityGrantSet, CapabilityInvocationAdmission, CapabilityNotGranted,
    ExecutionError, ExecutionPorts, ExecutionPortsError, ExecutionRequest, NodeOutcome,
    NoopStaticHookHandler, RunReport, StaticHookExecutionError, StaticHookHandlerPort,
    StaticHookResult, execute, execute_resumable, resume, resume_event,
};
pub use operational_usage::{
    CommittedNativeModelUsage, CommittedNativeModelUsageError, CommittedNativeModelUsageGateError,
    CommittedNativeModelUsageOutcome, CommittedNativeModelUsagePort,
    CommittedNativeModelUsageVersion, EvidencePositionRef, EvidencePositionRefType,
};
pub use ports::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionReceiver, CompositionRequest, EventAwait, EventOutcome, EventPort, EventRef,
    EventRefError, MemoryError, MemorySpace, ScopedMemoryPort,
};
pub use profile::{RuntimeProfile, RuntimeProfileError};
pub use readiness::{
    NodeLifecycle, NodeOccurrenceId, OperandSlotDecl, OperandValue, ReadinessError,
    ReadinessKernel, ReadinessKernelBuilder, ReadinessTransition, RunnableNode, SlotClaim,
};
pub use resume::{Continuation, ContinuationError, DurableLoopFrame, RunOutcome};
pub use session_ledger::{LedgerState, SessionLedger, SessionLedgerError, SessionLedgerStore};
