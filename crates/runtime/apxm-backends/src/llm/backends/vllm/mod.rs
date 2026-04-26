//! vLLM graph-aware backend module.
pub mod backend;
pub mod graph_meta;

pub use backend::{GraphAwareVllmBackend, GraphStatusResponse};
pub use graph_meta::{
    ApxmGraphHints, CompilerHints, GraphMetadata, LatencyClass, NodeGraphMetrics, NodeSpec,
    PinMode, PinPolicy, PriorityClass,
};
