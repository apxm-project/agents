//! The injected effect ports the canonical driver drives, beyond the model and
//! external-agent ports the kernel already defines. Each is an exact injected
//! implementation; the driver never discovers, ranks, or falls back.

use async_trait::async_trait;
use serde_json::Value;

pub use apxm_kernel::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionReceiver, CompositionRequest, EventApplication, EventApplicationResult, EventAwait,
    EventOutcome, EventPort, EventRef, EventRefError,
};

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
