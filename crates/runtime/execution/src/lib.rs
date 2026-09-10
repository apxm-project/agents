//! The canonical Agent Program runtime driver.
//!
//! It executes a compiled five-operation AIR node graph through the kernel's
//! injected ports and commits atomically through the one Execution Commit port.
//! It is generic over the effects: it selects no implementation, holds no
//! router or registry, and never falls back or picks a first-available target.
//!
//! Beyond the single-shot [`execute`], it provides durable park/resume
//! ([`execute_resumable`]/[`resume`]) for an invocation that suspends at an
//! `await.event` and continues on delivered input. Durable state is read and
//! written through the one Execution Commit port; lifecycle and persistence
//! implementations remain outside the runtime driver.
//!
//! Dependency readiness for one leased activation lives in [`readiness`]. It is
//! the single decision point that makes a node occurrence runnable; nothing else
//! in this crate may synthesize that transition.

mod bundle;
pub mod driver;
pub mod observe;
pub mod operational_usage;
pub mod ports;
pub mod profile;
pub mod readiness;
pub mod resume;
pub mod structural;

pub use bundle::ExecutionPortBundle;
pub use driver::{
    CancellationToken, CapabilityGrantOrigin, CapabilityGrantSet, CapabilityInvocationAdmission,
    CapabilityNotGranted, CapturedHookBodyHandler, EntrypointInput, ExecutionError, ExecutionPorts,
    ExecutionPortsError, ExecutionRequest, MAX_EXPRESSION_DEPTH, MAX_HOOK_BINDINGS,
    MAX_INITIAL_VALUES, MAX_SCHEDULE_STEPS, MAX_SEMANTIC_OPERATIONS, MAX_STRUCTURAL_REGIONS,
    MAX_VALUE_ASSEMBLIES, NodeOutcome, RunReport, RunTerminalStatus, StaticHookExecutionError,
    StaticHookHandlerPort, StaticHookInvocation, StaticHookResult, execute, execute_resumable,
    execute_resumable_with_resource_ceilings, execute_with_resource_ceilings, resume,
    resume_invocation, resume_invocation_with_resource_ceilings, resume_with_resource_ceilings,
    wake_from_event_application,
};
pub use observe::{
    AllowBroker, ApprovalBroker, ApprovalDecision, DenyBroker, ObservationFailurePolicy,
    ObservationRecorder, ObservationSink, ObservationSinkError, TimeoutBroker,
};
pub use operational_usage::{
    CommittedNativeModelUsage, CommittedNativeModelUsageError, CommittedNativeModelUsageGateError,
    CommittedNativeModelUsageOutcome, CommittedNativeModelUsagePort,
    CommittedNativeModelUsageVersion, EvidencePositionRef, EvidencePositionRefType,
};
pub use ports::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionReceiver, CompositionRequest, EventApplication, EventApplicationResult, EventAwait,
    EventOutcome, EventPort, EventRef, EventRefError, MemoryError, MemorySpace, ScopedMemoryPort,
};
pub use profile::{RuntimeProfile, RuntimeProfileError};
pub use readiness::{
    NodeLifecycle, NodeOccurrenceId, OperandSlotDecl, OperandValue, ReadinessError,
    ReadinessKernel, ReadinessKernelBuilder, ReadinessTransition, RunnableNode, SlotClaim,
};
pub use resume::{
    Continuation, ContinuationError, DurableLoopFrame, HookTargetSnapshot, RunOutcome,
};
