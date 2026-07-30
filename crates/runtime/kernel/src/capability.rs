//! The admitted runtime port for ordinary `capability.invoke` effects.
//!
//! The kernel owns the admitted port seam; the Agents-owned request and outcome
//! types enter execution only through a validated [`crate::PortBundle`].

pub use apxm_program::capability::{CapabilityOutcome, CapabilityRequest};
use async_trait::async_trait;

/// The admitted port for ordinary `capability.invoke` effects.
#[async_trait]
pub trait CapabilityPort: Send + Sync {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome;
}
