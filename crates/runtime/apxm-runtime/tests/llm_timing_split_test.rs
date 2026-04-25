//! Integration test: LLM call records prefill/decode timing that flows through
//! the dispatcher and into `emit_operation_end` as `Option<TimingBreakdown>`.
//!
//! For non-streaming MockLLMBackend the entire LLM call is one round-trip, so
//! prefill_ms = total_ms and decode_ms = 0.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use apxm_backends::llm::backends::mock::MockLLMBackend;
use apxm_core::types::TimingBreakdown;
use apxm_core::types::execution::{ExecutionDag, Node, NodeMetadata};
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_runtime::capability::CapabilitySystem;
use apxm_runtime::executor::{ExecutionContext, ExecutorEngine};
use apxm_runtime::memory::{MemoryConfig, MemorySystem};
use apxm_runtime::{ExecutionEventEmitter, TokenUsageSummary};

/// Recording emitter that captures every `emit_operation_end` call.
#[derive(Default)]
struct RecordingEmitter {
    ends: Mutex<Vec<(u64, Duration, Option<TimingBreakdown>)>>,
}

impl ExecutionEventEmitter for RecordingEmitter {
    fn emit_llm_token(&self, _content: &str) {}
    fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}
    fn emit_tool_end(&self, _name: &str, _result: &Value) {}

    fn emit_operation_end(
        &self,
        node_id: u64,
        _op_type: AISOperationType,
        duration: Duration,
        _success: bool,
        _tokens: Option<TokenUsageSummary>,
        timing: Option<TimingBreakdown>,
    ) {
        self.ends.lock().unwrap().push((node_id, duration, timing));
    }
}

#[tokio::test]
async fn llm_timing_split_populates_prefill_for_non_streaming_backend() {
    // 1. Build the runtime subsystems and register MockLLMBackend.
    let memory = Arc::new(
        MemorySystem::new(MemoryConfig::in_memory_ltm())
            .await
            .unwrap(),
    );
    let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
    let mock = MockLLMBackend::static_response("mock response text");
    llm_registry
        .register("mock-backend", mock)
        .expect("register mock backend");
    llm_registry
        .set_default("mock-backend")
        .expect("set default backend");

    let capability_system = Arc::new(CapabilitySystem::new());

    // 2. Wire a recording emitter into the ExecutionContext.
    let recorder: Arc<RecordingEmitter> = Arc::new(RecordingEmitter::default());
    let ctx = ExecutionContext::new(
        memory,
        llm_registry,
        capability_system,
        apxm_runtime::aam::Aam::new(),
    )
    .with_event_emitter(Some(recorder.clone()));

    // 3. Build a single-Ask DAG.
    let mut attrs = HashMap::new();
    attrs.insert(
        "prompt".to_string(),
        Value::String("test prompt".to_string()),
    );
    attrs.insert("model".to_string(), Value::String("mock-model".to_string()));
    let ask = Node {
        id: 1,
        op_type: AISOperationType::Ask,
        attributes: attrs,
        input_tokens: vec![],
        output_tokens: vec![10],
        metadata: NodeMetadata::default(),
    };
    let mut dag = ExecutionDag::new();
    dag.add_node(ask).unwrap();
    dag.entry_nodes = dag.find_entry_nodes();
    dag.exit_nodes = dag.find_exit_nodes();

    // 4. Execute via ExecutorEngine.
    let engine = ExecutorEngine::new(ctx);
    let result = engine.execute_dag(dag).await.expect("execution succeeds");
    assert_eq!(result.stats.executed_nodes, 1, "ASK node executed");
    assert_eq!(result.stats.failed_nodes, 0, "no failures");

    // 5. Assert the recorder captured timing for the LLM node.
    let ends = recorder.ends.lock().unwrap().clone();
    let (_, duration, timing) = ends
        .iter()
        .find(|(id, _, _)| *id == 1)
        .cloned()
        .expect("operation_end emitted for node 1");

    let timing = timing.expect("timing breakdown populated for LLM node");
    assert!(
        timing.prefill_ms > 0.0,
        "prefill_ms must be > 0 for a non-streaming round-trip; got {}",
        timing.prefill_ms
    );
    assert_eq!(
        timing.decode_ms, 0.0,
        "decode_ms must be 0 for non-streaming backend; got {}",
        timing.decode_ms
    );

    // duration_ms is u64 truncation of dispatcher wall-time. The LLM
    // round-trip (prefill_ms) is a strict sub-interval of dispatcher
    // execution, so it must not exceed dispatcher duration_ms by more than
    // the truncation slack (1ms).
    let duration_ms_f = duration.as_millis() as f64;
    assert!(
        timing.prefill_ms <= duration_ms_f + 1.0,
        "prefill_ms ({}) should not exceed dispatcher duration_ms ({}) by more than 1ms",
        timing.prefill_ms,
        duration_ms_f
    );
}
