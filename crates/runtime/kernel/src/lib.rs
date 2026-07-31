//! Agent Program runtime kernel.
//!
//! This crate owns the generic Program Instance runtime lifecycle: single-flight
//! fail-busy instances, typed Hook callbacks over the scoped Agent Facade lowered
//! to the private `HookEffect<C, T>` ABI, a typed port bundle constructed from
//! immutable Exact Port Bindings (validated at construction only — no registry,
//! discovery, first-party bypass, or rebind), the atomic Execution Commit port as
//! the only commit boundary, the prepared-effect reducer that decides what a
//! durable effect outcome means, a Confinement port, non-authoritative
//! post-commit telemetry, and lifecycle reconstruction from durable evidence. It
//! selects no implementation and admits nothing; adapters implement the ports it
//! defines.

pub mod bundle;
pub mod capability;
pub mod commit;
pub mod confinement;
pub mod effect;
pub mod events;
pub mod external_agent;
pub mod hook;
pub mod instance;
pub mod reconcile;

pub use bundle::{
    BundleError, ExactPortBinding, PortBundle, PortBundleSpec, PortImplementation, PortSlot,
};
pub use capability::{CapabilityOutcome, CapabilityPort, CapabilityRequest};
pub use commit::{
    ATOMIC_WRITE_SET, AtomicWriteSet, ExecutionCommitPort, ExecutionCommitRequest,
    ExecutionCommitResult, ExecutionCommitTuple, ProgramInstanceRef, ProgramInvocationRef,
};
pub use confinement::{
    ConfinementAttestation, ConfinementError, ConfinementPort, ConfinementRequest, ConfinementType,
};
pub use effect::{
    EffectError, EffectId, EffectRecord, EffectState, EffectTransition, PreparedEffect,
};
pub use events::{EventSink, NullEventSink, TelemetryNote};
pub use external_agent::{
    AcpPromptOutcome, AcpPromptRequest, ExternalAgentCapabilityPort, PromptEffectState,
    assemble_evidence,
};
pub use hook::{AgentFacade, Hook, HookEffect, HookReturn, apply_hooks};
pub use instance::{
    CapabilityInvocation, CapabilityReport, InstanceError, Invocation, InvocationReport,
    ProgramInstance,
};
pub use reconcile::{LifecycleView, reconstruct};
