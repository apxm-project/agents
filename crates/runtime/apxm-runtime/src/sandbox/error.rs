//! Sandbox error types.

use std::time::Duration;

/// Errors that can occur during sandbox operations.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// The requested backend is not available on this platform.
    #[error("sandbox backend not available: {0}")]
    NotAvailable(String),

    /// No registered backend satisfies the minimum isolation requirements.
    #[error("no backend satisfies requirements: {0}")]
    RequirementsNotMet(String),

    /// The command failed to execute inside the sandbox.
    #[error("sandbox execution failed: {0}")]
    ExecutionFailed(String),

    /// The command exceeded its timeout.
    #[error("sandbox execution timed out after {0:?}")]
    Timeout(Duration),

    /// The sandbox denied the operation (policy violation).
    #[error("sandbox permission denied: {0}")]
    PermissionDenied(String),

    /// Error creating or tearing down the sandbox context/session.
    #[error("sandbox session error: {0}")]
    SessionError(String),

    /// The backend validated the requirements and found them unsatisfiable.
    #[error("sandbox validation failed: {0}")]
    ValidationFailed(String),
}
