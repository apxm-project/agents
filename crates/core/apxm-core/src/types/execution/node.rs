//! Execution DAG node representation.
//!
//! Nodes represent operations in the execution DAG, with their inputs, outputs,
//! and metadata for scheduling.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::task::TaskId;
use crate::types::{AISOperationType, TokenId, Value, validate_operation};

/// Runtime-configurable latency tiers for model backends.
///
/// Maps backend names to estimated latency values (in nanoseconds) that
/// override the compile-time `estimated_latency` baked into the artifact.
/// This allows tuning cost budgets and scheduling weights without
/// recompilation.
///
/// When a node has a `"backend"` attribute matching a key in `tiers`,
/// its `estimated_latency` is overridden with the configured value.
/// Nodes without a matching backend keep their compile-time latency.
/// Nodes with no compile-time latency and no matching tier use
/// `default_latency_ns`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LatencyTierConfig {
    /// Backend name -> latency in nanoseconds.
    #[serde(default)]
    pub tiers: HashMap<String, u64>,
    /// Fallback latency (ns) for nodes that have a `backend` attribute
    /// but no matching tier entry.  Zero means "no override".
    #[serde(default = "default_latency_ns")]
    pub default_latency_ns: u64,
}

fn default_latency_ns() -> u64 {
    0
}

impl Default for LatencyTierConfig {
    fn default() -> Self {
        Self {
            tiers: HashMap::new(),
            default_latency_ns: 0,
        }
    }
}

impl LatencyTierConfig {
    /// Look up the runtime latency override for a given backend name.
    ///
    /// Returns `Some(latency_ns)` if the backend has a configured tier,
    /// or `Some(default_latency_ns)` when a non-zero default is set.
    /// Returns `None` when there is no override (preserving compile-time value).
    pub fn resolve(&self, backend: &str) -> Option<u64> {
        if let Some(&latency) = self.tiers.get(backend) {
            Some(latency)
        } else if self.default_latency_ns > 0 {
            Some(self.default_latency_ns)
        } else {
            None
        }
    }

    /// Returns `true` when no overrides are configured (the config is inert).
    pub fn is_empty(&self) -> bool {
        self.tiers.is_empty() && self.default_latency_ns == 0
    }
}

/// Type alias for node identifiers.
pub type NodeId = u64;

/// Metadata associated with a node for scheduling and optimization.
///
/// Fields with default values are omitted during serialization to keep the
/// JSON compact. When deserializing, missing fields fall back to their
/// defaults, so older payloads remain compatible.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NodeMetadata {
    /// Human-readable node name from the source graph.
    /// Runtime-only: never serialized to/from artifact binary format.
    /// Set during DAG lowering from ApxmGraph and used by ContextStack/CWD routing.
    #[serde(skip)]
    pub name: Option<String>,
    /// Priority for execution (higher = more important).
    /// Omitted from serialization when zero (the default).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub priority: u32,
    /// Estimated execution latency in nanoseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_latency: Option<u64>,
    /// Source task ID used to preserve task boundaries through lowering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_source_id: Option<TaskId>,
}

/// Returns `true` when a `u32` is zero (used by `skip_serializing_if`).
fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// Returns `true` when a `HashMap` is empty (used by `skip_serializing_if`).
fn is_empty_map<K, V>(m: &HashMap<K, V>) -> bool {
    m.is_empty()
}

/// Returns `true` when a `Vec` is empty (used by `skip_serializing_if`).
fn is_empty_vec<T>(v: &[T]) -> bool {
    v.is_empty()
}

/// Returns `true` when `NodeMetadata` equals its `Default`.
fn is_default_metadata(m: &NodeMetadata) -> bool {
    *m == NodeMetadata::default()
}

/// Represents a node in the execution DAG.
///
/// A node represents a single operation with its inputs, outputs, attributes,
/// and scheduling metadata.
///
/// Serialization is tuned for compact, readable JSON:
/// - Empty collections (`attributes`, `input_tokens`, `output_tokens`) are
///   omitted rather than emitted as `{}` / `[]`.
/// - Default `metadata` is omitted entirely.
/// - All fields use `#[serde(default)]` so older payloads without new fields
///   deserialize without error.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: NodeId,
    pub op_type: AISOperationType,
    #[serde(default, skip_serializing_if = "is_empty_map")]
    pub attributes: HashMap<String, Value>,
    #[serde(default, skip_serializing_if = "is_empty_vec")]
    pub input_tokens: Vec<TokenId>,
    #[serde(default, skip_serializing_if = "is_empty_vec")]
    pub output_tokens: Vec<TokenId>,
    #[serde(default, skip_serializing_if = "is_default_metadata")]
    pub metadata: NodeMetadata,
}

impl Node {
    pub fn new(id: NodeId, op_type: AISOperationType) -> Self {
        Node {
            id,
            op_type,
            attributes: HashMap::new(),
            input_tokens: Vec::new(),
            output_tokens: Vec::new(),
            metadata: NodeMetadata::default(),
        }
    }

    pub fn add_input_token(&mut self, token_id: TokenId) {
        self.input_tokens.push(token_id);
    }

    pub fn add_output_token(&mut self, token_id: TokenId) {
        self.output_tokens.push(token_id);
    }

    pub fn set_attribute(&mut self, key: String, value: Value) {
        self.attributes.insert(key, value);
    }

    pub fn get_attribute(&self, key: &str) -> Option<&Value> {
        self.attributes.get(key)
    }

    /// Validates attributes against the AIS op contract.
    pub fn validate(&self) -> Result<(), crate::types::ValidationError> {
        validate_operation(self.op_type, &self.attributes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::graph::attrs as graph_attrs;

    #[test]
    fn test_deserialization_with_missing_fields() {
        // Minimal JSON with only required fields should deserialize correctly,
        // filling in defaults for omitted optional fields.
        // Use the canonical serde name INV_TOOL ("INV" is a legacy fromStr alias,
        // not the serialized form — the Display / serde name is INV_TOOL).
        let json = r#"{"id":10,"op_type":"INV_TOOL"}"#;
        let node: Node = serde_json::from_str(json).expect("deserialize minimal node");

        assert_eq!(node.id, 10);
        assert_eq!(node.op_type, AISOperationType::InvTool);
        assert!(node.attributes.is_empty());
        assert!(node.input_tokens.is_empty());
        assert!(node.output_tokens.is_empty());
        assert_eq!(node.metadata, NodeMetadata::default());
    }

    #[test]
    fn test_roundtrip_with_all_fields() {
        let mut node = Node::new(5, AISOperationType::QMem);
        node.set_attribute(
            graph_attrs::QUERY.to_string(),
            Value::String("test".to_string()),
        );
        node.add_input_token(100);
        node.add_output_token(200);
        node.metadata.priority = 10;
        node.metadata.estimated_latency = Some(5000);
        node.metadata.task_source_id = Some(7);

        let json = serde_json::to_string(&node).expect("serialize");
        let restored: Node = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(node, restored);
    }

    #[test]
    fn test_validation() {
        let mut node = Node::new(1, AISOperationType::QMem);
        node.set_attribute(
            graph_attrs::QUERY.to_string(),
            Value::String("test".to_string()),
        );
        assert!(node.validate().is_ok());

        let node_invalid = Node::new(2, AISOperationType::QMem);
        assert!(node_invalid.validate().is_err());
    }

    // ── LatencyTierConfig tests ──────────────────────────────────────

    #[test]
    fn test_latency_tier_config_default_is_empty() {
        let cfg = LatencyTierConfig::default();
        assert!(cfg.is_empty());
        assert!(cfg.tiers.is_empty());
        assert_eq!(cfg.default_latency_ns, 0);
    }

    #[test]
    fn test_latency_tier_config_resolve_matching_tier() {
        let mut tiers = HashMap::new();
        tiers.insert("gpt-4".to_string(), 3000);
        tiers.insert("claude".to_string(), 2000);
        let cfg = LatencyTierConfig {
            tiers,
            default_latency_ns: 0,
        };

        assert_eq!(cfg.resolve("gpt-4"), Some(3000));
        assert_eq!(cfg.resolve("claude"), Some(2000));
        assert_eq!(cfg.resolve("unknown"), None);
    }

    #[test]
    fn test_latency_tier_config_resolve_default_fallback() {
        let cfg = LatencyTierConfig {
            tiers: HashMap::new(),
            default_latency_ns: 5000,
        };

        // No specific tier but default is set
        assert_eq!(cfg.resolve("any-backend"), Some(5000));
        assert!(!cfg.is_empty());
    }

    #[test]
    fn test_latency_tier_config_resolve_tier_over_default() {
        let mut tiers = HashMap::new();
        tiers.insert("fast".to_string(), 100);
        let cfg = LatencyTierConfig {
            tiers,
            default_latency_ns: 9999,
        };

        // Specific tier takes precedence
        assert_eq!(cfg.resolve("fast"), Some(100));
        // Unknown falls back to default
        assert_eq!(cfg.resolve("slow"), Some(9999));
    }

    #[test]
    fn test_latency_tier_config_serde_roundtrip() {
        let mut tiers = HashMap::new();
        tiers.insert("backend-a".to_string(), 1000);
        let cfg = LatencyTierConfig {
            tiers,
            default_latency_ns: 500,
        };

        let json = serde_json::to_string(&cfg).expect("serialize");
        let restored: LatencyTierConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(cfg, restored);
    }

    #[test]
    fn test_latency_tier_config_deserialize_empty() {
        // Empty JSON object should deserialize to defaults
        let json = "{}";
        let cfg: LatencyTierConfig = serde_json::from_str(json).expect("deserialize");
        assert!(cfg.is_empty());
    }
}
