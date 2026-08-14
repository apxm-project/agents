//! vLLM graph-aware backend module.
pub mod attrs;
pub mod backend;
pub mod graph_meta;

pub use backend::{GraphAwareVllmBackend, GraphStatusResponse};
pub use graph_meta::{
    ApxmGraphHints, GraphMetadata, LatencyClass, NodeGraphMetrics, NodeSpec, PROBE_GRAPH_ID,
    REQUEST_XARGS,
};
