//! Typed shape for `DispatchIrV1`. See `.apxm/docs/design/dispatch-ir.md`.
//!
//! These structs are the in-memory form of the compiled-graph intent that the
//! APXM runtime hands to a graph-aware inference backend. The serde shape is
//! documented in the design note as a sketch, not a committed wire ABI.

use apxm_core::types::graph_hints::{CompilerHints, PinPolicy, PriorityClass};
use apxm_core::types::graph_metrics::LatencyClass;
use serde::{Deserialize, Serialize};

pub(crate) const DISPATCH_IR_V1_SCHEMA_VERSION: &str = "dispatch-ir-v1";

/// Top-level Dispatch IR envelope at version 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DispatchIrV1 {
    pub schema_version: String,
    pub graph: GraphDispatchPlan,
    pub requirements: BackendCapabilityRequirements,
    pub nodes: Vec<NodeDispatchPlan>,
    pub telemetry: TelemetryContract,
}

impl DispatchIrV1 {
    pub fn new(
        graph: GraphDispatchPlan,
        requirements: BackendCapabilityRequirements,
        nodes: Vec<NodeDispatchPlan>,
        telemetry: TelemetryContract,
    ) -> Self {
        Self {
            schema_version: DISPATCH_IR_V1_SCHEMA_VERSION.to_owned(),
            graph,
            requirements,
            nodes,
            telemetry,
        }
    }
}

/// Graph-level identity and shape carried into the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GraphDispatchPlan {
    pub graph_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_path_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_pin_ttl_ms: Option<u32>,
}

/// Per-node intent: identity, routing, prefix cohort, pin policy, latency class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeDispatchPlan {
    pub node_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_class: Option<PriorityClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_class: Option<LatencyClass>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downstream_nodes: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_cohort: Option<String>,
    pub pin_policy: PinPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fanout_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_path_len: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_dynamic_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_group: Option<String>,
    #[serde(default, skip_serializing_if = "CompilerHints::is_empty")]
    pub compiler_hints: CompilerHints,
    #[serde(default, skip)]
    pub registration_node_name: Option<String>,
    #[serde(default, skip)]
    pub registration_estimated_prompt_tokens: Option<u32>,
    #[serde(default, skip)]
    pub registration_is_critical_path: Option<bool>,
}

/// What the backend must (or may) support before APXM relies on graph-aware
/// behaviour. `required` capabilities not honored should make APXM fall back to
/// a generic protocol; `optional` capabilities are best-effort.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct BackendCapabilityRequirements {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional: Vec<String>,
}

/// Labels and metrics APXM expects the backend to attribute back to the graph.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct TelemetryContract {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_metrics: Vec<String>,
}
