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
