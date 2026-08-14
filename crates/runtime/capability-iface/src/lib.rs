//! Mediating interface crate between `apxm-runtime`'s capability module and its
//! executor module.
//!
//! # Why this crate exists
//!
//! `apxm-runtime` hosts `executor` and `capability` modules that would form a
//! `use`-graph cycle if `capability` depended on executor's context/hooks
//! directly: executor holds a concrete `Arc<CapabilitySystem>`, while
//! capability's interceptor pipeline needs to call back into executor's event
//! observer. Because both live in one crate today, the cycle isn't a
//! *compile* problem — but it blocks ever splitting them into separate
//! crates, since a naive split would recreate the cycle at the `Cargo.toml`
//! dependency level.
//!
//! This crate is the trait seam that breaks it. It defines:
//!
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
//!
//! Per ADR-0019, this crate holds no durable timer, wake-notification host
//! bridge, or process-global park registry seam. Making a node runnable is
//! exclusively the readiness kernel's decision; there is no second path into
//! `runnable` here.

mod facade;
mod metadata;
mod token_usage;

pub mod events;
pub mod sandbox;

pub use facade::{
    ApprovalContext, CapabilityEffectReplayEvidence, CapabilityEffectReplayEvidenceEnvelope,
    CapabilityFacade, CapabilityInvocation, CapabilityInvocationError, CapabilitySandboxPreflight,
    HostEffectRequestEvidence, capability_effect_idempotency_key_digest,
};
pub use metadata::{
    CapabilityGrantRuntimeQuota, CapabilityGrantScopeRequirements, CapabilityGrantScopeSelector,
    RuntimeCapability,
};
pub use token_usage::TokenUsageSummary;
