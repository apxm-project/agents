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

    /// Invoke through the authority-preserving canonical request seam.
    ///
    /// The default keeps existing injected ports source-compatible; concrete
    /// service ports that execute real capabilities must override this method
    /// so the request's typed authority reaches their capability system.
    async fn invoke_authorized(&self, request: CapabilityRequest) -> CapabilityOutcome {
        self.invoke(request).await
    }
}
