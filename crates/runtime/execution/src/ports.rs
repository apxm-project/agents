//! The injected effect ports the canonical driver drives, beyond the model and
//! external-agent ports the kernel already defines. Each is an exact injected
//! implementation; the driver never discovers, ranks, or falls back.

use async_trait::async_trait;
use serde_json::Value;

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

/// The canonical memory scope for scoped session state. Short-term memory is
/// session-scoped conversational state; long-term memory is durable knowledge.
/// This is an owner-defined closed vocabulary, not a re-export of the legacy
/// engine's space enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemorySpace {
    Stm,
    Ltm,
}

/// Why a scoped-memory operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryError {
    pub message: String,
}

impl MemoryError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "scoped memory error: {}", self.message)
    }
}

impl std::error::Error for MemoryError {}

/// Scoped read/write/delete over session memory. It is the plane-correct
/// replacement for the session family's direct use of the legacy engine's
/// `MemorySystem`: an injected port with no ambient global store, keyed by an
/// explicit `(space, scope, key)` triple.
#[async_trait]
pub trait ScopedMemoryPort: Send + Sync {
    /// Read the value at `(space, scope, key)`, if present.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError`] when the underlying store read fails.
    async fn read_scoped(
        &self,
        space: MemorySpace,
        scope: &str,
        key: &str,
    ) -> Result<Option<Value>, MemoryError>;

    /// Write `value` at `(space, scope, key)`.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError`] when the underlying store write fails.
    async fn write_scoped(
        &self,
        space: MemorySpace,
        scope: &str,
        key: &str,
        value: Value,
    ) -> Result<(), MemoryError>;

    /// Delete the value at `(space, scope, key)`. Deleting an absent key is not
    /// an error.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError`] when the underlying store delete fails.
    async fn delete_scoped(
        &self,
        space: MemorySpace,
        scope: &str,
        key: &str,
    ) -> Result<(), MemoryError>;
}
