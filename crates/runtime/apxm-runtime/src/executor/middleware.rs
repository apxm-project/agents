//! Dispatcher-level operation middleware.
//!
//! Middleware wraps node execution around the dispatcher chokepoint. Each
//! layer can observe, mutate, short-circuit, or reject execution before the
//! concrete handler runs.

use super::{Result, context::ExecutionContext};
use apxm_core::types::{execution::Node, values::Value};
use async_trait::async_trait;
use std::{future::Future, pin::Pin, sync::Arc};

/// Boxed future used by the dispatcher and middleware chain.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) type DispatchFn =
    for<'a> fn(&'a ExecutionContext, &'a Node, Vec<Value>) -> BoxFuture<'a, Result<Value>>;

/// Handle passed to each middleware to continue execution.
pub struct Next<'a> {
    pub(crate) chain: &'a [Arc<dyn OperationMiddleware>],
    pub(crate) idx: usize,
    pub(crate) terminal: DispatchFn,
}

impl<'a> Next<'a> {
    /// Run the next middleware in the chain or fall through to the dispatcher.
    pub fn run(
        self,
        ctx: &'a ExecutionContext,
        node: &'a Node,
        inputs: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value>> {
        Box::pin(async move {
            if let Some(middleware) = self.chain.get(self.idx) {
                let next = Next {
                    chain: self.chain,
                    idx: self.idx + 1,
                    terminal: self.terminal,
                };
                middleware.around(ctx, node, inputs, next).await
            } else {
                (self.terminal)(ctx, node, inputs).await
            }
        })
    }
}

/// Middleware that wraps operation execution at the dispatcher level.
#[async_trait]
pub trait OperationMiddleware: Send + Sync {
    /// Stable middleware name for diagnostics.
    fn name(&self) -> &str {
        "operation-middleware"
    }

    /// Cheap predicate so callers can skip middleware on irrelevant nodes.
    fn applies_to(&self, _node: &Node) -> bool {
        true
    }

    /// Wrap execution, optionally short-circuiting before `next.run(...)`.
    async fn around(
        &self,
        ctx: &ExecutionContext,
        node: &Node,
        inputs: Vec<Value>,
        next: Next<'_>,
    ) -> Result<Value>;
}

