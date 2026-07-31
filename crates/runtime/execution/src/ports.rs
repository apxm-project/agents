//! The injected effect ports the canonical driver drives, beyond the model and
//! external-agent ports the kernel already defines. Each is an exact injected
//! implementation; the driver never discovers, ranks, or falls back.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use apxm_kernel::{CapabilityOutcome, CapabilityPort, CapabilityRequest};

/// The opaque identity of one OS-minted durable event.
///
/// Runtime code compares this identity exactly. It never interprets a selector,
/// route, or provider-specific payload to decide which parked invocation to
/// resume.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventRef(String);

impl EventRef {
    /// Construct one non-empty durable event identity.
    pub fn new(value: impl Into<String>) -> Result<Self, EventRefError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(EventRefError::Empty);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EventRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An invalid runtime event identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventRefError {
    Empty,
}

impl std::fmt::Display for EventRefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("event reference must not be empty"),
        }
    }
}

impl std::error::Error for EventRefError {}

/// A request to await one external event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventAwait {
    pub node_id: String,
    pub event_ref: EventRef,
}

/// The typed outcome of an `await.event` effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventOutcome {
    Fulfilled {
        event_ref: EventRef,
        payload: String,
    },
    Parked,
    Expired,
    Cancelled,
    Mismatched {
        delivered_event_ref: EventRef,
    },
}

/// The event port for `await.event` effects.
#[async_trait]
pub trait EventPort: Send + Sync {
    async fn await_event(&self, request: EventAwait) -> EventOutcome;
}

/// The typed receiver of a composition. Compiled AIR encodes `program.invoke`
/// as `operands.receiver.{program_ref | program_instance_ref}`; the union
/// distinguishes a one-shot `ProgramRef` invocation from a stateful invocation
/// of an already-created `ProgramInstanceRef`. `program.new` always carries a
/// `Program` receiver.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionReceiver {
    Program { program_ref: String },
    Instance { program_instance_ref: String },
}

impl CompositionReceiver {
    /// The reference string this receiver resolves against, regardless of kind.
    pub fn reference(&self) -> &str {
        match self {
            Self::Program { program_ref } => program_ref,
            Self::Instance {
                program_instance_ref,
            } => program_instance_ref,
        }
    }

    /// True when this receiver targets an existing stateful instance.
    pub fn is_instance(&self) -> bool {
        matches!(self, Self::Instance { .. })
    }
}

/// A request to compose a child Program (`program.new`/`program.invoke`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionRequest {
    pub node_id: String,
    pub receiver: CompositionReceiver,
}

/// The typed outcome of composing or invoking a child Program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
