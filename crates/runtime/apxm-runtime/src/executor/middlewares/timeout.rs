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

