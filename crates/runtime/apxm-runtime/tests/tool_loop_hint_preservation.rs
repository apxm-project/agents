//! Step 3b regression test: APXM graph hints and provider-specific request
//! controls survive every iteration of the tool-call retry loop in
//! `execute_ask_with_tools`.
//!
//! Without the explicit `apxm_hints` clone added at the bottom of the loop,
//! only the first chat completion would carry hints — every follow-up
//! request would arrive with `apxm_hints == None` because the backend's
//! `inject_hints` skips re-injection once the backend-specific APXM hint field
//! is set.
//!
//! This test wires a custom recording backend through the runtime, runs an
//! Ask node that forces N rounds of tool calls, and asserts that every
//! recorded request carried the same `apxm_hints` and `extra_body` payloads.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use apxm_backends::llm::backends::{LLMBackend, StreamChunk};
use apxm_backends::{LLMRequest, LLMResponse};
use apxm_core::constants::llm::apxm as apxm_llm;
use apxm_core::types::execution::NodeMetadata;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_core::types::{ExecutionDag, FinishReason, ModelInfo, Node, TokenUsage, ToolCall};
use apxm_runtime::{Runtime, RuntimeConfig};
use async_trait::async_trait;
use tokio_stream::Stream;

#[derive(Clone, Debug, PartialEq)]
struct RequestSnapshot {
    hints: Option<serde_json::Value>,
    extra_body: Option<serde_json::Value>,
}

/// Records APXM hints and provider-specific request controls the backend sees.
#[derive(Clone, Default)]
struct HintsRecorder {
    /// Snapshot of each request, in arrival order.
    snapshots: Arc<Mutex<Vec<RequestSnapshot>>>,
}

impl HintsRecorder {
    fn new() -> Self {
        Self {
            snapshots: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn record(&self, request: &LLMRequest) {
        let hints = request
            .apxm_hints
            .as_ref()
            .and_then(|h| serde_json::to_value(h).ok());
        self.snapshots.lock().unwrap().push(RequestSnapshot {
            hints,
            extra_body: request.extra_body.clone(),
        });
    }

    fn snapshots(&self) -> Vec<RequestSnapshot> {
        self.snapshots.lock().unwrap().clone()
    }
}

/// Mock backend that emits `total_tool_iterations` rounds of tool calls and
/// then a final text response. Each call's `apxm_hints` is recorded.
#[derive(Clone)]
struct ToolLoopMock {
    recorder: HintsRecorder,
    total_tool_iterations: usize,
    counter: Arc<Mutex<usize>>,
}

impl ToolLoopMock {
    fn new(recorder: HintsRecorder, total_tool_iterations: usize) -> Self {
        Self {
            recorder,
            total_tool_iterations,
            counter: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl LLMBackend for ToolLoopMock {
    async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse> {
        self.recorder.record(&request);

        let mut count = self.counter.lock().unwrap();
        let current = *count;
        *count += 1;
        drop(count);

        let usage = TokenUsage::new(5, 5);

        if current < self.total_tool_iterations {
            // Emit a single tool call to force another loop iteration.
            let call = ToolCall::new(
                format!("call_{}", current),
                "echo",
                serde_json::json!({ "message": format!("step {}", current) }),
            );
            Ok(
                LLMResponse::new("", "tool-loop-mock", usage, FinishReason::ToolUse)
                    .with_tool_calls(vec![call]),
            )
        } else {
            // Final response: no tool calls → loop exits.
            Ok(LLMResponse::new(
                "done",
                "tool-loop-mock",
                usage,
                FinishReason::Stop,
            ))
        }
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send + '_>> {
        // Default impl wraps generate() into a single Done chunk; reuse it so
        // both the non-streaming and streaming runtime paths exercise record().
        Box::pin(futures::stream::once(async move {
            let response = self.generate(request).await?;
            Ok(StreamChunk::Done(response))
        }))
    }

    fn name(&self) -> &str {
        "tool-loop-mock"
    }

    fn model(&self) -> &str {
        "tool-loop-mock"
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: "tool-loop-mock".to_string(),
            name: "Tool Loop Mock".to_string(),
            context_window: 8_192,
            supports_vision: false,
            supports_functions: true,
        }])
    }
}

/// Build an Ask node that:
/// - forces tools on (TOOLS_ENABLED=true, no explicit TOOLS list → all caps),
/// - caps tool iterations at `max_iters + 1` so the loop runs to natural exit,
/// - carries graph hints we can verify survive every retry.
fn ask_node_with_graph_hints(max_iters: usize) -> Node {
    use apxm_core::constants::graph::attrs as a;

    let mut attrs: HashMap<String, Value> = HashMap::new();
    attrs.insert(a::PROMPT.to_string(), Value::String("hello".into()));
    attrs.insert(a::TOOLS_ENABLED.to_string(), Value::Bool(true));
    attrs.insert(
        a::MAX_TOOL_ITERATIONS.to_string(),
        Value::Number(apxm_core::types::values::Number::Integer(
            (max_iters + 1) as i64,
        )),
    );

    // Graph hints must reach the backend on every retry, not just the first one.
    attrs.insert(
        a::PRIORITY.to_string(),
        Value::Number(apxm_core::types::values::Number::Integer(
            apxm_core::constants::graph::metadata::CRITICAL_PATH_PRIORITY_THRESHOLD,
        )),
    );
    attrs.insert(a::REUSE_GROUP.to_string(), Value::String("group-A".into()));
    attrs.insert(
        a::DOWNSTREAM_NODES.to_string(),
        Value::Array(vec![
            Value::Number(apxm_core::types::values::Number::Integer(2)),
            Value::Number(apxm_core::types::values::Number::Integer(3)),
        ]),
    );
    attrs.insert(
        a::SHARED_PREFIX_EST_TOKENS.to_string(),
        Value::Number(apxm_core::types::values::Number::Integer(1024)),
    );
    attrs.insert(a::WARMUP_CANDIDATE.to_string(), Value::Bool(true));
    attrs.insert(
        "vllm_cache_salt".to_string(),
        Value::String("execution".into()),
    );

    Node {
        id: 1,
        op_type: AISOperationType::Ask,
        attributes: attrs,
        input_tokens: vec![],
        output_tokens: vec![10],
        metadata: NodeMetadata::default(),
    }
}

fn ask_dag(max_iters: usize) -> ExecutionDag {
    let ask = ask_node_with_graph_hints(max_iters);
    let mut dag = ExecutionDag::new();
    dag.add_node(ask).unwrap();
    dag.entry_nodes = dag.find_entry_nodes();
    dag.exit_nodes = dag.find_exit_nodes();
    dag
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apxm_hints_preserved_across_every_tool_loop_iteration() {
    use apxm_runtime::capability::executor::EchoCapability;

    const TOOL_ITERATIONS: usize = 3;

    let recorder = HintsRecorder::new();
    let backend = ToolLoopMock::new(recorder.clone(), TOOL_ITERATIONS);

    let runtime = Arc::new(
        Runtime::new(RuntimeConfig::in_memory())
            .await
            .expect("runtime init"),
    );

    // Register the recording backend as default so the Ask handler routes to it.
    runtime
        .llm_registry()
        .register("tool-loop-mock", backend)
        .expect("register backend");
    runtime
        .llm_registry()
        .set_default("tool-loop-mock")
        .expect("set default backend");

    // Echo capability satisfies the tool call from the mock without external
    // dependencies.
    runtime
        .capability_system()
        .register(Arc::new(EchoCapability::new()))
        .expect("register echo capability");

    let dag = ask_dag(TOOL_ITERATIONS);
    runtime
        .execute(dag)
        .await
        .expect("graph should execute to completion");

    let snapshots = recorder.snapshots();

    // Sanity: tool loop ran TOOL_ITERATIONS + 1 times (N tool rounds + 1 final).
    assert_eq!(
        snapshots.len(),
        TOOL_ITERATIONS + 1,
        "expected {} backend calls (got {})",
        TOOL_ITERATIONS + 1,
        snapshots.len(),
    );

    // Every call must carry hints and provider controls; neither may be
    // dropped on retry.
    for (i, snapshot) in snapshots.iter().enumerate() {
        assert!(
            snapshot.hints.is_some(),
            "iteration {} dropped apxm_hints (Step 3b regression)",
            i
        );
        assert!(
            snapshot.extra_body.is_some(),
            "iteration {} dropped extra_body request controls",
            i
        );
    }

    // All snapshots must be byte-identical: the loop must clone the same
    // hints, not rebuild a different shape.
    let first = snapshots[0]
        .hints
        .as_ref()
        .expect("first call must carry hints");
    for (i, snap) in snapshots.iter().enumerate().skip(1) {
        let other = snap.hints.as_ref().expect("checked above");
        assert_eq!(
            other, first,
            "iteration {} hints diverged from initial request",
            i
        );
    }

    // Spot-check that the graph hint fields survived intact end-to-end.
    let obj = first.as_object().expect("hints must serialize as object");
    assert_eq!(
        obj.get(apxm_llm::PRIORITY_CLASS),
        Some(&serde_json::json!(apxm_llm::PRIORITY_CRITICAL_PATH)),
    );
    assert_eq!(
        obj.get(apxm_llm::REUSE_GROUP),
        Some(&serde_json::json!("group-A"))
    );
    assert_eq!(
        obj.get(apxm_llm::DOWNSTREAM_NODES),
        Some(&serde_json::json!([2, 3])),
    );
    assert_eq!(
        obj.get(apxm_llm::COMPILER_HINTS)
            .and_then(|c| c.get(apxm_llm::SHARED_PREFIX_EST_TOKENS)),
        Some(&serde_json::json!(1024)),
    );
    assert_eq!(
        obj.get(apxm_llm::COMPILER_HINTS)
            .and_then(|c| c.get(apxm_llm::WARMUP_CANDIDATE)),
        Some(&serde_json::json!(true)),
    );
    assert_eq!(
        obj.get(apxm_llm::PIN_POLICY)
            .and_then(|p| p.get(apxm_llm::PIN_POLICY_MODE)),
        Some(&serde_json::json!(apxm_llm::PIN_MODE_PREFIX)),
    );

    // `vllm_cache_salt` lowers into extra_body. Tool-loop continuations must
    // preserve it exactly, otherwise backend cache/priority controls are lost
    // after the first tool call.
    let first_extra = snapshots[0]
        .extra_body
        .as_ref()
        .expect("first call must carry extra_body");
    let cache_salt = first_extra
        .get("cache_salt")
        .and_then(|value| value.as_str())
        .expect("cache_salt must be lowered into extra_body");
    assert!(
        !cache_salt.is_empty(),
        "execution-derived cache_salt must not be empty"
    );
    for (i, snap) in snapshots.iter().enumerate().skip(1) {
        let other = snap.extra_body.as_ref().expect("checked above");
        assert_eq!(
            other, first_extra,
            "iteration {} extra_body diverged from initial request",
            i
        );
    }
}
