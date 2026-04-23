//! TRYCATCH operation — Structured exception handling with recovery subgraph
//!
//! Executes the try-branch sub-DAG. On success, returns the try-branch output.
//! On failure, converts the error to a JSON value and passes it as input
//! to the catch-branch sub-DAG, returning the catch-branch output.

use std::future::Future;
use std::pin::Pin;

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use crate::executor::ExecutorEngine;
use apxm_core::constants::graph::attrs as graph_attrs;

/// Execute a TRY_CATCH operation.
///
/// Returns a boxed future to break the async recursion chain (try_catch ->
/// execute_dag -> dispatch -> try_catch).
pub fn execute<'a>(
    ctx: &'a ExecutionContext,
    node: &'a Node,
    inputs: Vec<Value>,
) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
    Box::pin(execute_impl(ctx, node, inputs))
}

async fn execute_impl(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let try_label = get_string_attribute(node, graph_attrs::TRY_LABEL)?;
    let catch_label = get_string_attribute(node, graph_attrs::CATCH_LABEL)?;

    tracing::info!(
        node_id = node.id,
        try_label = %try_label,
        catch_label = %catch_label,
        "Executing TRY_CATCH"
    );

    // Execute try-branch
    let engine = ExecutorEngine::new(ctx.child());
    let try_result = engine.run_subgraph_by_label(&try_label, inputs).await;

    match try_result {
        Ok(value) => {
            tracing::debug!(
                node_id = node.id,
                "TRY_CATCH: try-branch succeeded, skipping catch"
            );
            Ok(value)
        }
        Err(try_err) => {
            tracing::warn!(
                node_id = node.id,
                error = %try_err,
                "TRY_CATCH: try-branch failed, executing catch-branch"
            );

            // Convert the error to a JSON value for the catch-branch input
            let error_value = Value::try_from(try_err.to_value())
                .unwrap_or_else(|_| Value::String(format!("Error conversion failed: {}", try_err)));

            // Execute catch-branch with the error as input
            let catch_engine = ExecutorEngine::new(ctx.child());
            let catch_result = catch_engine
                .run_subgraph_by_label(&catch_label, vec![error_value])
                .await;

            match catch_result {
                Ok(value) => {
                    tracing::info!(
                        node_id = node.id,
                        "TRY_CATCH: catch-branch recovered successfully"
                    );
                    Ok(value)
                }
                Err(catch_err) => {
                    tracing::error!(
                        node_id = node.id,
                        error = %catch_err,
                        "TRY_CATCH: catch-branch also failed, propagating error"
                    );
                    Err(catch_err)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::capability::flow_registry::FlowRegistry;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::execution::{ExecutionDag, NodeMetadata};
    use apxm_core::types::operations::AISOperationType;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Create a sub-DAG that returns a constant string.
    fn make_success_dag(value: &str) -> ExecutionDag {
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

    /// Create a sub-DAG that always fails with a RuntimeError.
    ///
    /// Uses a CONST_STR node missing its required "value" attribute,
    /// which triggers `RuntimeError::Operation { message: "Missing required attribute: value" }`.
    fn make_failing_dag() -> ExecutionDag {
        let node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(), // missing required "value" attr
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

    fn make_try_catch_node(
        try_label: &str,
        catch_label: &str,
    ) -> apxm_core::types::execution::Node {
        let mut node = apxm_core::types::execution::Node {
            id: 10,
            op_type: AISOperationType::TryCatch,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
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
        node
    }

    async fn make_ctx_with_registry(registry: Arc<FlowRegistry>) -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        ExecutionContext {
            flow_registry: registry,
            ..ctx
        }
    }

    #[tokio::test]
    async fn test_try_catch_happy_path_try_succeeds_catch_skipped() {
        let registry = Arc::new(FlowRegistry::new());
        registry.register_flow("Agent", "try_block", make_success_dag("all good"));
        registry.register_flow("Agent", "catch_block", make_success_dag("recovered"));

        let ctx = make_ctx_with_registry(registry).await;
        let node = make_try_catch_node("try_block", "catch_block");

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("all good".to_string()));
    }

    #[tokio::test]
    async fn test_try_catch_error_path_catch_runs_and_recovers() {
        let registry = Arc::new(FlowRegistry::new());
        registry.register_flow("Agent", "try_block", make_failing_dag());
        registry.register_flow("Agent", "catch_block", make_success_dag("recovered"));

        let ctx = make_ctx_with_registry(registry).await;
        let node = make_try_catch_node("try_block", "catch_block");

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("recovered".to_string()));
    }

    #[tokio::test]
    async fn test_try_catch_error_in_catch_propagates() {
        let registry = Arc::new(FlowRegistry::new());
        registry.register_flow("Agent", "try_block", make_failing_dag());
        registry.register_flow("Agent", "catch_block", make_failing_dag());

        let ctx = make_ctx_with_registry(registry).await;
        let node = make_try_catch_node("try_block", "catch_block");

        let result = execute(&ctx, &node, vec![]).await;
        assert!(
            result.is_err(),
            "Expected error to propagate from catch-branch"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Missing required attribute"),
            "Expected catch-branch error to propagate, got: {}",
            err_msg
        );
    }
}
