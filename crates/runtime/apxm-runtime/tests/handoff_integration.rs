//! Integration test: HANDOFF operation — two-agent flow with state transfer.
//!
//! Verifies:
//!   1. HANDOFF transfers execution from source to target agent.
//!   2. HANDOFF_START / HANDOFF_END events are emitted.
//!   3. transfer_state=false also works.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::execution::{DependencyType, Edge, ExecutionDag, NodeMetadata};
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;

// ─── Helpers ──────────────────────────────────────────────────────────────

fn make_node(
    id: u64,
    op_type: AISOperationType,
    attrs: Vec<(&str, Value)>,
    input_tokens: Vec<u64>,
    output_tokens: Vec<u64>,
) -> apxm_core::types::execution::Node {
    let mut attributes = HashMap::new();
    for (k, v) in attrs {
        attributes.insert(k.to_string(), v);
    }
    apxm_core::types::execution::Node {
        id,
        op_type,
        attributes,
        input_tokens,
        output_tokens,
        metadata: NodeMetadata::default(),
    }
}

// ─── Recording emitter for event assertions ──────────────────────────────

#[derive(Default)]
struct RecordingEmitter {
    events: Mutex<Vec<(String, u64)>>,
    current_span: Mutex<Option<String>>,
}

impl RecordingEmitter {
    fn snapshot(&self) -> Vec<(String, u64)> {
        self.events.lock().unwrap().clone()
    }
}

impl apxm_runtime::ExecutionEventEmitter for RecordingEmitter {
    fn set_current_span_id(&self, span_id: Option<String>) {
        *self.current_span.lock().unwrap() = span_id;
    }

    fn current_span_id(&self) -> Option<String> {
        self.current_span.lock().unwrap().clone()
    }

    fn emit_llm_token(&self, _content: &str) {}
    fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}
    fn emit_tool_end(&self, _name: &str, _result: &Value) {}

    fn emit_operation_start(&self, node_id: u64, op_type: &str) {
        self.events
            .lock()
            .unwrap()
            .push((format!("start:{op_type}"), node_id));
    }

    fn emit_operation_end(
        &self,
        node_id: u64,
        op_type: &str,
        _duration: Duration,
        _success: bool,
        _tokens: Option<apxm_runtime::TokenUsageSummary>,
        _timing: Option<apxm_core::types::TimingBreakdown>,
    ) {
        self.events
            .lock()
            .unwrap()
            .push((format!("end:{op_type}"), node_id));
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn handoff_transfers_execution_to_target_agent() {
    use apxm_runtime::aam::Aam;
    use apxm_runtime::capability::CapabilitySystem;
    use apxm_runtime::capability::flow_registry::FlowRegistry;
    use apxm_runtime::executor::ExecutorEngine;
    use apxm_runtime::executor::ExecutionContext;
    use apxm_runtime::memory::{MemoryConfig, MemorySystem};

    // Create target agent's flow: just returns a constant string
    let target_dag = ExecutionDag {
        nodes: vec![make_node(
            1,
            AISOperationType::ConstStr,
            vec![(graph_attrs::VALUE, Value::String("target response".into()))],
            vec![],
            vec![100],
        )],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![1],
        metadata: Default::default(),
    };

    let memory = Arc::new(
        MemorySystem::new(MemoryConfig::in_memory_ltm())
            .await
            .unwrap(),
    );
    let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
    let capability_system = Arc::new(CapabilitySystem::new());
    let flow_registry = Arc::new(FlowRegistry::new());
    flow_registry.register_flow("target_bot", "communicate", target_dag);

    let emitter = Arc::new(RecordingEmitter::default());
    let ctx = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
        .with_event_emitter(Some(emitter.clone()));
    let ctx = ExecutionContext {
        flow_registry,
        ..ctx
    };

    // Build a DAG: CONST_STR → HANDOFF
    let dag = ExecutionDag {
        nodes: vec![
            make_node(
                1,
                AISOperationType::ConstStr,
                vec![(graph_attrs::VALUE, Value::String("hello from source".into()))],
                vec![],
                vec![10],
            ),
            make_node(
                2,
                AISOperationType::Handoff,
                vec![
                    (graph_attrs::HANDOFF_FROM, Value::String("source_bot".into())),
                    (graph_attrs::HANDOFF_TO, Value::String("target_bot".into())),
                    (graph_attrs::TRANSFER_STATE, Value::Bool(true)),
                ],
                vec![10],
                vec![20],
            ),
        ],
        edges: vec![Edge::new(1, 2, 10, DependencyType::Data)],
        entry_nodes: vec![1],
        exit_nodes: vec![2],
        metadata: Default::default(),
    };

    let engine = ExecutorEngine::new(ctx);
    let result = engine.execute_dag(dag).await.unwrap();

    // The handoff should return the target agent's response
    let output = result
        .results
        .values()
        .find(|v| !matches!(v, Value::Null))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(output, Value::String("target response".into()));

    // Verify events were emitted
    let events = emitter.snapshot();
    let handoff_start_events: Vec<_> = events
        .iter()
        .filter(|(name, _)| name.contains("HANDOFF_START"))
        .collect();
    let handoff_end_events: Vec<_> = events
        .iter()
        .filter(|(name, _)| name.contains("HANDOFF_END"))
        .collect();

    assert!(
        !handoff_start_events.is_empty(),
        "Expected HANDOFF_START event"
    );
    assert!(
        !handoff_end_events.is_empty(),
        "Expected HANDOFF_END event"
    );
}

#[tokio::test]
async fn handoff_without_transfer_state() {
    use apxm_runtime::aam::Aam;
    use apxm_runtime::capability::CapabilitySystem;
    use apxm_runtime::capability::flow_registry::FlowRegistry;
    use apxm_runtime::executor::ExecutorEngine;
    use apxm_runtime::executor::ExecutionContext;
    use apxm_runtime::memory::{MemoryConfig, MemorySystem};

    let target_dag = ExecutionDag {
        nodes: vec![make_node(
            1,
            AISOperationType::ConstStr,
            vec![(graph_attrs::VALUE, Value::String("no-state response".into()))],
            vec![],
            vec![100],
        )],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![1],
        metadata: Default::default(),
    };

    let memory = Arc::new(
        MemorySystem::new(MemoryConfig::in_memory_ltm())
            .await
            .unwrap(),
    );
    let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
    let capability_system = Arc::new(CapabilitySystem::new());
    let flow_registry = Arc::new(FlowRegistry::new());
    flow_registry.register_flow("target_bot", "communicate", target_dag);

    let ctx = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new());
    let ctx = ExecutionContext {
        flow_registry,
        ..ctx
    };

    let dag = ExecutionDag {
        nodes: vec![
            make_node(
                1,
                AISOperationType::ConstStr,
                vec![(graph_attrs::VALUE, Value::String("payload".into()))],
                vec![],
                vec![10],
            ),
            make_node(
                2,
                AISOperationType::Handoff,
                vec![
                    (graph_attrs::HANDOFF_FROM, Value::String("src".into())),
                    (graph_attrs::HANDOFF_TO, Value::String("target_bot".into())),
                    (graph_attrs::TRANSFER_STATE, Value::Bool(false)),
                ],
                vec![10],
                vec![20],
            ),
        ],
        edges: vec![Edge::new(1, 2, 10, DependencyType::Data)],
        entry_nodes: vec![1],
        exit_nodes: vec![2],
        metadata: Default::default(),
    };

    let engine = ExecutorEngine::new(ctx);
    let result = engine.execute_dag(dag).await.unwrap();

    let output = result
        .results
        .values()
        .find(|v| !matches!(v, Value::Null))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(output, Value::String("no-state response".into()));
}
