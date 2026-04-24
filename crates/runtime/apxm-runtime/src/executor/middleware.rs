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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        aam::Aam,
        capability::CapabilitySystem,
        executor::{ExecutionContext, OperationDispatcher},
        memory::{MemoryConfig, MemorySystem},
    };
    use apxm_backends::LLMRegistry;
    use apxm_core::{
        error::RuntimeError,
        types::{execution::Node, operations::AISOperationType, values::Value},
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

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

    fn nop_node(node_id: u64) -> Node {
        Node::new(node_id, AISOperationType::Nop)
    }

    struct RecordingMiddleware {
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl OperationMiddleware for RecordingMiddleware {
        fn name(&self) -> &str {
            self.name
        }

        async fn around(
            &self,
            ctx: &ExecutionContext,
            node: &Node,
            inputs: Vec<Value>,
            next: Next<'_>,
        ) -> Result<Value> {
            self.events
                .lock()
                .unwrap()
                .push(format!("{}:before", self.name));
            let result = next.run(ctx, node, inputs).await;
            self.events
                .lock()
                .unwrap()
                .push(format!("{}:after", self.name));
            result
        }
    }

    struct ShortCircuitMiddleware;

    #[async_trait]
    impl OperationMiddleware for ShortCircuitMiddleware {
        fn name(&self) -> &str {
            "short-circuit"
        }

        async fn around(
            &self,
            _ctx: &ExecutionContext,
            _node: &Node,
            _inputs: Vec<Value>,
            _next: Next<'_>,
        ) -> Result<Value> {
            Ok(Value::String("short-circuit".to_string()))
        }
    }

    struct CountingMiddleware {
        hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl OperationMiddleware for CountingMiddleware {
        fn name(&self) -> &str {
            "counting"
        }

        async fn around(
            &self,
            ctx: &ExecutionContext,
            node: &Node,
            inputs: Vec<Value>,
            next: Next<'_>,
        ) -> Result<Value> {
            self.hits.fetch_add(1, Ordering::Relaxed);
            next.run(ctx, node, inputs).await
        }
    }

    struct ErrorMiddleware;

    #[async_trait]
    impl OperationMiddleware for ErrorMiddleware {
        fn name(&self) -> &str {
            "error"
        }

        async fn around(
            &self,
            _ctx: &ExecutionContext,
            _node: &Node,
            _inputs: Vec<Value>,
            _next: Next<'_>,
        ) -> Result<Value> {
            Err(RuntimeError::Executor("middleware failed".to_string()))
        }
    }

    #[tokio::test]
    async fn middleware_chain_runs_in_registration_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let ctx = test_context().await.with_middlewares(vec![
            Arc::new(RecordingMiddleware {
                name: "outer",
                events: Arc::clone(&events),
            }),
            Arc::new(RecordingMiddleware {
                name: "inner",
                events: Arc::clone(&events),
            }),
        ]);

        let node = nop_node(1);
        let value =
            OperationDispatcher::dispatch(&ctx, &node, vec![Value::String("ok".to_string())])
                .await
                .unwrap();

        assert_eq!(value, Value::String("ok".to_string()));
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &["outer:before", "inner:before", "inner:after", "outer:after"]
        );
    }

    #[tokio::test]
    async fn middleware_short_circuit_skips_dispatch_inner() {
        let ctx = test_context()
            .await
            .with_middleware(Arc::new(ShortCircuitMiddleware));
        let node = nop_node(1);

        let value =
            OperationDispatcher::dispatch(&ctx, &node, vec![Value::String("ignored".to_string())])
                .await
                .unwrap();

        assert_eq!(value, Value::String("short-circuit".to_string()));
    }

    #[tokio::test]
    async fn middleware_errors_propagate() {
        let ctx = test_context()
            .await
            .with_middleware(Arc::new(ErrorMiddleware));
        let node = nop_node(1);

        let error = OperationDispatcher::dispatch(&ctx, &node, vec![])
            .await
            .unwrap_err();

        assert!(matches!(error, RuntimeError::Executor(_)));
    }

    #[tokio::test]
    async fn child_context_inherits_middlewares() {
        let hits = Arc::new(AtomicUsize::new(0));
        let parent = test_context()
            .await
            .with_middleware(Arc::new(CountingMiddleware {
                hits: Arc::clone(&hits),
            }));
        let child = parent.child();
        let node = nop_node(7);

        OperationDispatcher::dispatch(&child, &node, vec![])
            .await
            .unwrap();

        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }
}
