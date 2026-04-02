//! Agent thread — a concurrent operation within a process.
//!
//! An `AgentThread` tracks a single node execution within an agent process.
//! All threads in a process share the process's AAM state, memory, and
//! capability system — like OS threads sharing an address space.

use crate::process::ProcessId;
use apxm_core::types::operations::AISOperationType;

/// Unique identifier for an agent thread.
pub type ThreadId = String;

/// A concurrent operation within an agent process.
pub struct AgentThread {
    /// Unique thread identifier (UUID v7).
    pub id: ThreadId,
    /// The process this thread belongs to.
    pub process_id: ProcessId,
    /// The DAG node being executed.
    pub node_id: u64,
    /// The AIS operation type being executed.
    pub op_type: AISOperationType,
    /// Current thread state.
    pub state: ThreadState,
    /// When this thread started.
    pub started_at: std::time::Instant,
}

/// Lifecycle state of an agent thread.
pub enum ThreadState {
    /// Thread is ready to run.
    Ready,
    /// Thread is actively executing.
    Running,
    /// Thread is blocked waiting on something.
    Blocked { reason: String },
    /// Thread completed successfully.
    Completed,
    /// Thread failed with an error.
    Failed { error: String },
}
