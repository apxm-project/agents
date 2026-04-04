//! APXM graph metadata types sent to vLLM for scheduling hints.
//!
//! These structures mirror the Python-side `ApxmRequestHints` and
//! `ApxmGraphRegisterRequest` schemas in the vLLM fork.

use serde::{Deserialize, Serialize};

/// KV-cache pin policy for a single request.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PinPolicy {
    /// "prefix" — pin the prompt's KV blocks after completion; "none" — no pinning.
    pub mode: String,
    /// TTL for the pin in milliseconds. None = use graph-level default (30 s fallback).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u32>,
}

impl PinPolicy {
    /// No pinning (default).
    pub fn none() -> Self {
        Self {
            mode: "none".to_string(),
            ttl_ms: None,
        }
    }

    /// Pin the prompt prefix blocks for `ttl_ms` milliseconds.
    pub fn prefix(ttl_ms: u32) -> Self {
        Self {
            mode: "prefix".to_string(),
            ttl_ms: Some(ttl_ms),
        }
    }
}

/// Compiler-level hints that inform vLLM's eager-prefill and warmup decisions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompilerHints {
    /// Estimated number of tokens in a shared prefix (enables eager prefill).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_prefix_est_tokens: Option<u32>,
    /// True if vLLM should pre-warm KV cache for this request's prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_candidate: Option<bool>,
    /// True if this node will be re-used across pipeline iterations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline_candidate: Option<bool>,
}

/// Per-request APXM scheduling hints injected into `extra_body.apxm`.
///
/// These are sent in every chat-completion request to vLLM and parsed
/// by `ApxmRequestHints.from_extra_args()` in the vLLM fork.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApxmGraphHints {
    /// Schema version — always 1 for now.
    pub schema_version: u8,
    /// The execution graph this request belongs to (matches the registered graph_id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_id: Option<String>,
    /// Unique identifier for this specific graph execution run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    /// Numeric node id within the DAG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<u32>,
    /// Human-readable node name (debug / tracing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// "critical_path" → scheduler bumps priority; "parallel" → normal; None → default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_class: Option<String>,
    /// Nodes that depend on this node's output (used for prefetch decisions).
    pub downstream_nodes: Vec<u32>,
    /// KV-cache reuse group — nodes with the same group share pinned blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_group: Option<String>,
    /// KV-cache pin policy for this request.
    pub pin_policy: PinPolicy,
    /// Compiler hints for eager prefill and warmup.
    pub compiler_hints: CompilerHints,
}

impl Default for ApxmGraphHints {
    fn default() -> Self {
        Self {
            schema_version: 1,
            graph_id: None,
            execution_id: None,
            node_id: None,
            node_name: None,
            priority_class: None,
            downstream_nodes: Vec::new(),
            reuse_group: None,
            pin_policy: PinPolicy::none(),
            compiler_hints: CompilerHints::default(),
        }
    }
}

impl ApxmGraphHints {
    /// Create hints for a critical-path node with KV-cache pinning.
    pub fn critical_path(
        graph_id: impl Into<String>,
        execution_id: impl Into<String>,
        node_id: u32,
        node_name: impl Into<String>,
        downstream_nodes: Vec<u32>,
        pin_ttl_ms: u32,
    ) -> Self {
        Self {
            schema_version: 1,
            graph_id: Some(graph_id.into()),
            execution_id: Some(execution_id.into()),
            node_id: Some(node_id),
            node_name: Some(node_name.into()),
            priority_class: Some("critical_path".to_string()),
            downstream_nodes,
            reuse_group: None,
            pin_policy: PinPolicy::prefix(pin_ttl_ms),
            compiler_hints: CompilerHints::default(),
        }
    }

    /// Create hints for a parallel branch node.
    pub fn parallel(
        graph_id: impl Into<String>,
        execution_id: impl Into<String>,
        node_id: u32,
        node_name: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            graph_id: Some(graph_id.into()),
            execution_id: Some(execution_id.into()),
            node_id: Some(node_id),
            node_name: Some(node_name.into()),
            priority_class: Some("parallel".to_string()),
            downstream_nodes: Vec::new(),
            reuse_group: None,
            pin_policy: PinPolicy::none(),
            compiler_hints: CompilerHints::default(),
        }
    }

    /// True if any graph context fields are set.
    pub fn has_graph_context(&self) -> bool {
        self.graph_id.is_some() || self.node_id.is_some()
    }
}

/// Per-node spec for graph registration (`POST /v1/apxm/graphs/register`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpec {
    pub node_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_prompt_tokens: Option<u32>,
    pub downstream_nodes: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_group: Option<String>,
    pub is_critical_path: bool,
}

/// Graph metadata sent to `POST /v1/apxm/graphs/register`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMetadata {
    pub graph_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_path_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<u32>,
    pub nodes: Vec<NodeSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_pin_ttl_ms: Option<u32>,
}

impl GraphMetadata {
    /// Create a minimal graph metadata object with just an id and node count.
    pub fn new(graph_id: impl Into<String>, execution_id: impl Into<String>) -> Self {
        Self {
            graph_id: graph_id.into(),
            execution_id: Some(execution_id.into()),
            critical_path_length: None,
            node_count: None,
            max_parallelism: None,
            nodes: Vec::new(),
            default_pin_ttl_ms: None,
        }
    }

    /// Set default KV-cache pin TTL for all nodes in this graph.
    pub fn with_pin_ttl(mut self, ttl_ms: u32) -> Self {
        self.default_pin_ttl_ms = Some(ttl_ms);
        self
    }

    /// Set critical path length (number of sequential nodes).
    pub fn with_critical_path_length(mut self, len: u32) -> Self {
        self.critical_path_length = Some(len);
        self
    }

    /// Add per-node specs.
    pub fn with_nodes(mut self, nodes: Vec<NodeSpec>) -> Self {
        self.node_count = Some(nodes.len() as u32);
        self.nodes = nodes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_critical_path_hints_serialization() {
        let hints = ApxmGraphHints::critical_path(
            "graph-abc",
            "exec-123",
            5,
            "reason-node",
            vec![6, 7],
            30_000,
        );
        let json = serde_json::to_value(&hints).unwrap();
        assert_eq!(json["graph_id"], "graph-abc");
        assert_eq!(json["node_id"], 5);
        assert_eq!(json["priority_class"], "critical_path");
        assert_eq!(json["pin_policy"]["mode"], "prefix");
        assert_eq!(json["pin_policy"]["ttl_ms"], 30_000);
        assert_eq!(json["downstream_nodes"], serde_json::json!([6, 7]));
        assert_eq!(json["schema_version"], 1);
    }

    #[test]
    fn test_parallel_hints_no_pin() {
        let hints = ApxmGraphHints::parallel("graph-abc", "exec-123", 3, "search-node");
        let json = serde_json::to_value(&hints).unwrap();
        assert_eq!(json["priority_class"], "parallel");
        assert_eq!(json["pin_policy"]["mode"], "none");
        assert!(json.get("pin_policy").unwrap().get("ttl_ms").is_none());
    }

    #[test]
    fn test_default_hints_has_no_graph_context() {
        let hints = ApxmGraphHints::default();
        assert!(!hints.has_graph_context());
    }

    #[test]
    fn test_graph_metadata_builder() {
        let meta = GraphMetadata::new("graph-xyz", "exec-456")
            .with_pin_ttl(60_000)
            .with_critical_path_length(4)
            .with_nodes(vec![NodeSpec {
                node_id: 0,
                node_name: Some("plan".to_string()),
                estimated_prompt_tokens: Some(500),
                downstream_nodes: vec![1, 2],
                priority_class: Some("critical_path".to_string()),
                reuse_group: None,
                is_critical_path: true,
            }]);
        assert_eq!(meta.graph_id, "graph-xyz");
        assert_eq!(meta.default_pin_ttl_ms, Some(60_000));
        assert_eq!(meta.critical_path_length, Some(4));
        assert_eq!(meta.node_count, Some(1));
        assert_eq!(meta.nodes[0].node_id, 0);
    }

    #[test]
    fn test_pin_policy_constructors() {
        let none = PinPolicy::none();
        assert_eq!(none.mode, "none");
        assert!(none.ttl_ms.is_none());

        let prefix = PinPolicy::prefix(45_000);
        assert_eq!(prefix.mode, "prefix");
        assert_eq!(prefix.ttl_ms, Some(45_000));
    }
}
