//! vLLM graph-aware backend module.
pub mod backend;
pub mod graph_meta;

pub use backend::{GraphAwareVllmBackend, GraphStatusResponse};
pub use graph_meta::{
    ApxmGraphHints, CompilerHints, GraphMetadata, NodeSpec, PinMode, PinPolicy, PriorityClass,
};
