//! End-to-end Layer 1 + Layer 2 event stream smoke.
//!
//! Builds a tiny AIR graph:
//! `CONST_STR → SPAWN_AGENT → ASK → INV_TOOL → MERGE`
//! and asserts the executor produces both the Layer 1 graph events and
//! the Layer 2 agent-layer events in the order CLAUDE.md §10 specifies.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use apxm_backends::LLMRegistry;
use apxm_backends::llm::backends::mock::MockLLMBackend;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::TimingBreakdown;
use apxm_core::types::execution::{ExecutionDag, Node, NodeMetadata};
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_runtime::aam::Aam;
use apxm_runtime::capability::CapabilitySystem;
use apxm_runtime::capability::executor::EchoCapability;
use apxm_runtime::executor::{ExecutionContext, ExecutorEngine};
use apxm_runtime::memory::{MemoryConfig, MemorySystem};
use apxm_runtime::{ExecutionEventEmitter, TokenUsageSummary};

#[derive(Default)]
struct StreamRecorder {
    events: Mutex<Vec<String>>,
}

impl StreamRecorder {
    fn push(&self, name: &str) {
        self.events.lock().unwrap().push(name.to_string());
    }
    fn snapshot(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

impl ExecutionEventEmitter for StreamRecorder {
    fn emit_llm_token(&self, _content: &str) {}
    fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {
        self.push("tool_start");
    }
    fn emit_tool_end(&self, _name: &str, _result: &Value) {
        self.push("tool_end");
    }
    fn emit_operation_start(&self, _node_id: u64, op_type: AISOperationType) {
        self.push(&format!("operation_start[{:?}]", op_type));
    }
    fn emit_operation_start_with_context(
        &self,
        _node_id: u64,
        op_type: AISOperationType,
        _context: serde_json::Value,
    ) {
        self.push(&format!("operation_start[{:?}]", op_type));
    }
    fn emit_operation_end(
        &self,
        _node_id: u64,
        op_type: AISOperationType,
        _duration: Duration,
        _success: bool,
        _tokens: Option<TokenUsageSummary>,
        _timing: Option<TimingBreakdown>,
    ) {
        self.push(&format!("operation_end[{:?}]", op_type));
    }
    fn emit_turn_started(
        &self,
        _execution_id: &str,
        _turn_id: Option<&str>,
        _coordinator_label: Option<&str>,
    ) {
        self.push("turn_started");
    }
    fn emit_turn_complete(&self, _execution_id: &str, _duration_ms: u64, _had_answer: bool) {
        self.push("turn_complete");
    }
    fn emit_turn_aborted(
        &self,
        _execution_id: &str,
        _duration_ms: u64,
        _reason: &str,
        _error_message_safe: Option<&str>,
    ) {
        self.push("turn_aborted");
    }
    fn emit_agent_spawned(
        &self,
        _node_id: u64,
        _agent_code: &str,
        _parent_execution_id: &str,
        _profile: Option<&str>,
        _process_id: Option<&str>,
        _scope_policy: Option<&str>,
    ) {
        self.push("agent_spawned");
    }
    fn emit_subagent_spawn_begin(
        &self,
        _agent_code: &str,
        _agent_name: Option<&str>,
        _agent_type: Option<&str>,
        _module_key: Option<&str>,
        _autonomy_policy: Option<&str>,
        _parent_span_id: Option<&str>,
    ) {
        self.push("subagent_spawn_begin");
    }
    fn emit_subagent_spawn_end(&self, _agent_code: &str) {
        self.push("subagent_spawn_end");
    }
    fn emit_subagent_llm_call_begin(
        &self,
        _agent_code: &str,
        _model: &str,
        _backend: &str,
        _tool_manifest_count: usize,
    ) {
        self.push("subagent_llm_call_begin");
    }
    fn emit_subagent_llm_call_end(
        &self,
        _agent_code: &str,
        _finish_reason: &str,
        _input_tokens: usize,
        _output_tokens: usize,
        _content_len: usize,
    ) {
        self.push("subagent_llm_call_end");
    }
    fn emit_tool_call_begin(&self, _agent_code: &str, _tool_name: &str, _argument_keys: &[String]) {
        self.push("tool_call_begin");
    }
    fn emit_tool_call_end(
        &self,
        _agent_code: &str,
        _tool_name: &str,
        _result_keys: &[String],
        _status: &str,
        _latency_ms: u64,
    ) {
        self.push("tool_call_end");
    }
    fn emit_subagent_done(
        &self,
        _agent_code: &str,
        _total_tool_calls: usize,
        _input_tokens_total: usize,
        _output_tokens_total: usize,
        _evidence_excerpt: Option<&str>,
    ) {
        self.push("subagent_done");
    }
    fn emit_subagent_failed(
        &self,
        _agent_code: &str,
        _error_class: &str,
        _error_message_safe: &str,
    ) {
        self.push("subagent_failed");
    }
    fn emit_agent_message(
        &self,
        _text: &str,
        _item_id: Option<&str>,
        _response_id: Option<&str>,
        _input_tokens: Option<usize>,
        _output_tokens: Option<usize>,
    ) {
        self.push("agent_message");
    }
}

#[tokio::test]
async fn dual_layer_events_smoke_emits_paired_l1_l2_stream() {
    // ── Setup ───────────────────────────────────────────────────────
    let memory = Arc::new(
        MemorySystem::new(MemoryConfig::in_memory_ltm())
            .await
            .unwrap(),
    );
    let llm_registry = Arc::new(LLMRegistry::new());
    llm_registry
        .register("mock", MockLLMBackend::static_response("the answer is 42"))
        .unwrap();
    llm_registry.set_default("mock").unwrap();
    let capability_system = Arc::new(CapabilitySystem::new());
    capability_system
        .register(Arc::new(EchoCapability::new()))
        .unwrap();

    let recorder: Arc<StreamRecorder> = Arc::new(StreamRecorder::default());
    let emitter: Arc<dyn ExecutionEventEmitter> = recorder.clone();
    let mut ctx = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new());
    ctx.event_emitter = Some(emitter);

    // ── DAG: SPAWN_AGENT → ASK → INV_TOOL (sequential) ─────────────
    let mut spawn = Node {
        id: 1,
        op_type: AISOperationType::SpawnAgent,
        attributes: HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![100],
        metadata: NodeMetadata::default(),
    };
    spawn.attributes.insert(
        graph_attrs::AGENT_NAME.to_string(),
        Value::String("crm".to_string()),
    );

    let mut ask = Node {
        id: 2,
        op_type: AISOperationType::Ask,
        attributes: HashMap::new(),
        input_tokens: vec![100],
        output_tokens: vec![101],
        metadata: NodeMetadata::default(),
    };
    ask.attributes.insert(
        graph_attrs::PROMPT.to_string(),
        Value::String("ping".to_string()),
    );
    // Declare a single input_name so the templater stays happy.
    ask.attributes.insert(
        graph_attrs::INPUT_NAMES.to_string(),
        Value::Array(vec![Value::String("agent_info".to_string())]),
    );

    let mut inv = Node {
        id: 3,
        op_type: AISOperationType::InvTool,
        attributes: HashMap::new(),
        // Consume ASK's output so the scheduler runs INV_TOOL after ASK.
        input_tokens: vec![101],
        output_tokens: vec![102],
        metadata: NodeMetadata::default(),
    };
    inv.attributes.insert(
        graph_attrs::CAPABILITY.to_string(),
        Value::String("echo".to_string()),
    );
    inv.attributes.insert(
        graph_attrs::PARAMS_JSON.to_string(),
        Value::String(r#"{"message":"world"}"#.to_string()),
    );
    // INV_TOOL receives one upstream input (ASK's text); declare its name
    // so the templater pairs it with the (placeholder-free) params_json.
    inv.attributes.insert(
        graph_attrs::INPUT_NAMES.to_string(),
        Value::Array(vec![Value::String("answer".to_string())]),
    );

    let dag = ExecutionDag {
        nodes: vec![spawn, ask, inv],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![3],
        metadata: Default::default(),
    };

    let engine = ExecutorEngine::new(ctx);
    let _ = engine.execute_dag(dag).await.expect("execute_dag");

    // ── Assertions ─────────────────────────────────────────────────
    let events = recorder.snapshot();
    println!("dual-layer events: {:?}", events);

    // Layer 1 + Layer 2 pairings must all be present.
    assert!(events.iter().any(|e| e == "turn_started"));
    assert!(events.iter().any(|e| e == "turn_complete"));
    assert!(events.iter().any(|e| e == "operation_start[SpawnAgent]"));
    assert!(events.iter().any(|e| e == "agent_spawned"));
    assert!(events.iter().any(|e| e == "subagent_spawn_begin"));
    assert!(events.iter().any(|e| e == "subagent_spawn_end"));
    assert!(events.iter().any(|e| e == "operation_start[Ask]"));
    assert!(events.iter().any(|e| e == "subagent_llm_call_begin"));
    assert!(events.iter().any(|e| e == "subagent_llm_call_end"));
    assert!(events.iter().any(|e| e == "operation_end[Ask]"));
    assert!(events.iter().any(|e| e == "operation_start[InvTool]"));
    assert!(events.iter().any(|e| e == "tool_call_begin"));
    assert!(events.iter().any(|e| e == "tool_call_end"));
    assert!(events.iter().any(|e| e == "operation_end[InvTool]"));
    // Safety-net pop after the run drains the crm scope SPAWN_AGENT pushed.
    assert!(events.iter().any(|e| e == "subagent_done"));

    // Ordering assertions for key pairs.
    let pos = |needle: &str| events.iter().position(|e| e == needle).unwrap();
    assert!(pos("turn_started") < pos("subagent_spawn_begin"));
    assert!(pos("subagent_spawn_begin") < pos("subagent_spawn_end"));
    assert!(pos("subagent_spawn_end") < pos("subagent_llm_call_begin"));
    assert!(pos("subagent_llm_call_begin") < pos("subagent_llm_call_end"));
    assert!(pos("subagent_llm_call_end") < pos("tool_call_begin"));
    assert!(pos("tool_call_begin") < pos("tool_call_end"));
    assert!(pos("tool_call_end") < pos("subagent_done"));
    assert!(pos("subagent_done") < pos("turn_complete"));
}
