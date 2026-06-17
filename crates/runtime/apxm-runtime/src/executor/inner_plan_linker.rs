//! Inner plan linker interface for runtime
//!
//! This module provides the interface for linking inner plan AIR payloads
//! during runtime execution. The linker acts as a bridge between the
//! runtime and the compiler, delegating parsing/validation to the compiler.

use apxm_core::{
    error::RuntimeError,
    types::execution::{ExecutionDag, TaskDag},
};
use async_trait::async_trait;

/// Result type for inner plan linking
pub type LinkResult = Result<ExecutionDag, RuntimeError>;

/// Trait for linking inner plan AIR payloads into ExecutionDAGs
///
/// The linker bridges the runtime and compiler:
/// - Runtime calls linker with AIR text
/// - Linker delegates to compiler for parsing/validation
/// - Linker returns validated DAG to runtime
#[async_trait]
pub trait InnerPlanLinker: Send + Sync {
    /// Link inner plan AIR payload into an ExecutionDAG.
    async fn link_inner_plan(&self, air_payload: &str, source_name: &str) -> LinkResult;

    /// Link a structured inner-plan task DAG into an ExecutionDAG.
    async fn link_task_dag(&self, dag: TaskDag) -> LinkResult;
}

/// No-op linker for contexts that don't support inner plan linking
pub struct NoOpLinker;

#[async_trait]
impl InnerPlanLinker for NoOpLinker {
    async fn link_inner_plan(&self, _graph_payload: &str, _source_name: &str) -> LinkResult {
        Err(RuntimeError::State(
            "Inner plan linking not supported in this context".to_string(),
        ))
    }

    async fn link_task_dag(&self, _dag: TaskDag) -> LinkResult {
        Err(RuntimeError::State(
            "Inner plan linking not supported in this context".to_string(),
        ))
    }
}
