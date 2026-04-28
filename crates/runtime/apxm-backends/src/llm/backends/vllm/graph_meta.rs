pub const BACKEND_NAME: &str = "vllm-graph-aware";
pub const REQUEST_XARGS: &str = "vllm_xargs";
pub const PROBE_GRAPH_ID: &str = "__apxm_probe__";

/// Scheduler policy that the APXM critical-path boost relies on.
///
/// Mirrors `SchedulerPolicy::PRIORITY` in `tools/scripts/apxm_vllm_contract.py`
/// and the `policy: SchedulerPolicy = "priority"` default in
/// `external/vllm/vllm/config/scheduler.py`.
pub const SCHEDULER_POLICY_PRIORITY: &str = "priority";

pub use apxm_core::types::{
    ApxmGraphHints, CompilerHints, GraphMetadata, LatencyClass, NodeGraphMetrics, NodeSpec,
    PinMode, PinPolicy, PriorityClass,
};
