pub const BACKEND_NAME: &str = "vllm-graph-aware";
pub const REQUEST_XARGS: &str = "vllm_xargs";
pub const PROBE_GRAPH_ID: &str = "__apxm_probe__";

pub use apxm_core::types::{
    ApxmGraphHints, CompilerHints, GraphMetadata, LatencyClass, NodeGraphMetrics, NodeSpec,
    PinMode, PinPolicy, PriorityClass,
};
