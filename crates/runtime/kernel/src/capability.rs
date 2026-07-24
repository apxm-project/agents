//! The admitted runtime port for ordinary `capability.invoke` effects.
//!
//! The kernel owns this contract because a normal Capability implementation
//! enters execution only through a validated [`crate::PortBundle`].

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

/// The admitted port for ordinary `capability.invoke` effects.
#[async_trait]
pub trait CapabilityPort: Send + Sync {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome;
}
