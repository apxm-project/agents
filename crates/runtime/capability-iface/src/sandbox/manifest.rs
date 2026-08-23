//! Security manifest — compiler output describing sandbox requirements.
//!
//! The APXM compiler statically analyzes a graph and emits a
//! [`SecurityManifest`] that tells the runtime what minimum isolation
//! level is needed and what capabilities each node requires.

use super::types::{ExecRequest, IsolationLevel, SandboxCapabilities};
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
/// | LINK-TOOLS    | QMEM, UMEM, SMEM, … | PolicyOnly | Agent memory read/write |
/// | LINK-RUNTIME  | ASK (w/ tools), INV, UPDATE_GOAL, EMIT | OsLevel | External I/O |
/// | T3   | RELEASE, RESUME, DELEGATE, SPAWN | Container | Multi-agent, resource claims |
pub mod tier {
    pub const PURE: u8 = 0;
    #[allow(dead_code)] // Reserved for memory-tier classification.
    pub const MEMORY: u8 = 1;
    pub const IO: u8 = 2;
    #[allow(dead_code)] // Reserved for privileged operations.
    pub const PRIVILEGED: u8 = 3;
}

/// Per-node sandbox requirements derived from static analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// The complete, typed requirement set projected from a security manifest.
///
/// Keeping this as a distinct type prevents backend selection from accidentally
/// consulting only the manifest's convenient aggregate isolation field. The
/// node requirements and all aggregate claims remain available to a backend
/// validator as one value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxRequirements {
    pub max_tier: u8,
    pub min_isolation: IsolationLevel,
    pub node_requirements: Vec<NodeSandboxReq>,
    pub needs_network: bool,
    pub needs_filesystem_write: bool,
    pub needs_process_spawn: bool,
    pub tool_capabilities_used: Vec<String>,
}

/// An invalid or internally minimized compiler security manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestValidationError(String);

impl std::fmt::Display for ManifestValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ManifestValidationError {}

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

    /// Validate and project every manifest claim into a backend-facing typed
    /// requirement set.
    pub fn requirements(&self) -> Result<SandboxRequirements, ManifestValidationError> {
        if self.max_tier > tier::PRIVILEGED {
            return Err(ManifestValidationError(format!(
                "max_tier {} is outside the supported 0..={}",
                self.max_tier,
                tier::PRIVILEGED
            )));
        }

        let mut max_node_tier = tier::PURE;
        let mut required_isolation = IsolationLevel::None;
        let mut node_ids = std::collections::HashSet::new();

        for node in &self.node_requirements {
            if !node_ids.insert(node.node_id) {
                return Err(ManifestValidationError(format!(
                    "node_requirements contains duplicate node_id {}",
                    node.node_id
                )));
            }
            if node.tier > tier::PRIVILEGED {
                return Err(ManifestValidationError(format!(
                    "node {} tier {} is outside the supported 0..={}",
                    node.node_id,
                    node.tier,
                    tier::PRIVILEGED
                )));
            }

            let (operation_tier, operation_isolation) = classify_operation(node.op);
            if node.tier < operation_tier {
                return Err(ManifestValidationError(format!(
                    "node {} minimizes {:?}: tier {} is below required {}",
                    node.node_id, node.op, node.tier, operation_tier
                )));
            }
            if node.min_isolation < operation_isolation {
                return Err(ManifestValidationError(format!(
                    "node {} minimizes {:?}: isolation {} is below required {}",
                    node.node_id, node.op, node.min_isolation, operation_isolation
                )));
            }

            let tier_isolation = isolation_for_tier(node.tier);
            if node.min_isolation < tier_isolation {
                return Err(ManifestValidationError(format!(
                    "node {} claims tier {} with only {} isolation",
                    node.node_id, node.tier, node.min_isolation
                )));
            }

            max_node_tier = max_node_tier.max(node.tier);
            required_isolation = required_isolation.max(node.min_isolation);
            for capability in &node.capabilities_used {
                if !self.tool_capabilities_used.contains(capability) {
                    return Err(ManifestValidationError(format!(
                        "node {} capability {:?} is absent from tool_capabilities_used",
                        node.node_id, capability
                    )));
                }
            }
        }

        if self.max_tier < max_node_tier {
            return Err(ManifestValidationError(format!(
                "max_tier {} minimizes node requirement tier {}",
                self.max_tier, max_node_tier
            )));
        }

        let aggregate_requires_io = self.needs_network
            || self.needs_filesystem_write
            || self.needs_process_spawn
            || !self.tool_capabilities_used.is_empty();
        if aggregate_requires_io {
            if self.max_tier < tier::IO {
                return Err(ManifestValidationError(
                    "aggregate I/O requirements are minimized by max_tier".to_string(),
                ));
            }
            required_isolation = required_isolation.max(IsolationLevel::OsLevel);
        }

        let max_tier_isolation = isolation_for_tier(self.max_tier);
        required_isolation = required_isolation.max(max_tier_isolation);
        if self.min_isolation < required_isolation {
            return Err(ManifestValidationError(format!(
                "min_isolation {} is below required {}",
                self.min_isolation, required_isolation
            )));
        }

        Ok(SandboxRequirements {
            max_tier: self.max_tier,
            min_isolation: self.min_isolation,
            node_requirements: self.node_requirements.clone(),
            needs_network: self.needs_network,
            needs_filesystem_write: self.needs_filesystem_write,
            needs_process_spawn: self.needs_process_spawn,
            tool_capabilities_used: self.tool_capabilities_used.clone(),
        })
    }

    /// Validate only, without retaining a second copy of the requirements.
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        self.requirements().map(|_| ())
    }
}

impl SandboxRequirements {
    /// Convert the request fields shared with command execution. Manifest-only
    /// fields remain available on this typed value for backend validation.
    pub fn as_exec_request(&self) -> ExecRequest {
        ExecRequest {
            min_isolation: self.min_isolation,
            needs_network: self.needs_network,
            needs_process_spawn: self.needs_process_spawn,
            ..ExecRequest::default()
        }
    }

    /// List every enforcement primitive required before this manifest can be
    /// admitted. OS-level manifests require the complete confinement bundle;
    /// accepting a partial bundle would silently weaken the compiler claim.
    pub fn missing_backend_capabilities(
        &self,
        capabilities: &SandboxCapabilities,
    ) -> Vec<&'static str> {
        let requires_os_confinement = self.min_isolation >= IsolationLevel::OsLevel
            || self.needs_network
            || self.needs_filesystem_write
            || self.needs_process_spawn
            || !self.tool_capabilities_used.is_empty();
        if !requires_os_confinement {
            return Vec::new();
        }

        let mut missing = Vec::new();
        if capabilities.isolation_level < self.min_isolation {
            missing.push("isolation-level");
        }
        if !capabilities.supports_filesystem_restriction {
            missing.push("filesystem-restriction");
        }
        if !capabilities.supports_network_restriction {
            missing.push("network-restriction");
        }
        if !capabilities.supports_process_restriction {
            missing.push("process-restriction");
        }
        if !capabilities.supports_syscall_filtering {
            missing.push("syscall-filtering");
        }
        if !capabilities.supports_resource_limits {
            missing.push("resource-limits");
        }
        missing
    }
}

fn isolation_for_tier(tier: u8) -> IsolationLevel {
    match tier {
        tier::PURE => IsolationLevel::None,
        tier::MEMORY => IsolationLevel::PolicyOnly,
        tier::IO => IsolationLevel::OsLevel,
        _ => IsolationLevel::Container,
    }
}

/// Classify the closed, typed AIS operation family. Unknown string-based
/// classifications remain available through [`classify_op`] for compatibility,
/// but manifest validation must use this typed mapping.
pub fn classify_operation(op: AISOperationType) -> (u8, IsolationLevel) {
    match op {
        AISOperationType::ModelCall => (tier::PURE, IsolationLevel::None),
        AISOperationType::CapabilityInvoke => (tier::IO, IsolationLevel::OsLevel),
        AISOperationType::ProgramNew | AISOperationType::ProgramInvoke => {
            (tier::PRIVILEGED, IsolationLevel::Container)
        }
        AISOperationType::AwaitEvent => (tier::IO, IsolationLevel::OsLevel),
    }
}

/// Classify an AIS operation into its sandbox tier.
#[allow(dead_code)] // Static analysis helper.
pub fn classify_op(op: &str) -> (u8, IsolationLevel) {
    match op {
        // T0: Pure LLM operations — no side effects
        "THINK" | "REASON" | "PLAN" | "REFLECT" | "VERIFY" | "EXPLAIN" | "SUMMARIZE"
        | "CRITIQUE" | "DECIDE" | "SCORE" | "RANK" | "CLASSIFY" | "EXTRACT" | "TRANSFORM"
        | "SELECT" | "MERGE" | "SPLIT" | "WAIT_ALL" | "FENCE" | "BRANCH" | "SWITCH" => {
            (tier::PURE, IsolationLevel::None)
        }

        // LINK-TOOLS: Memory operations — read/write agent state
        "QMEM" | "UMEM" | "SMEM" | "AMEM" | "RMEM" | "STM_PUT" | "STM_GET" => {
            (tier::MEMORY, IsolationLevel::PolicyOnly)
        }

        // LINK-RUNTIME: I/O operations — external tool calls, filesystem, network
        "ASK" | "INV" | "UPDATE_GOAL" | "EMIT" | "COMMUNICATE" => {
            (tier::IO, IsolationLevel::OsLevel)
        }

        // T3: Privileged operations — multi-agent coordination
        "RELEASE" | "RESUME" | "DELEGATE" | "SPAWN" => {
            (tier::PRIVILEGED, IsolationLevel::Container)
        }

        // Unknown ops default to I/O tier (safe by default)
        _ => (tier::IO, IsolationLevel::OsLevel),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_operation_classification_is_used_for_manifest_validation() {
        let manifest = SecurityManifest {
            max_tier: tier::PURE,
            min_isolation: IsolationLevel::None,
            node_requirements: vec![NodeSandboxReq {
                node_id: 7,
                op: AISOperationType::CapabilityInvoke,
                tier: tier::PURE,
                min_isolation: IsolationLevel::None,
                capabilities_used: Vec::new(),
            }],
            ..SecurityManifest::default()
        };

        let error = manifest.validate().expect_err("minimized node claim");
        assert!(error.to_string().contains("minimizes"));
    }

    #[test]
    fn aggregate_claims_cannot_minimize_the_graph() {
        let manifest = SecurityManifest {
            max_tier: tier::PURE,
            needs_network: true,
            ..SecurityManifest::default()
        };

        let error = manifest.validate().expect_err("minimized aggregate claim");
        assert!(error.to_string().contains("aggregate I/O"));
    }
}
