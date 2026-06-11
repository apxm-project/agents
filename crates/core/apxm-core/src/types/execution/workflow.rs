//! Recursive workflow composition.
//!
//! `WorkflowNode` unifies the three sub-workflow invocation patterns
//! (leaf operations, flow calls, external invocations) and adds an inline
//! `SubWorkflow` variant that can recursively contain other `WorkflowNode`s,
//! breaking the fixed 4-level Agent -> Flow -> Task -> Node hierarchy.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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
pub struct WorkflowInvocation {
    pub kind: WorkflowInvocationKind,
    pub target: WorkflowTarget,
    #[serde(default)]
    pub args: HashMap<String, serde_json::Value>,
    #[serde(default = "default_await_result")]
    pub await_result: bool,
    #[serde(default)]
    pub session_root: Option<String>,
    #[serde(default)]
    pub session_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_node_id: Option<u64>,
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
    WorkflowSpawn { invocation: WorkflowInvocation },

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AISOperationType, NodeMetadata};

    fn make_node(id: u64, op: AISOperationType) -> Node {
        Node {
            id,
            op_type: op,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: NodeMetadata::default(),
        }
    }

    #[test]
    fn operation_leaf_depth_zero() {
        let wn = WorkflowNode::Operation(make_node(1, AISOperationType::InvTool));
        assert_eq!(wn.depth(), 0);
        assert!(wn.is_leaf());
        assert!(!wn.is_sub_workflow());
        assert_eq!(wn.kind(), "operation");
    }

    #[test]
    fn flow_call_leaf_depth_zero() {
        let wn = WorkflowNode::FlowCall {
            flow_name: "my_flow".into(),
            inputs: vec!["arg1".into()],
        };
        assert_eq!(wn.depth(), 0);
        assert!(wn.is_leaf());
        assert_eq!(wn.kind(), "flow_call");
    }

    #[test]
    fn invocation_leaf_depth_zero() {
        let wn = WorkflowNode::Invocation {
            capability: "search".into(),
            args: serde_json::json!({"q": "test"}),
        };
        assert_eq!(wn.depth(), 0);
        assert!(wn.is_leaf());
        assert_eq!(wn.kind(), "invocation");
    }

    #[test]
    fn workflow_spawn_leaf_depth_zero() {
        let wn = WorkflowNode::WorkflowSpawn {
            invocation: WorkflowInvocation {
                kind: WorkflowInvocationKind::WorkflowSpawn,
                target: WorkflowTarget::AirPath {
                    path: "steps/reviewer.air".into(),
                },
                args: HashMap::from([("topic".into(), serde_json::json!("apxm"))]),
                await_result: true,
                session_root: None,
                session_dir: None,
                parent_execution_id: None,
                parent_session_dir: None,
                parent_scope_id: None,
                spawn_node_id: None,
            },
        };
        assert_eq!(wn.depth(), 0);
        assert!(wn.is_leaf());
        assert_eq!(wn.kind(), "workflow_spawn");
    }

    #[test]
    fn flatten_operation_returns_node() {
        let node = make_node(42, AISOperationType::Ask);
        let wn = WorkflowNode::Operation(node.clone());
        let flat = wn.flatten();
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].id, 42);
    }

    #[test]
    fn flatten_flow_call_returns_empty() {
        let wn = WorkflowNode::FlowCall {
            flow_name: "f".into(),
            inputs: vec![],
        };
        assert!(wn.flatten().is_empty());
    }

    #[test]
    fn flatten_invocation_returns_empty() {
        let wn = WorkflowNode::Invocation {
            capability: "c".into(),
            args: serde_json::Value::Null,
        };
        assert!(wn.flatten().is_empty());
    }

    #[test]
    fn flatten_workflow_spawn_returns_empty() {
        let wn = WorkflowNode::WorkflowSpawn {
            invocation: WorkflowInvocation {
                kind: WorkflowInvocationKind::WorkflowSpawn,
                target: WorkflowTarget::WorkflowPath {
                    path: "workflow/review.apxmw".into(),
                },
                args: HashMap::new(),
                await_result: true,
                session_root: None,
                session_dir: None,
                parent_execution_id: None,
                parent_session_dir: None,
                parent_scope_id: None,
                spawn_node_id: None,
            },
        };
        assert!(wn.flatten().is_empty());
    }

    #[test]
    fn workflow_target_kind_names_are_stable() {
        assert_eq!(
            WorkflowTarget::RegisteredFlow {
                agent_name: "agent".into(),
                flow_name: "main".into(),
            }
            .kind_name(),
            WORKFLOW_TARGET_KIND_REGISTERED_FLOW
        );
        assert_eq!(
            WorkflowTarget::AirPath {
                path: "steps/reviewer.air".into(),
            }
            .kind_name(),
            WORKFLOW_TARGET_KIND_AIR_PATH
        );
        assert_eq!(
            WorkflowTarget::ArtifactPath {
                path: "build/reviewer.apxmobj".into(),
            }
            .kind_name(),
            WORKFLOW_TARGET_KIND_ARTIFACT_PATH
        );
        assert_eq!(
            WorkflowTarget::WorkflowPath {
                path: "workflow/review.apxmw".into(),
            }
            .kind_name(),
            WORKFLOW_TARGET_KIND_WORKFLOW_PATH
        );
    }

    #[test]
    fn workflow_spawn_target_kind_parser_accepts_supported_kinds() {
        assert!(matches!(
            WorkflowTarget::from_path_target_kind(
                WORKFLOW_TARGET_KIND_AIR_PATH,
                "steps/reviewer.air"
            ),
            Ok(WorkflowTarget::AirPath { .. })
        ));
        assert!(matches!(
            WorkflowTarget::from_path_target_kind(
                WORKFLOW_TARGET_KIND_ARTIFACT_PATH,
                "build/reviewer.apxmobj"
            ),
            Ok(WorkflowTarget::ArtifactPath { .. })
        ));
        assert!(matches!(
            WorkflowTarget::from_path_target_kind(
                WORKFLOW_TARGET_KIND_WORKFLOW_PATH,
                "workflow/review.apxmw"
            ),
            Ok(WorkflowTarget::WorkflowPath { .. })
        ));
    }

    #[test]
    fn workflow_spawn_target_kind_parser_rejects_unknown_kind() {
        let error = WorkflowTarget::from_path_target_kind("registered_flow", "oops")
            .expect_err("registered_flow is not a supported WORKFLOW_SPAWN target kind");
        assert!(error.contains(WORKFLOW_TARGET_KIND_AIR_PATH));
        assert!(error.contains(WORKFLOW_TARGET_KIND_ARTIFACT_PATH));
        assert!(error.contains(WORKFLOW_TARGET_KIND_WORKFLOW_PATH));
    }

    #[test]
    fn sub_workflow_depth_one_with_leaf_children() {
        let wn = WorkflowNode::SubWorkflow {
            name: "inner".into(),
            nodes: vec![
                WorkflowNode::Operation(make_node(1, AISOperationType::InvTool)),
                WorkflowNode::FlowCall {
                    flow_name: "helper".into(),
                    inputs: vec![],
                },
            ],
            edges: vec![],
        };
        assert_eq!(wn.depth(), 1);
        assert!(!wn.is_leaf());
        assert!(wn.is_sub_workflow());
        assert_eq!(wn.kind(), "sub_workflow");
    }

    #[test]
    fn sub_workflow_empty_children_depth_one() {
        let wn = WorkflowNode::SubWorkflow {
            name: "empty".into(),
            nodes: vec![],
            edges: vec![],
        };
        assert_eq!(wn.depth(), 1);
    }

    #[test]
    fn nested_sub_workflows_depth() {
        let wn = WorkflowNode::SubWorkflow {
            name: "level1".into(),
            nodes: vec![WorkflowNode::SubWorkflow {
                name: "level2".into(),
                nodes: vec![WorkflowNode::SubWorkflow {
                    name: "level3".into(),
                    nodes: vec![WorkflowNode::Operation(make_node(1, AISOperationType::Ask))],
                    edges: vec![],
                }],
                edges: vec![],
            }],
            edges: vec![],
        };
        assert_eq!(wn.depth(), 3);
    }

    #[test]
    fn flatten_nested_collects_all_operations() {
        let wn = WorkflowNode::SubWorkflow {
            name: "outer".into(),
            nodes: vec![
                WorkflowNode::Operation(make_node(1, AISOperationType::Ask)),
                WorkflowNode::SubWorkflow {
                    name: "inner".into(),
                    nodes: vec![
                        WorkflowNode::Operation(make_node(2, AISOperationType::Think)),
                        WorkflowNode::FlowCall {
                            flow_name: "other".into(),
                            inputs: vec![],
                        },
                        WorkflowNode::Operation(make_node(3, AISOperationType::InvTool)),
                    ],
                    edges: vec![],
                },
                WorkflowNode::Invocation {
                    capability: "tool".into(),
                    args: serde_json::Value::Null,
                },
                WorkflowNode::WorkflowSpawn {
                    invocation: WorkflowInvocation {
                        kind: WorkflowInvocationKind::WorkflowSpawn,
                        target: WorkflowTarget::ArtifactPath {
                            path: "dist/reviewer.apxmobj".into(),
                        },
                        args: HashMap::new(),
                        await_result: true,
                        session_root: None,
                        session_dir: None,
                        parent_execution_id: None,
                        parent_session_dir: None,
                        parent_scope_id: None,
                        spawn_node_id: None,
                    },
                },
            ],
            edges: vec![],
        };

        let flat = wn.flatten();
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].id, 1);
        assert_eq!(flat[1].id, 2);
        assert_eq!(flat[2].id, 3);
    }

    #[test]
    fn node_count_flat() {
        let wn = WorkflowNode::Operation(make_node(1, AISOperationType::Ask));
        assert_eq!(wn.node_count(), 1);
    }

    #[test]
    fn node_count_nested() {
        let wn = WorkflowNode::SubWorkflow {
            name: "outer".into(),
            nodes: vec![
                WorkflowNode::Operation(make_node(1, AISOperationType::Ask)),
                WorkflowNode::SubWorkflow {
                    name: "inner".into(),
                    nodes: vec![
                        WorkflowNode::Operation(make_node(2, AISOperationType::Think)),
                        WorkflowNode::Operation(make_node(3, AISOperationType::InvTool)),
                    ],
                    edges: vec![],
                },
            ],
            edges: vec![],
        };
        assert_eq!(wn.node_count(), 5);
    }

    #[test]
    fn serde_roundtrip_operation() {
        let wn = WorkflowNode::Operation(make_node(10, AISOperationType::QMem));
        let json = serde_json::to_string(&wn).unwrap();
        let restored: WorkflowNode = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.flatten().len(), 1);
        assert_eq!(restored.flatten()[0].id, 10);
    }

    #[test]
    fn serde_roundtrip_nested() {
        let wn = WorkflowNode::SubWorkflow {
            name: "root".into(),
            nodes: vec![
                WorkflowNode::Operation(make_node(1, AISOperationType::Ask)),
                WorkflowNode::FlowCall {
                    flow_name: "sub".into(),
                    inputs: vec!["x".into()],
                },
                WorkflowNode::WorkflowSpawn {
                    invocation: WorkflowInvocation {
                        kind: WorkflowInvocationKind::WorkflowSpawn,
                        target: WorkflowTarget::WorkflowPath {
                            path: "workflow/review.apxmw".into(),
                        },
                        args: HashMap::from([("depth".into(), serde_json::json!(2))]),
                        await_result: false,
                        session_root: None,
                        session_dir: None,
                        parent_execution_id: None,
                        parent_session_dir: None,
                        parent_scope_id: None,
                        spawn_node_id: None,
                    },
                },
                WorkflowNode::SubWorkflow {
                    name: "child".into(),
                    nodes: vec![WorkflowNode::Invocation {
                        capability: "tool".into(),
                        args: serde_json::json!({"key": "val"}),
                    }],
                    edges: vec![],
                },
            ],
            edges: vec![],
        };

        let json = serde_json::to_string_pretty(&wn).unwrap();
        let restored: WorkflowNode = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.depth(), 2);
        assert_eq!(restored.node_count(), 6);
        assert_eq!(restored.flatten().len(), 1);
    }

    #[test]
    fn deeply_nested_stress() {
        let mut current = WorkflowNode::Operation(make_node(1, AISOperationType::InvTool));
        for i in 0..50 {
            current = WorkflowNode::SubWorkflow {
                name: format!("level_{i}"),
                nodes: vec![current],
                edges: vec![],
            };
        }
        assert_eq!(current.depth(), 50);
        assert_eq!(current.flatten().len(), 1);
        assert_eq!(current.node_count(), 51);
    }

    #[test]
    fn workflow_target_label_formats_diagnostic_name() {
        let target = WorkflowTarget::RegisteredFlow {
            agent_name: "research".into(),
            flow_name: "main".into(),
        };
        assert_eq!(target.label(), "research.main");
    }

    #[test]
    fn workflow_invocation_cross_execution_only_for_spawn() {
        let flow_call = WorkflowInvocation {
            kind: WorkflowInvocationKind::FlowCall,
            target: WorkflowTarget::RegisteredFlow {
                agent_name: "research".into(),
                flow_name: "main".into(),
            },
            args: HashMap::new(),
            await_result: true,
            session_root: None,
            session_dir: None,
            parent_execution_id: None,
            parent_session_dir: None,
            parent_scope_id: None,
            spawn_node_id: None,
        };
        let spawned = WorkflowInvocation {
            kind: WorkflowInvocationKind::WorkflowSpawn,
            target: WorkflowTarget::WorkflowPath {
                path: "workflows/review.apxmw".into(),
            },
            args: HashMap::new(),
            await_result: true,
            session_root: None,
            session_dir: None,
            parent_execution_id: None,
            parent_session_dir: None,
            parent_scope_id: None,
            spawn_node_id: None,
        };
        assert!(!flow_call.is_cross_execution());
        assert!(spawned.is_cross_execution());
    }
}
