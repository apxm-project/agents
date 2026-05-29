//! Plan-graph wire DTO — the single source of truth for the "plan as graph"
//! emission schema (`apxm-plan-as-graph/schema.json`).
//!
//! This is the typed shape that the apxm-server plan tool deserializes and that
//! external authoring front-ends (e.g. apxm-studio) lower a canvas into. Keeping
//! it here, beside the AIS operation definitions, means there is exactly one
//! definition of the wire vocabulary; every consumer depends on this crate
//! instead of re-declaring parallel copies.
//!
//! AIR is position-free: node `x/y` never appears here (layout lives in a
//! separate sidecar so the content hash is layout-independent).

use serde::{Deserialize, Serialize};

use crate::AISOperationType;

/// A complete plan graph — the artifact handed to the compiler / dispatched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanGraph {
    pub name: String,
    pub entry: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<PlanParameter>,
    pub nodes: Vec<PlanNode>,
}

/// A graph parameter (workflow input). Trigger payloads bind these.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: PlanParamType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// A plan-graph node. Field requiredness depends on `op` (the compiler enforces
/// it; conformant lowering produces valid nodes).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanNode {
    pub id: u64,
    pub name: String,
    pub op: PlanNodeOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// For `inv_tool`: the capability id (integration / plugin / social action).
    /// Credential material is NEVER inlined here — only a path reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
    /// For `ask`/`agent`/`think`: per-node output token budget, lowered to the
    /// `token_budget` attribute (runtime `max_tokens`). `None` defers to the
    /// provider/model default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<PlanDependency>,
}

/// A typed dataflow dependency edge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanDependency {
    pub node: u64,
    pub dependency: PlanDependencyKind,
}

/// Edge kind. `Data` carries values, `Control` orders/gates a branch, `Effect`
/// serializes side effects. Wire-serialized as `Data`/`Control`/`Effect`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanDependencyKind {
    Data,
    Control,
    Effect,
}

/// Parameter value type. Wire-serialized as `str`/`int`/`float`/`bool`/`json`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlanParamType {
    Str,
    Int,
    Float,
    Bool,
    Json,
}

impl PlanParamType {
    /// Stable wire token (matches the schema enum and serde representation).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Str => "str",
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::Json => "json",
        }
    }
}

/// The wire op set (subset of [`AISOperationType`] surfaced by the plan schema).
/// Wire-serialized as `agent`/`ask`/`think`/`inv_tool`/`wait_all`/`yield`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanNodeOp {
    Agent,
    Ask,
    Think,
    InvTool,
    WaitAll,
    Yield,
}

impl PlanNodeOp {
    /// Stable wire token (matches the schema enum and serde representation).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Ask => "ask",
            Self::Think => "think",
            Self::InvTool => "inv_tool",
            Self::WaitAll => "wait_all",
            Self::Yield => "yield",
        }
    }

    /// Whether this op requires a `prompt` field.
    #[must_use]
    pub const fn requires_prompt(self) -> bool {
        matches!(self, Self::Agent | Self::Ask | Self::Think | Self::Yield)
    }

    /// Map to the canonical runtime operation type.
    #[must_use]
    pub const fn ais(self) -> AISOperationType {
        match self {
            Self::Agent => AISOperationType::Agent,
            Self::Ask => AISOperationType::Ask,
            Self::Think => AISOperationType::Think,
            Self::InvTool => AISOperationType::InvTool,
            Self::WaitAll => AISOperationType::WaitAll,
            Self::Yield => AISOperationType::Yield,
        }
    }
}

impl From<PlanNodeOp> for AISOperationType {
    fn from(op: PlanNodeOp) -> Self {
        op.ais()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_wire_tokens_round_trip() {
        for (op, token) in [
            (PlanNodeOp::Agent, "agent"),
            (PlanNodeOp::Ask, "ask"),
            (PlanNodeOp::Think, "think"),
            (PlanNodeOp::InvTool, "inv_tool"),
            (PlanNodeOp::WaitAll, "wait_all"),
            (PlanNodeOp::Yield, "yield"),
        ] {
            assert_eq!(op.as_str(), token);
            assert_eq!(serde_json::to_value(op).unwrap(), serde_json::json!(token));
            assert_eq!(
                serde_json::from_value::<PlanNodeOp>(serde_json::json!(token)).unwrap(),
                op
            );
        }
    }

    #[test]
    fn graph_round_trips_with_max_tokens() {
        let json = serde_json::json!({
            "name": "flow",
            "entry": "out",
            "nodes": [
                { "id": 1, "name": "greet", "op": "ask", "prompt": "hi", "max_tokens": 4096 },
                { "id": 2, "name": "out", "op": "yield", "prompt": "{greet}",
                  "depends_on": [{ "node": 1, "dependency": "Data" }] }
            ]
        });
        let g: PlanGraph = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(g.nodes[0].max_tokens, Some(4096));
        assert_eq!(g.nodes[1].depends_on[0].dependency, PlanDependencyKind::Data);
        // re-serialize and ensure it parses again (stable shape)
        let back = serde_json::to_value(&g).unwrap();
        let _g2: PlanGraph = serde_json::from_value(back).unwrap();
    }
}
