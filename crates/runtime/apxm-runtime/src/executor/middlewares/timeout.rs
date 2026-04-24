use crate::executor::{ExecutionContext, Next, OperationMiddleware, Result};
use apxm_core::{
    constants::graph::attrs as graph_attrs,
    error::RuntimeError,
    types::{execution::Node, values::Value},
};
use async_trait::async_trait;
use std::time::Duration;

/// Enforces a per-node timeout around the remaining middleware chain.
#[derive(Debug, Clone)]
pub struct TimeoutMiddleware {
    default_timeout: Option<Duration>,
}

impl TimeoutMiddleware {
    pub fn new(default_timeout: Option<Duration>) -> Self {
        Self { default_timeout }
    }

    fn resolve_timeout(&self, node: &Node) -> Option<Duration> {
        node.attributes
            .get(graph_attrs::TIMEOUT_MS)
            .and_then(|value| value.as_u64())
            .map(Duration::from_millis)
            .or(self.default_timeout)
    }
}

#[async_trait]
impl OperationMiddleware for TimeoutMiddleware {
    fn name(&self) -> &str {
        "timeout"
    }

    async fn around(
        &self,
        ctx: &ExecutionContext,
        node: &Node,
        inputs: Vec<Value>,
        next: Next<'_>,
    ) -> Result<Value> {
        let Some(timeout) = self.resolve_timeout(node) else {
            return next.run(ctx, node, inputs).await;
        };

        match tokio::time::timeout(timeout, next.run(ctx, node, inputs)).await {
            Ok(result) => result,
            Err(_) => Err(RuntimeError::Timeout {
                op_id: node.id,
                timeout,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        aam::Aam,
        capability::CapabilitySystem,
        executor::OperationDispatcher,
        memory::{MemoryConfig, MemorySystem},
    };
    use apxm_backends::LLMRegistry;
    use apxm_core::types::{execution::Node, operations::AISOperationType, values::Number};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct DelayMiddleware {
        delay: Duration,
    }

    #[async_trait]
    impl OperationMiddleware for DelayMiddleware {
        fn name(&self) -> &str {
            "delay"
        }

        async fn around(
            &self,
            ctx: &ExecutionContext,
            node: &Node,
            inputs: Vec<Value>,
            next: Next<'_>,
        ) -> Result<Value> {
            tokio::time::sleep(self.delay).await;
            next.run(ctx, node, inputs).await
        }
    }

    async fn test_context() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
    }

    #[tokio::test]
    async fn timeout_middleware_aborts_slow_inner_chain() {
        let ctx = test_context().await.with_middlewares(vec![
            Arc::new(TimeoutMiddleware::new(Some(Duration::from_millis(10)))),
            Arc::new(DelayMiddleware {
                delay: Duration::from_millis(50),
            }),
        ]);
        let node = Node::new(11, AISOperationType::Nop);

        let error = OperationDispatcher::dispatch(&ctx, &node, vec![])
            .await
            .unwrap_err();

        assert!(matches!(error, RuntimeError::Timeout { op_id: 11, .. }));
    }

    #[tokio::test]
    async fn timeout_middleware_reads_node_timeout_ms_attribute() {
        let ctx = test_context().await.with_middlewares(vec![
            Arc::new(TimeoutMiddleware::new(None)),
            Arc::new(DelayMiddleware {
                delay: Duration::from_millis(50),
            }),
        ]);
        let mut node = Node::new(12, AISOperationType::Nop);
        node.attributes.insert(
            graph_attrs::TIMEOUT_MS.to_string(),
            Value::Number(Number::Integer(5)),
        );

        let error = OperationDispatcher::dispatch(&ctx, &node, vec![])
            .await
            .unwrap_err();

        assert!(matches!(error, RuntimeError::Timeout { op_id: 12, .. }));
    }
}
