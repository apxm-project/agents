//! Integration test: span hierarchy — verify that parent_span_id chains are
//! correctly maintained across node execution in a DAG.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use apxm_core::events::kind;
use apxm_core::events::payload::{OperationEndPayload, OperationStartPayload};
use apxm_core::events::{ApxmEvent, EventEmitter, EventSource};
use apxm_core::types::execution::{ExecutionDag, Node, NodeMetadata};
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_runtime::capability::CapabilitySystem;
use apxm_runtime::executor::{EmitterAdapter, ExecutionContext, ExecutorEngine};
use apxm_runtime::memory::{MemoryConfig, MemorySystem};

/// Event emitter sink that collects all events for inspection.
struct CollectingEmitter {
    events: Mutex<Vec<ApxmEvent>>,
}

impl CollectingEmitter {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl EventEmitter for CollectingEmitter {
    fn emit(&self, event: ApxmEvent) {
        self.events.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn span_hierarchy_two_node_chain() {
    let memory = Arc::new(
        MemorySystem::new(MemoryConfig::in_memory_ltm())
            .await
            .unwrap(),
    );
    let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
    let capability_system = Arc::new(CapabilitySystem::new());

    let collecting = Arc::new(CollectingEmitter::new());
    let adapter = Arc::new(EmitterAdapter::new(
        collecting.clone(),
        EventSource::Runtime,
        "test-trace",
    ));

    let ctx = ExecutionContext::new(
        memory,
        llm_registry,
        capability_system,
        apxm_runtime::aam::Aam::new(),
    )
    .with_event_emitter(Some(adapter));

    // Build a two-node DAG: CONST_STR(1) and CONST_STR(2), both independent.
    // They execute sequentially in the sequential fallback path.
    let mut node1 = Node {
        id: 1,
        op_type: AISOperationType::ConstStr,
        attributes: HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![100],
        metadata: NodeMetadata::default(),
    };
    node1
        .attributes
        .insert("value".to_string(), Value::String("hello".to_string()));

    let mut node2 = Node {
        id: 2,
        op_type: AISOperationType::ConstStr,
        attributes: HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![101],
        metadata: NodeMetadata::default(),
    };
    node2
        .attributes
        .insert("value".to_string(), Value::String("world".to_string()));

    let dag = ExecutionDag {
        nodes: vec![node1, node2],
        edges: vec![],
        entry_nodes: vec![1, 2],
        exit_nodes: vec![1, 2],
        metadata: Default::default(),
    };

    let engine = ExecutorEngine::new(ctx);
    let result = engine.execute_dag(dag).await.unwrap();
    assert_eq!(result.stats.executed_nodes, 2);
    assert_eq!(result.stats.failed_nodes, 0);

    // Inspect emitted events.
    let events = collecting.events.lock().unwrap();

    // We expect OperationStart/OperationEnd pairs for each node.
    // Collect span info from operation events.
    let op_events: Vec<_> = events
        .iter()
        .filter(|e| {
            let event_kind = e.kind();
            event_kind == kind::OPERATION_START || event_kind == kind::OPERATION_END
        })
        .collect();

    // Should have at least 4 operation events (start+end for 2 nodes).
    assert!(
        op_events.len() >= 4,
        "Expected at least 4 operation events, got {}",
        op_events.len()
    );

    // Every event must have a non-empty span_id.
    for event in &*events {
        assert!(
            !event.meta.span_id.is_empty(),
            "Event {:?} has empty span_id",
            event.kind().name()
        );
    }

    // All events within one node execution share the same parent_span_id.
    // Since these are root-level operations (no FLOW_CALL nesting),
    // parent_span_id should be None (no enclosing parent scope).
    // But after dispatch, each node gets its own span, so nested events
    // within a node share the same parent.

    // Verify that different nodes get different parent_span_ids
    // (each node pushes its own span scope).
    let node1_start = op_events.iter().find(|e| {
        e.kind() == kind::OPERATION_START
            && e.payload
                .downcast_ref::<OperationStartPayload>()
                .map_or(false, |p| p.node_id == 1)
    });
    let node2_start = op_events.iter().find(|e| {
        e.kind() == kind::OPERATION_START
            && e.payload
                .downcast_ref::<OperationStartPayload>()
                .map_or(false, |p| p.node_id == 2)
    });

    if let (Some(n1), Some(n2)) = (node1_start, node2_start) {
        assert_ne!(
            n1.meta.parent_span_id, n2.meta.parent_span_id,
            "Different nodes should have different parent_span_ids"
        );
        // Both should have non-None parent_span_id (they're nested under
        // the node span pushed by the dispatcher).
        assert!(
            n1.meta.parent_span_id.is_some(),
            "Node 1 events should have a parent_span_id"
        );
        assert!(
            n2.meta.parent_span_id.is_some(),
            "Node 2 events should have a parent_span_id"
        );
    }

    // Verify that start and end events for the same node share the same
    // parent_span_id (they are emitted within the same span scope).
    let node1_end = op_events.iter().find(|e| {
        e.kind() == kind::OPERATION_END
            && e.payload
                .downcast_ref::<OperationEndPayload>()
                .map_or(false, |p| p.node_id == 1)
    });

    if let (Some(start), Some(end)) = (node1_start, node1_end) {
        assert_eq!(
            start.meta.parent_span_id, end.meta.parent_span_id,
            "Start and end events for the same node should share the same parent_span_id"
        );
    }
}

#[tokio::test]
async fn span_hierarchy_single_node_has_span() {
    let memory = Arc::new(
        MemorySystem::new(MemoryConfig::in_memory_ltm())
            .await
            .unwrap(),
    );
    let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
    let capability_system = Arc::new(CapabilitySystem::new());

    let collecting = Arc::new(CollectingEmitter::new());
    let adapter = Arc::new(EmitterAdapter::new(
        collecting.clone(),
        EventSource::Runtime,
        "test-trace-single",
    ));

    let ctx = ExecutionContext::new(
        memory,
        llm_registry,
        capability_system,
        apxm_runtime::aam::Aam::new(),
    )
    .with_event_emitter(Some(adapter));

    let mut node = Node {
        id: 1,
        op_type: AISOperationType::ConstStr,
        attributes: HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![],
        metadata: NodeMetadata::default(),
    };
    node.attributes
        .insert("value".to_string(), Value::String("test".to_string()));

    let engine = ExecutorEngine::new(ctx);
    let result = engine.execute_node(&node, vec![]).await.unwrap();
    assert_eq!(result, Value::String("test".to_string()));

    // The single node execution should have generated events with span_id.
    let events = collecting.events.lock().unwrap();
    assert!(!events.is_empty(), "Expected at least one event");

    for event in &*events {
        assert!(
            !event.meta.span_id.is_empty(),
            "Event should have a non-empty span_id"
        );
    }
}
