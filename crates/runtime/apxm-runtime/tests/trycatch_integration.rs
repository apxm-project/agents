//! Integration test for TRY_CATCH real semantics.
//!
//! Tests the full pipeline: a TRY_CATCH node that references try/catch
//! sub-DAGs in the flow registry, executes the try-branch, and on failure
//! falls through to the catch-branch for graceful recovery.

use apxm_backends::LLMRegistry;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::execution::{ExecutionDag, NodeMetadata};
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_runtime::capability::CapabilitySystem;
use apxm_runtime::capability::flow_registry::FlowRegistry;
use apxm_runtime::executor::{ExecutionContext, ExecutorEngine};
use apxm_runtime::memory::{MemoryConfig, MemorySystem};
use std::collections::HashMap;
use std::sync::Arc;

/// A sub-DAG that returns a constant string.
fn success_dag(value: &str) -> ExecutionDag {
    let mut node = apxm_core::types::execution::Node {
        id: 1,
        op_type: AISOperationType::ConstStr,
        attributes: HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![100],
        metadata: NodeMetadata::default(),
    };
    node.attributes
        .insert("value".to_string(), Value::String(value.to_string()));

    ExecutionDag {
        nodes: vec![node],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![1],
        metadata: Default::default(),
    }
}

/// A sub-DAG that always fails (CONST_STR missing required "value" attr).
fn failing_dag() -> ExecutionDag {
    let node = apxm_core::types::execution::Node {
        id: 1,
        op_type: AISOperationType::ConstStr,
        attributes: HashMap::new(), // missing required "value" attribute
        input_tokens: vec![],
        output_tokens: vec![100],
        metadata: NodeMetadata::default(),
    };

    ExecutionDag {
        nodes: vec![node],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![1],
        metadata: Default::default(),
    }
}

/// Build a full DAG containing a single TRY_CATCH node.
fn trycatch_dag(try_label: &str, catch_label: &str) -> ExecutionDag {
    let mut node = apxm_core::types::execution::Node {
        id: 1,
        op_type: AISOperationType::TryCatch,
        attributes: HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![200],
        metadata: NodeMetadata::default(),
    };
    node.attributes.insert(
        graph_attrs::TRY_LABEL.to_string(),
        Value::String(try_label.to_string()),
    );
    node.attributes.insert(
        graph_attrs::CATCH_LABEL.to_string(),
        Value::String(catch_label.to_string()),
    );

    ExecutionDag {
        nodes: vec![node],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![1],
        metadata: Default::default(),
    }
}

async fn make_engine(registry: Arc<FlowRegistry>) -> ExecutorEngine {
    let memory = Arc::new(
        MemorySystem::new(MemoryConfig::in_memory_ltm())
            .await
            .unwrap(),
    );
    let llm_registry = Arc::new(LLMRegistry::new());
    let capability_system = Arc::new(CapabilitySystem::new());
    let ctx = ExecutionContext::new(
        memory,
        llm_registry,
        capability_system,
        apxm_runtime::aam::Aam::new(),
    );
    let ctx = ExecutionContext {
        flow_registry: registry,
        ..ctx
    };
    ExecutorEngine::new(ctx)
}

/// Happy path: try succeeds, catch is never executed.
#[tokio::test]
async fn trycatch_happy_path_try_succeeds() {
    let registry = Arc::new(FlowRegistry::new());
    registry.register_flow("App", "try_flow", success_dag("try succeeded"));
    registry.register_flow("App", "catch_flow", success_dag("should not see this"));

    let engine = make_engine(registry).await;
    let dag = trycatch_dag("try_flow", "catch_flow");
    let result = engine.execute_dag(dag).await.unwrap();

    let output = result.results.values().next().unwrap();
    assert_eq!(output, &Value::String("try succeeded".to_string()));
}

/// Error path: try fails, catch runs and recovers.
#[tokio::test]
async fn trycatch_error_path_catch_recovers() {
    let registry = Arc::new(FlowRegistry::new());
    registry.register_flow("App", "try_flow", failing_dag());
    registry.register_flow("App", "catch_flow", success_dag("gracefully recovered"));

    let engine = make_engine(registry).await;
    let dag = trycatch_dag("try_flow", "catch_flow");
    let result = engine.execute_dag(dag).await.unwrap();

    let output = result.results.values().next().unwrap();
    assert_eq!(output, &Value::String("gracefully recovered".to_string()));
}

/// Error in catch: both try and catch fail, error propagates.
#[tokio::test]
async fn trycatch_error_in_catch_propagates() {
    let registry = Arc::new(FlowRegistry::new());
    registry.register_flow("App", "try_flow", failing_dag());
    registry.register_flow("App", "catch_flow", failing_dag());

    let engine = make_engine(registry).await;
    let dag = trycatch_dag("try_flow", "catch_flow");
    let result = engine.execute_dag(dag).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Missing required attribute"),
        "Expected error from catch branch, got: {}",
        err
    );
}

/// Missing try_label: the "not found" error from the try-branch is caught
/// by the catch-branch, which recovers gracefully.
#[tokio::test]
async fn trycatch_missing_try_label_caught_by_catch() {
    let registry = Arc::new(FlowRegistry::new());
    // Only register catch, not try — the try-branch error gets caught
    registry.register_flow(
        "App",
        "catch_flow",
        success_dag("recovered from missing try"),
    );

    let engine = make_engine(registry).await;
    let dag = trycatch_dag("nonexistent_try", "catch_flow");
    let result = engine.execute_dag(dag).await.unwrap();

    let output = result.results.values().next().unwrap();
    assert_eq!(
        output,
        &Value::String("recovered from missing try".to_string())
    );
}

/// Missing both try and catch labels results in a propagated error.
#[tokio::test]
async fn trycatch_missing_both_labels_errors() {
    let registry = Arc::new(FlowRegistry::new());
    // Neither try nor catch registered

    let engine = make_engine(registry).await;
    let dag = trycatch_dag("nonexistent_try", "nonexistent_catch");
    let result = engine.execute_dag(dag).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found"),
        "Expected 'not found' error, got: {}",
        err
    );
}
