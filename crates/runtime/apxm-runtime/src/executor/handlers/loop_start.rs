//! LOOP_START operation - Loop initialization

use super::{ExecutionContext, Node, Result, Value};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    // Initialize loop counter
    let max_iterations = node
        .attributes
        .get(graph_attrs::MAX_ITERATIONS)
        .and_then(|v| v.as_u64())
        .unwrap_or(100);

    // Record loop initialization in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::LOOP_START_PREFIX,
            ctx.execution_id,
            node.id
        ),
        Value::Number(apxm_core::types::values::Number::Integer(
            max_iterations as i64,
        )),
        label,
    );

    Ok(Value::Number(apxm_core::types::values::Number::Integer(
        max_iterations as i64,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::values::Number;
    use std::sync::Arc;

    async fn test_ctx() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        ExecutionContext::new(
            memory,
            Arc::new(apxm_backends::LLMRegistry::new()),
            Arc::new(CapabilitySystem::new()),
            crate::aam::Aam::new(),
        )
    }

    fn loop_start_node(max_iter: Option<u64>) -> Node {
        let mut node = Node::new(1, AISOperationType::LoopStart);
        if let Some(n) = max_iter {
            node.attributes.insert(
                graph_attrs::MAX_ITERATIONS.to_string(),
                Value::Number(Number::Integer(n as i64)),
            );
        }
        node
    }

    #[tokio::test]
    async fn test_loop_start_sets_counter() {
        let ctx = test_ctx().await;
        let node = loop_start_node(Some(5));
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::Number(Number::Integer(5)));
    }

    #[tokio::test]
    async fn test_loop_start_default_limit() {
        let ctx = test_ctx().await;
        let node = loop_start_node(None);
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::Number(Number::Integer(100)));
    }

    #[tokio::test]
    async fn test_loop_start_ignores_inputs() {
        let ctx = test_ctx().await;
        let node = loop_start_node(Some(10));
        let result = execute(&ctx, &node, vec![Value::String("ignored".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::Number(Number::Integer(10)));
    }
}
