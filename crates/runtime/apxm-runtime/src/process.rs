//! Agent process — isolated execution context for an agent.
//!
//! An `AgentProcess` is the APXM equivalent of an OS process. It provides
//! isolation boundaries for agent execution: each agent runs in its own
//! process with its own state, session, and lifecycle.

use std::sync::Arc;
use tokio::sync::Mutex;

/// Unique identifier for an agent process.
pub type ProcessId = String;

/// An isolated agent execution context.
///
/// `Clone` is supported: `ProcessKind::External` clones the `Arc` reference
/// to the session (cheap reference count bump, not a deep copy).
pub struct AgentProcess {
    /// Unique process identifier (UUID v7).
    pub id: ProcessId,
    /// Human-readable agent name (used for lookup via ProcessTable).
    pub name: String,
    /// Parent process ID (the spawner), if any.
    pub parent_id: Option<ProcessId>,
    /// The kind of agent process (local flow or external subprocess).
    pub kind: ProcessKind,
    /// Current lifecycle state.
    pub state: ProcessState,
    /// When this process was spawned.
    pub spawned_at: std::time::Instant,
}

/// Distinguishes local flow-based agents from external ACP subprocess agents.
pub enum ProcessKind {
    /// Local agent running flows via ExecutorEngine.
    Local,
    /// External ACP agent subprocess (Claude, Codex, Gemini, etc.).
    ///
    /// The session is type-erased because apxm-runtime cannot depend on
    /// apxm-acp. The concrete type is `Arc<Mutex<AcpSession>>` — handlers
    /// with access to both crates perform the downcast.
    External {
        /// Type-erased ACP session handle.
        session: Arc<Mutex<dyn std::any::Any + Send + Sync>>,
        /// APXM ACP agent profile name.
        profile_name: String,
    },
}

impl Clone for AgentProcess {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            name: self.name.clone(),
            parent_id: self.parent_id.clone(),
            kind: self.kind.clone(),
            state: self.state.clone(),
            spawned_at: self.spawned_at,
        }
    }
}

impl Clone for ProcessKind {
    fn clone(&self) -> Self {
        match self {
            ProcessKind::Local => ProcessKind::Local,
            ProcessKind::External {
                session,
                profile_name,
            } => ProcessKind::External {
                session: Arc::clone(session),
                profile_name: profile_name.clone(),
            },
        }
    }
}

/// Lifecycle state of an agent process.
#[derive(Clone)]
pub enum ProcessState {
    /// Process is actively executing.
    Running,
    /// Process is idle (waiting for messages).
    Idle,
    /// Process has terminated.
    Terminated { reason: String },
}
