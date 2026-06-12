//! Security manifest — compiler output describing sandbox requirements.
//!
//! The APXM compiler statically analyzes a graph and emits a
//! [`SecurityManifest`] that tells the runtime what minimum isolation
//! level is needed and what capabilities each node requires.

use super::types::IsolationLevel;
use apxm_core::types::operations::AISOperationType;
use serde::{Deserialize, Serialize};

/// AIS operation sandbox tiers.
///
/// Each AIS operation is classified into exactly one tier based on its
/// side-effect profile. The compiler uses this classification to derive
/// the minimum isolation level for the entire graph.
///
/// | Tier | Operations | Min Isolation | Rationale |
/// |------|-----------|---------------|-----------|
/// | T0   | THINK, REASON, PLAN, REFLECT, VERIFY, … | None | Pure LLM, no side effects |
/// | T1   | QMEM, UMEM, SMEM, … | PolicyOnly | Agent memory read/write |
/// | T2   | ASK (w/ tools), INV, UPDATE_GOAL, EMIT | OsLevel | External I/O |
/// | T3   | GUARD, CLAIM, RELEASE, RESUME, DELEGATE, SPAWN | Container | Multi-agent, resource claims |
pub mod tier {
    pub const PURE: u8 = 0;
    #[allow(dead_code)] // Reserved for memory-tier classification (not yet wired to runtime)
    pub const MEMORY: u8 = 1;
    pub const IO: u8 = 2;
    #[allow(dead_code)] // Reserved for privileged operations (not yet wired to runtime)
    pub const PRIVILEGED: u8 = 3;
}

/// Per-node sandbox requirements derived from static analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSandboxReq {
    /// The node ID in the graph.
    pub node_id: u32,
    /// The AIS operation.
    pub op: AISOperationType,
    /// The sandbox tier (0–3).
    pub tier: u8,
    /// The minimum isolation level for this node.
    pub min_isolation: IsolationLevel,
    /// Capabilities/tools this node invokes (for INV nodes).
    pub capabilities_used: Vec<String>,
}

/// Compiler output: security requirements for an entire graph.
///
/// The runtime uses this to select an appropriate [`SandboxBackend`](crate::SandboxBackend)
/// and to decide which nodes require sandboxed execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityManifest {
    /// Highest tier present in the graph.
    pub max_tier: u8,
    /// Minimum isolation level needed for the entire graph.
    pub min_isolation: IsolationLevel,
    /// Per-node requirements (empty if analysis was skipped).
    pub node_requirements: Vec<NodeSandboxReq>,
    /// Whether any node requires network access.
    pub needs_network: bool,
    /// Whether any node writes to the filesystem.
    pub needs_filesystem_write: bool,
    /// Whether any node spawns child processes.
    pub needs_process_spawn: bool,
    /// All tool/capability names referenced by INV nodes.
    pub tool_capabilities_used: Vec<String>,
}

impl Default for SecurityManifest {
    fn default() -> Self {
        Self {
            max_tier: tier::PURE,
            min_isolation: IsolationLevel::None,
            node_requirements: Vec::new(),
            needs_network: false,
            needs_filesystem_write: false,
            needs_process_spawn: false,
            tool_capabilities_used: Vec::new(),
        }
    }
}

impl SecurityManifest {
    /// Create a manifest for a pure graph (LLM-only, no tools).
    pub fn pure() -> Self {
        Self::default()
    }

    /// Create a manifest that requires at least the given isolation level.
    pub fn with_min_isolation(mut self, level: IsolationLevel) -> Self {
        self.min_isolation = level;
        self
    }

    /// Whether any node in the graph requires sandboxed execution.
    pub fn requires_sandbox(&self) -> bool {
        self.max_tier >= tier::IO
    }

    /// Whether the graph is pure (LLM-only, no tool invocations).
    pub fn is_pure(&self) -> bool {
        self.max_tier == tier::PURE
    }
}

/// Classify an AIS operation into its sandbox tier.
#[allow(dead_code)] // Planned for static analysis phase, not yet wired
pub fn classify_op(op: &str) -> (u8, IsolationLevel) {
    match op {
        // T0: Pure LLM operations — no side effects
        "THINK" | "REASON" | "PLAN" | "REFLECT" | "VERIFY" | "EXPLAIN" | "SUMMARIZE"
        | "CRITIQUE" | "DECIDE" | "SCORE" | "RANK" | "CLASSIFY" | "EXTRACT" | "TRANSFORM"
        | "SELECT" | "MERGE" | "SPLIT" | "WAIT_ALL" | "FENCE" | "BRANCH" | "SWITCH" => {
            (tier::PURE, IsolationLevel::None)
        }

        // T1: Memory operations — read/write agent state
        "QMEM" | "UMEM" | "SMEM" | "AMEM" | "RMEM" | "STM_PUT" | "STM_GET" => {
            (tier::MEMORY, IsolationLevel::PolicyOnly)
        }

        // T2: I/O operations — external tool calls, filesystem, network
        "ASK" | "INV" | "UPDATE_GOAL" | "EMIT" | "COMMUNICATE" => {
            (tier::IO, IsolationLevel::OsLevel)
        }

        // T3: Privileged operations — multi-agent coordination
        "GUARD" | "CLAIM" | "RELEASE" | "RESUME" | "DELEGATE" | "SPAWN" => {
            (tier::PRIVILEGED, IsolationLevel::Container)
        }

        // Unknown ops default to I/O tier (safe by default)
        _ => (tier::IO, IsolationLevel::OsLevel),
    }
}

