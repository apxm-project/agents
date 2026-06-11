//! ACP (Agent Client Protocol) client for APXM.
//!
//! Spawns coding agents (Claude Code, Codex, Gemini CLI, etc.) via subprocess,
//! speaks JSON-RPC 2.0 over stdio, and integrates with APXM's runtime systems.

pub mod auth;
pub mod constants;
pub mod content;
pub mod controls;
pub mod events;
pub mod protocol;
pub mod registry;
pub mod reverse;
pub mod session;
pub mod terminal;

pub mod aam_bridge;

pub use registry::{
    AcpAgentProfile, AgentRegistry, CapabilityServerConfig, PermissionMode,
    default_route_capabilities,
};
pub use session::AcpSession;

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("Failed to spawn agent '{agent}': {reason}")]
    Spawn { agent: String, reason: String },
    #[error("ACP protocol error: {0}")]
    Protocol(String),
    #[error("Agent returned JSON-RPC error {code}: {message}")]
    AgentError { code: i64, message: String },
    #[error("Session closed unexpectedly")]
    SessionClosed,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("Unknown reverse request method: {0}")]
    UnknownMethod(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}
