//! Recursive workflow composition.
//!
//! `WorkflowNode` unifies the three sub-workflow invocation patterns
//! (leaf operations, flow calls, external invocations) and adds an inline
//! `SubWorkflow` variant that can recursively contain other `WorkflowNode`s,
//! breaking the fixed 4-level Agent -> Flow -> Task -> Node hierarchy.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::context_contracts::ChildExecutionEnvelope;

use super::{Edge, Node};

pub const WORKFLOW_TARGET_KIND_REGISTERED_FLOW: &str = "registered_flow";
pub const WORKFLOW_TARGET_KIND_AIR_PATH: &str = "air_path";
pub const WORKFLOW_TARGET_KIND_ARTIFACT_PATH: &str = "artifact_path";
pub const WORKFLOW_TARGET_KIND_WORKFLOW_PATH: &str = "workflow_path";
pub const WORKFLOW_SPAWN_PATH_TARGET_KINDS: [&str; 3] = [
    WORKFLOW_TARGET_KIND_AIR_PATH,
    WORKFLOW_TARGET_KIND_ARTIFACT_PATH,
    WORKFLOW_TARGET_KIND_WORKFLOW_PATH,
];

/// High-level invocation mode for composed workflow execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowInvocationKind {
    Embed,
    FlowCall,
    WorkflowSpawn,
}

/// Explicit target of a composed workflow invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target_kind", rename_all = "snake_case")]
pub enum WorkflowTarget {
    RegisteredFlow {
        agent_name: String,
        flow_name: String,
    },
    AirPath {
        path: String,
    },
    ArtifactPath {
        path: String,
    },
    WorkflowPath {
        path: String,
    },
}

impl WorkflowTarget {
    pub fn kind_name(&self) -> &'static str {
        match self {
            WorkflowTarget::RegisteredFlow { .. } => WORKFLOW_TARGET_KIND_REGISTERED_FLOW,
            WorkflowTarget::AirPath { .. } => WORKFLOW_TARGET_KIND_AIR_PATH,
            WorkflowTarget::ArtifactPath { .. } => WORKFLOW_TARGET_KIND_ARTIFACT_PATH,
            WorkflowTarget::WorkflowPath { .. } => WORKFLOW_TARGET_KIND_WORKFLOW_PATH,
        }
    }

    pub fn from_path_target_kind(
        target_kind: &str,
        path: impl Into<String>,
    ) -> Result<Self, String> {
        let path = path.into();
        match target_kind {
            WORKFLOW_TARGET_KIND_AIR_PATH => Ok(WorkflowTarget::AirPath { path }),
            WORKFLOW_TARGET_KIND_ARTIFACT_PATH => Ok(WorkflowTarget::ArtifactPath { path }),
            WORKFLOW_TARGET_KIND_WORKFLOW_PATH => Ok(WorkflowTarget::WorkflowPath { path }),
            _ => Err(format!(
                "Unsupported workflow target_kind '{target_kind}'. Expected one of: {}",
                WORKFLOW_SPAWN_PATH_TARGET_KINDS.join(", ")
            )),
        }
    }

    /// Human-readable label for diagnostics and manifests.
    pub fn label(&self) -> String {
        match self {
            WorkflowTarget::RegisteredFlow {
                agent_name,
                flow_name,
            } => format!("{agent_name}.{flow_name}"),
            WorkflowTarget::AirPath { path }
            | WorkflowTarget::ArtifactPath { path }
            | WorkflowTarget::WorkflowPath { path } => path.clone(),
        }
    }
}

/// Typed boundary record for nested workflow invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChildExecutionAdmission {
    /// A child starts without parent grants, credentials, memory, prompt, or
    /// selected-skill context. This is the only default state.
    Isolated,
    /// The host has admitted the exact envelope, sealed context transport, and
    /// runtime-grant projection for this one child execution.
    Delegated {
        envelope: Box<ChildExecutionEnvelope>,
        sealed_context_transport: String,
        runtime_capability_grants: String,
    },
}

/// Typed boundary record for nested workflow invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowInvocation {
    pub kind: WorkflowInvocationKind,
    pub target: WorkflowTarget,
    #[serde(default)]
    pub args: HashMap<String, serde_json::Value>,
    #[serde(default = "default_await_result")]
    pub await_result: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_node_id: Option<u64>,
    pub child_execution_admission: ChildExecutionAdmission,
}

const fn default_await_result() -> bool {
    true
}

impl WorkflowInvocation {
    /// Returns true when the invocation creates a separate workflow execution.
    pub fn is_cross_execution(&self) -> bool {
        matches!(self.kind, WorkflowInvocationKind::WorkflowSpawn)
    }
}

/// A workflow step that can recursively contain sub-workflows.
///
/// This enum enables arbitrary nesting depth beyond the original fixed
/// 4-level hierarchy. Existing handlers continue to work with `Node`
/// directly; `WorkflowNode` is an overlay type that can be adopted
/// incrementally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowNode {
    /// A single AIS operation (leaf node).
    Operation(Node),

    /// Invoke another registered flow by name.
    FlowCall {
        flow_name: String,
        inputs: Vec<String>,
    },

    /// Invoke an external tool/capability.
    Invocation {
        capability: String,
        args: serde_json::Value,
    },

    /// Spawn another workflow/graph/artifact as a separate execution.
    WorkflowSpawn { invocation: Box<WorkflowInvocation> },

    /// An inline sub-workflow (recursive).
    SubWorkflow {
        name: String,
        nodes: Vec<WorkflowNode>,
        edges: Vec<Edge>,
    },
}

impl WorkflowNode {
    /// Recursively collect all leaf `Operation` nodes in depth-first order.
    pub fn flatten(&self) -> Vec<Node> {
        match self {
            WorkflowNode::Operation(node) => vec![node.clone()],
            WorkflowNode::FlowCall { .. }
            | WorkflowNode::Invocation { .. }
            | WorkflowNode::WorkflowSpawn { .. } => Vec::new(),
            WorkflowNode::SubWorkflow { nodes, .. } => {
                nodes.iter().flat_map(|n| n.flatten()).collect()
            }
        }
    }

    /// Maximum nesting depth of this workflow node.
    ///
    /// - Leaf variants (`Operation`, `FlowCall`, `Invocation`, `WorkflowSpawn`)
    ///   have depth 0.
    /// - A `SubWorkflow` has depth 1 + max depth of its children
    ///   (or 1 if it has no children).
    pub fn depth(&self) -> usize {
        match self {
            WorkflowNode::Operation(_)
            | WorkflowNode::FlowCall { .. }
            | WorkflowNode::Invocation { .. }
            | WorkflowNode::WorkflowSpawn { .. } => 0,
            WorkflowNode::SubWorkflow { nodes, .. } => {
                1 + nodes.iter().map(|n| n.depth()).max().unwrap_or(0)
            }
        }
    }

    /// Returns `true` when this node is a leaf (no further nesting).
    pub fn is_leaf(&self) -> bool {
        matches!(
            self,
            WorkflowNode::Operation(_)
                | WorkflowNode::FlowCall { .. }
                | WorkflowNode::Invocation { .. }
                | WorkflowNode::WorkflowSpawn { .. }
        )
    }

    /// Returns `true` when this node is a `SubWorkflow`.
    pub fn is_sub_workflow(&self) -> bool {
        matches!(self, WorkflowNode::SubWorkflow { .. })
    }

    /// Returns a human-readable label for this node variant.
    pub fn kind(&self) -> &'static str {
        match self {
            WorkflowNode::Operation(_) => "operation",
            WorkflowNode::FlowCall { .. } => "flow_call",
            WorkflowNode::Invocation { .. } => "invocation",
            WorkflowNode::WorkflowSpawn { .. } => "workflow_spawn",
            WorkflowNode::SubWorkflow { .. } => "sub_workflow",
        }
    }

    /// Total number of `WorkflowNode`s in this tree, including self.
    pub fn node_count(&self) -> usize {
        match self {
            WorkflowNode::Operation(_)
            | WorkflowNode::FlowCall { .. }
            | WorkflowNode::Invocation { .. }
            | WorkflowNode::WorkflowSpawn { .. } => 1,
            WorkflowNode::SubWorkflow { nodes, .. } => {
                1 + nodes.iter().map(|n| n.node_count()).sum::<usize>()
            }
        }
    }
}
