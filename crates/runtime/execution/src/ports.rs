//! The injected effect ports the canonical driver drives, beyond the model and
//! external-agent ports the kernel already defines. Each is an exact injected
//! implementation; the driver never discovers, ranks, or falls back.

use async_trait::async_trait;

/// A request to invoke one non-model Capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub node_id: String,
    pub capability_ref: String,
}

/// The typed outcome of a Capability invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityOutcome {
    Completed { result: String },
    Failed { message: String },
    OutcomeUnknown { message: String },
}

/// The Capability port for `capability.invoke` effects that are not External
/// Agent capabilities.
#[async_trait]
pub trait CapabilityPort: Send + Sync {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome;
}

/// A request to await one external event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventAwait {
    pub node_id: String,
    pub selector: String,
}

/// The typed outcome of an `await.event` effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventOutcome {
    Fulfilled { payload: String },
    Parked,
    Expired,
}

/// The event port for `await.event` effects.
#[async_trait]
pub trait EventPort: Send + Sync {
    async fn await_event(&self, request: EventAwait) -> EventOutcome;
}

/// A request to compose a child Program (`program.new`/`program.invoke`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositionRequest {
    pub node_id: String,
    pub program_ref: String,
}

/// The typed outcome of composing or invoking a child Program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompositionOutcome {
    Created { child_instance_ref: String },
    Invoked { child_instance_ref: String },
    Failed { message: String },
}

/// The composition port for `program.new`/`program.invoke`/`instance.invoke`.
/// It creates and invokes child Program instances; it never spawns a peer
/// process or resolves an implementation itself.
#[async_trait]
pub trait CompositionPort: Send + Sync {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome;
    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome;
}
