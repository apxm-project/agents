pub const BACKEND_NAME: &str = "vllm-graph-aware";
pub const PROBE_GRAPH_ID: &str = "__apxm_probe__";

/// vLLM-owned request mechanisms.
///
/// These names describe this provider's wire, not APXM semantics, so they live
/// in the adapter that implements them exactly like `attrs.rs` does for
/// vLLM-only node attributes. The common contract crate names none of them.
pub mod mechanisms {
    /// Provider extension container carrying the APXM envelope.
    pub const REQUEST_XARGS: &str = "vllm_xargs";
    /// Provider scheduler queue value.
    pub const REQUEST_PRIORITY: &str = "priority";
    /// Queue value the fork's critical-path boost consults.
    pub const PRIORITY_CRITICAL_PATH: u8 = 0;
    pub const PRIORITY_DEFAULT: u8 = 5;

    /// Closed mechanism identifiers this adapter cites in projection evidence.
    pub const APXM_XARGS: &str = "vllm.apxm_xargs";
    pub const REQUEST_PRIORITY_MECHANISM: &str = "vllm.request_priority";
    pub const PREFIX_PIN: &str = "vllm.prefix_pin";

    /// Adapter-owned resource observations. Their units are vLLM's, not APXM's.
    pub const OBSERVED_PINNED_HANDLES: &str = "vllm.pinned_handles";
    pub const OBSERVED_PINNED_BLOCKS: &str = "vllm.pinned_blocks";
}

pub use mechanisms::REQUEST_XARGS;

/// Scheduler policy value the APXM critical-path boost relies on; the
/// vLLM scheduler refuses priority hints under any other policy.
pub const SCHEDULER_POLICY_PRIORITY: &str = "priority";

pub use apxm_core::types::{
    ApxmGraphHints, GraphMetadata, LatencyClass, NodeGraphMetrics, NodeSpec,
};
