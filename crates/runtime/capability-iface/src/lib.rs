//! Mediating interface crate between `apxm-runtime`'s capability module and its
//! scheduler/executor modules.
//!
//! # Why this crate exists
//!
//! `apxm-runtime` currently hosts three modules — `scheduler`, `executor`, and
//! `capability` — that form a genuine three-way
//! `use`-graph cycle: executor dispatches through the scheduler, the scheduler
//! calls back into executor context/hooks, and capability's builtin `schedule`
//! tool calls the scheduler's park registry directly while executor's
//! `ExecutionContext` holds a concrete `Arc<CapabilitySystem>`. Because all
//! three live in one crate today, the cycle isn't a *compile* problem — but it
//! blocks ever splitting them into separate crates, since a naive split
//! would recreate the cycle at the `Cargo.toml` dependency level.
//!
//! This crate is the trait seam that breaks it. It defines:
//!
//! - [`CapabilityHost`] — the narrow wake-notification interface capability's
//!   builtin `schedule` tool needs from the scheduler (not the whole
//!   `park_registry` module).
//! - [`ExecutionEventEmitter`] — the event-observer trait capability's
//!   interceptor pipeline calls into, moved here from executor so both sides
//!   can depend on the trait without depending on each other.
//! - [`sandbox`] — the sandbox isolation types (`IsolationLevel`,
//!   `SandboxRegistry`, `ExecRequest`, `ExecResult`, ...), relocated here
//!   wholesale since they had zero dependencies on other `apxm-runtime`
//!   internals.
//! - [`CapabilityFacade`] — executor's actual usage surface of
//!   `CapabilitySystem`, so `ExecutionContext.capability_system` can hold
//!   `Arc<dyn CapabilityFacade>` instead of the concrete type.
//!
//! This crate carries the shared traits and types; concrete runtime
//! implementations remain in their owning modules.

mod facade;
mod host;
mod metadata;
mod token_usage;

pub mod events;
pub mod sandbox;

pub use facade::{
    ApprovalContext, CapabilityEffectReplayEvidence, CapabilityEffectReplayEvidenceEnvelope,
    CapabilityFacade, CapabilityInvocation, CapabilitySandboxPreflight, HostEffectPrepareEvidence,
    capability_effect_idempotency_key_digest,
};
pub use host::{CapabilityHost, CapabilityHostError};
pub use metadata::{
    CapabilityGrantRuntimeQuota, CapabilityGrantScopeRequirements, CapabilityGrantScopeSelector,
    RuntimeCapability,
};
pub use token_usage::TokenUsageSummary;
