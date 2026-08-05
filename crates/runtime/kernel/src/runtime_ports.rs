//! Kernel-owned runtime effect ports that are shared by every execution profile.
//!
//! These seams are intentionally transport-neutral. The kernel admits the
//! exact implementation and binding; the execution driver only dispatches the
//! already-admitted port. No host or product protocol is defined here.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The opaque identity of one OS-minted durable event.
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
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// An invalid runtime event identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventRefError {
    Empty,
}

impl std::fmt::Display for EventRefError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("event reference must be non-empty"),
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

/// The admitted port for durable `await.event` effects.
#[async_trait]
pub trait EventPort: Send + Sync {
    async fn await_event(&self, request: EventAwait) -> EventOutcome;
}

/// The typed receiver of a Program composition operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionReceiver {
    Program { program_ref: String },
    Instance { program_instance_ref: String },
}

impl CompositionReceiver {
    /// The exact reference carried by this receiver.
    pub fn reference(&self) -> &str {
        match self {
            Self::Program { program_ref } => program_ref,
            Self::Instance {
                program_instance_ref,
            } => program_instance_ref,
        }
    }

    #[must_use]
    pub fn is_instance(&self) -> bool {
        matches!(self, Self::Instance { .. })
    }
}

/// A request to compose a child Program.
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

/// The admitted port for Program creation and invocation.
#[async_trait]
pub trait CompositionPort: Send + Sync {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome;
    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome;
}
