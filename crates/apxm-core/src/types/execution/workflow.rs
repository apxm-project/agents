//! Recursive workflow composition.
//!
//! `WorkflowNode` unifies the three sub-workflow invocation patterns
//! (leaf operations, flow calls, external invocations) and adds an inline
//! `SubWorkflow` variant that can recursively contain other `WorkflowNode`s,
//! breaking the fixed 4-level Agent -> Flow -> Task -> Node hierarchy.

use serde::{Deserialize, Serialize};

use super::{Edge, Node};

/// A workflow step that can recursively contain sub-workflows.
///
/// This enum enables arbitrary nesting depth beyond the original fixed
/// 4-level hierarchy.  Existing handlers continue to work with `Node`
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
            WorkflowNode::FlowCall { .. } | WorkflowNode::Invocation { .. } => Vec::new(),
            WorkflowNode::SubWorkflow { nodes, .. } => {
                nodes.iter().flat_map(|n| n.flatten()).collect()
            }
        }
    }

    /// Maximum nesting depth of this workflow node.
    ///
    /// - Leaf variants (`Operation`, `FlowCall`, `Invocation`) have depth 0.
    /// - A `SubWorkflow` has depth 1 + max depth of its children
    ///   (or 1 if it has no children).
    pub fn depth(&self) -> usize {
        match self {
            WorkflowNode::Operation(_)
            | WorkflowNode::FlowCall { .. }
            | WorkflowNode::Invocation { .. } => 0,
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
            WorkflowNode::SubWorkflow { .. } => "sub_workflow",
        }
    }

    /// Total number of `WorkflowNode`s in this tree, including self.
    pub fn node_count(&self) -> usize {
        match self {
            WorkflowNode::Operation(_)
            | WorkflowNode::FlowCall { .. }
            | WorkflowNode::Invocation { .. } => 1,
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
    use std::collections::HashMap;

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
        // depth-3 tree:
        //   SubWorkflow (level1)
        //     SubWorkflow (level2)
        //       SubWorkflow (level3)
        //         Operation (leaf, depth 0)
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
        // outer(1) + op(1) + inner(1) + op(1) + op(1) = 5
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
        assert_eq!(restored.node_count(), 5);
        assert!(restored.flatten().len() == 1); // only the Operation
    }

    #[test]
    fn deeply_nested_stress() {
        // Build 50 levels of nesting to make sure recursion works
        let mut current = WorkflowNode::Operation(make_node(1, AISOperationType::InvTool));
        for i in 0..50 {
            current = WorkflowNode::SubWorkflow {
                name: format!("level_{}", i),
                nodes: vec![current],
                edges: vec![],
            };
        }
        assert_eq!(current.depth(), 50);
        assert_eq!(current.flatten().len(), 1);
        assert_eq!(current.node_count(), 51); // 50 SubWorkflows + 1 Operation
    }
}
