//! QMEM operation - Query memory

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute, get_string_attribute,
};
use crate::{
    aam::{STAGED_BELIEF_PREFIX, TransitionLabel},
    memory::MemorySpace,
};
use apxm_core::constants::graph::attrs as graph_attrs;

/// Execute QMEM operation - Query memory from specified tier
pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let query = get_string_attribute(node, graph_attrs::QUERY)?;
    let memory_tier = get_optional_string_attribute(node, graph_attrs::MEMORY_TIER)?;
    let limit = node
        .attributes
        .get(graph_attrs::LIMIT)
        .and_then(|v| v.as_u64())
        .unwrap_or(apxm_core::constants::defaults::DEFAULT_MEMORY_LIMIT) as usize;

    // Determine memory space
    let space = match memory_tier.as_deref() {
        Some(tier) => tier.parse::<MemorySpace>()?,
        None => MemorySpace::Stm, // Default to STM
    };

    // Search memory
    let results = ctx
        .memory
        .search_scoped(space, ctx.scope_id(), &query, limit)
        .await?;

    // Emit memory-read event
    if let Some(emitter) = &ctx.event_emitter {
        let scope = memory_tier.as_deref().unwrap_or("stm");
        emitter.emit_memory_read(scope, &query);
    }

    // Convert search results to Value::Array
    let values: Vec<Value> = results
        .into_iter()
        .map(|r| {
            // Create object with key and value
            let mut obj = std::collections::HashMap::new();
            obj.insert("key".to_string(), Value::String(r.key));
            obj.insert("value".to_string(), r.value);
            obj.insert(
                "score".to_string(),
                Value::Number(apxm_core::types::values::Number::Float(r.score)),
            );
            Value::Object(obj)
        })
        .collect();

    let array_value = Value::Array(values.clone());

    // Stage results as a belief so downstream ops can refer to them.
    let staging_id = node
        .attributes
        .get(graph_attrs::STAGING_ID)
        .and_then(|v| v.as_string().map(|s| s.to_string()))
        .unwrap_or_else(|| format!("{}:{}", ctx.execution_id, node.id));
    let stage_key = format!("{}{}", STAGED_BELIEF_PREFIX, staging_id);
    ctx.aam.set_belief(
        stage_key,
        array_value.clone(),
        TransitionLabel::operation(node.id, format!("{:?}", node.op_type)),
    );

    Ok(array_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::executor::events::{ExecutionEvent, ExecutionEventEmitter};
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use std::sync::{Arc, Mutex};

    /// Test event emitter that captures events for assertions.
    struct TestEventEmitter {
        events: Arc<Mutex<Vec<ExecutionEvent>>>,
    }

    impl TestEventEmitter {
        fn new() -> (Self, Arc<Mutex<Vec<ExecutionEvent>>>) {
            let events = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    events: events.clone(),
                },
                events,
            )
        }
    }

    impl ExecutionEventEmitter for TestEventEmitter {
        fn emit_llm_token(&self, _content: &str) {}
        fn emit_tool_start(&self, _name: &str, _args: &std::collections::HashMap<String, Value>) {}
        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_memory_read(&self, scope: &str, key: &str) {
            self.events
                .lock()
                .unwrap()
                .push(ExecutionEvent::MemoryRead {
                    scope: scope.to_string(),
                    key: key.to_string(),
                });
        }

        fn emit_memory_write(&self, scope: &str, key: &str) {
            self.events
                .lock()
                .unwrap()
                .push(ExecutionEvent::MemoryWrite {
                    scope: scope.to_string(),
                    key: key.to_string(),
                });
        }
    }

    #[tokio::test]
    async fn test_qmem_stm() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory.clone(),
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        // Populate STM
        memory
            .write_scoped(
                MemorySpace::Stm,
                ctx.scope_id(),
                "user:1".to_string(),
                Value::String("alice".to_string()),
            )
            .await
            .unwrap();
        memory
            .write_scoped(
                MemorySpace::Stm,
                ctx.scope_id(),
                "user:2".to_string(),
                Value::String("bob".to_string()),
            )
            .await
            .unwrap();

        // Create QMEM node
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::QMem,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::QUERY.to_string(),
            Value::String("user".to_string()),
        );
        node.attributes.insert(
            graph_attrs::MEMORY_TIER.to_string(),
            Value::String("stm".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await.unwrap();

        // Should return array of results
        if let Value::Array(arr) = result {
            assert_eq!(arr.len(), 2);
        } else {
            panic!("Expected array result");
        }
    }

    #[tokio::test]
    async fn test_qmem_emits_memory_read_event() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let (emitter, captured_events) = TestEventEmitter::new();
        let ctx = ExecutionContext::new(
            memory.clone(),
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_event_emitter(Some(Arc::new(emitter)));

        // Populate STM
        memory
            .write_scoped(
                MemorySpace::Stm,
                ctx.scope_id(),
                "doc:1".to_string(),
                Value::String("hello".to_string()),
            )
            .await
            .unwrap();

        // Create QMEM node
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::QMem,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::QUERY.to_string(),
            Value::String("doc".to_string()),
        );
        node.attributes.insert(
            graph_attrs::MEMORY_TIER.to_string(),
            Value::String("stm".to_string()),
        );

        let _result = execute(&ctx, &node, vec![]).await.unwrap();

        let events = captured_events.lock().unwrap();
        assert_eq!(events.len(), 1, "expected exactly one event");
        match &events[0] {
            ExecutionEvent::MemoryRead { scope, key } => {
                assert_eq!(scope, "stm");
                assert_eq!(key, "doc");
            }
            other => panic!("expected MemoryRead event, got {:?}", other),
        }
    }
}
