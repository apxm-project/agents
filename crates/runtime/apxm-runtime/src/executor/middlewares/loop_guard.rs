use crate::executor::{ExecutionContext, Next, OperationMiddleware, Result};
use apxm_core::{
    error::RuntimeError,
    types::{execution::Node, values::Value},
};
use async_trait::async_trait;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LoopKey {
    node_id: u64,
    input_fingerprint: String,
}

/// Conservative loop guard that rejects repeated identical node/input pairs.
#[derive(Clone, Default)]
pub struct LoopGuardMiddleware {
    max_repeats: usize,
    seen: Arc<Mutex<HashMap<LoopKey, usize>>>,
}

impl LoopGuardMiddleware {
    pub fn new(max_repeats: usize) -> Self {
        Self {
            max_repeats: max_repeats.max(1),
            seen: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn fingerprint(inputs: &[Value]) -> String {
        serde_json::to_string(inputs).unwrap_or_else(|_| format!("{inputs:?}"))
    }
}

#[async_trait]
impl OperationMiddleware for LoopGuardMiddleware {
    fn name(&self) -> &str {
        "loop_guard"
    }

    async fn around(
        &self,
        ctx: &ExecutionContext,
        node: &Node,
        inputs: Vec<Value>,
        next: Next<'_>,
    ) -> Result<Value> {
        let key = LoopKey {
            node_id: node.id,
            input_fingerprint: Self::fingerprint(&inputs),
        };

        {
            let mut seen = self.seen.lock().unwrap();
            let count = seen.entry(key).or_insert(0);
            *count += 1;
            if *count > self.max_repeats {
                return Err(RuntimeError::State(format!(
                    "loop guard tripped for node {} after {} repeated executions",
                    node.id, count
                )));
            }
        }

        next.run(ctx, node, inputs).await
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
    use apxm_core::types::{execution::Node, operations::AISOperationType, values::Value};

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
    async fn loop_guard_rejects_repeated_identical_node_inputs() {
        let ctx = test_context()
            .await
            .with_middleware(Arc::new(LoopGuardMiddleware::new(1)));
        let node = Node::new(21, AISOperationType::Nop);
        let inputs = vec![Value::String("same".to_string())];

        let first = OperationDispatcher::dispatch(&ctx, &node, inputs.clone())
            .await
            .unwrap();
        let second = OperationDispatcher::dispatch(&ctx, &node, inputs)
            .await
            .unwrap_err();

        assert_eq!(first, Value::String("same".to_string()));
        assert!(matches!(second, RuntimeError::State(_)));
        assert!(second.to_string().contains("loop guard"));
    }
}
