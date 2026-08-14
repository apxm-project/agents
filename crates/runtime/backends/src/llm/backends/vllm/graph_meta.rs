pub const BACKEND_NAME: &str = "vllm-graph-aware";
pub const REQUEST_XARGS: &str =
    apxm_core::constants::llm::apxm::graph_hints::VLLM_REQUEST_XARGS;
pub const PROBE_GRAPH_ID: &str = "__apxm_probe__";

/// Scheduler policy value the APXM critical-path boost relies on; the
/// vLLM scheduler refuses priority hints under any other policy.
pub const SCHEDULER_POLICY_PRIORITY: &str = "priority";

pub use apxm_core::types::{
    ApxmGraphHints, GraphMetadata, LatencyClass, NodeGraphMetrics, NodeSpec,
};
