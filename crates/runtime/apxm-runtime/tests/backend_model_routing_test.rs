//! Integration regressions for backend/model routing from graph attrs.
//!
//! These tests verify the runtime preserves the distinction between:
//! - `backend`: explicit backend selection for registry routing
//! - `model`: model identifier forwarded to the chosen backend
//!
//! The critical regression is the Ask/tool-loop path: if the runtime drops the
//! node's `backend` attr, requests fall back to the registry default backend.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use apxm_backends::llm::backends::mock::{MockLLMBackend, MockResponse};
use apxm_backends::{LLMBackend, LLMRequest, LLMResponse, StreamChunk};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::observability::CallTrace;
use apxm_core::types::execution::NodeMetadata;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::{Number, Value};
use apxm_core::types::{ExecutionDag, FinishReason, ModelInfo, Node, TokenUsage, ToolCall};
use apxm_runtime::capability::executor::EchoCapability;
use apxm_runtime::{Runtime, RuntimeConfig};
use async_trait::async_trait;
use parking_lot::RwLock;
use tokio_stream::Stream;

fn single_ask_dag(prompt: &str, backend: &str, model: &str) -> ExecutionDag {
    let mut attrs = HashMap::new();
    attrs.insert(
        graph_attrs::PROMPT.to_string(),
        Value::String(prompt.to_string()),
    );
    attrs.insert(
        graph_attrs::BACKEND.to_string(),
        Value::String(backend.to_string()),
    );
    attrs.insert(
        graph_attrs::MODEL.to_string(),
        Value::String(model.to_string()),
    );

    let ask = Node {
        id: 1,
        op_type: AISOperationType::Ask,
        attributes: attrs,
        input_tokens: vec![],
        output_tokens: vec![10],
        metadata: NodeMetadata::default(),
    };

    let mut dag = ExecutionDag::new();
    dag.add_node(ask).expect("add ask node");
    dag.entry_nodes = dag.find_entry_nodes();
    dag.exit_nodes = dag.find_exit_nodes();
    dag
}

fn ask_tool_loop_dag(
    prompt: &str,
    backend: &str,
    model: &str,
    max_iterations: usize,
) -> ExecutionDag {
    let mut dag = single_ask_dag(prompt, backend, model);
    let ask = dag.nodes.first_mut().expect("ask node present");
    ask.attributes
        .insert(graph_attrs::TOOLS_ENABLED.to_string(), Value::Bool(true));
    ask.attributes.insert(
        graph_attrs::MAX_TOOL_ITERATIONS.to_string(),
        Value::Number(Number::Integer(max_iterations as i64)),
    );
    dag
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoutedRequest {
    backend: Option<String>,
    model: Option<String>,
    prompt: String,
}

#[derive(Clone, Default)]
struct RequestRecorder {
    requests: Arc<Mutex<Vec<RoutedRequest>>>,
}

impl RequestRecorder {
    fn record(&self, request: &LLMRequest) {
        self.requests.lock().unwrap().push(RoutedRequest {
            backend: request.backend.clone(),
            model: request.model.clone(),
            prompt: request.prompt.clone(),
        });
    }

    fn snapshots(&self) -> Vec<RoutedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[derive(Clone)]
struct ToolLoopRoutingBackend {
    recorder: RequestRecorder,
    total_tool_iterations: usize,
    counter: Arc<Mutex<usize>>,
}

impl ToolLoopRoutingBackend {
    fn new(recorder: RequestRecorder, total_tool_iterations: usize) -> Self {
        Self {
            recorder,
            total_tool_iterations,
            counter: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl LLMBackend for ToolLoopRoutingBackend {
    async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse> {
        self.recorder.record(&request);

        let mut count = self.counter.lock().unwrap();
        let current = *count;
        *count += 1;
        drop(count);

        if current < self.total_tool_iterations {
            let tool_call = ToolCall::new(
                format!("call_{current}"),
                "echo",
                serde_json::json!({ "message": format!("step {current}") }),
            );
            Ok(LLMResponse::new(
                "",
                "override-native-model",
                TokenUsage::new(5, 5),
                FinishReason::ToolUse,
            )
            .with_tool_calls(vec![tool_call]))
        } else {
            Ok(LLMResponse::new(
                "done",
                "override-native-model",
                TokenUsage::new(5, 5),
                FinishReason::Stop,
            ))
        }
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send + '_>> {
        Box::pin(futures::stream::once(async move {
            let response = self.generate(request).await?;
            Ok(StreamChunk::Done(response))
        }))
    }

    fn name(&self) -> &str {
        "override-backend"
    }

    fn model(&self) -> &str {
        "override-native-model"
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: "override-native-model".to_string(),
            name: "Override Native Model".to_string(),
            context_window: 8_192,
            supports_vision: false,
            supports_functions: true,
        }])
    }
}

#[tokio::test]
async fn ask_backend_attr_overrides_default_backend_without_changing_model() {
    let runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("runtime init");

    let override_trace = Arc::new(RwLock::new(CallTrace::new()));
    let default_backend = MockLLMBackend::new().always_fail("default backend should not be used");
    let override_backend = MockLLMBackend::new_with_trace(override_trace.clone())
        .named("override-backend")
        .model_name("override-native-model")
        .default(MockResponse::new("override response"));

    runtime
        .llm_registry()
        .register("default-backend", default_backend.clone())
        .expect("register default backend");
    runtime
        .llm_registry()
        .register("override-backend", override_backend.clone())
        .expect("register override backend");
    runtime
        .llm_registry()
        .set_default("default-backend")
        .expect("set default backend");

    runtime
        .execute(single_ask_dag(
            "test prompt",
            "override-backend",
            "requested-model",
        ))
        .await
        .expect("execution succeeds");

    assert_eq!(
        default_backend.call_count(),
        0,
        "default backend must not be used"
    );
    assert_eq!(
        override_backend.call_count(),
        1,
        "override backend should handle the ask"
    );

    let trace = override_trace.read();
    assert_eq!(trace.len(), 1, "expected one backend call");
    assert_eq!(trace.events[0].op, "ASK");
    assert_eq!(trace.events[0].prompt, "test prompt");
    assert_eq!(
        trace.events[0].model, "requested-model",
        "runtime must preserve requested model while routing to explicit backend",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ask_tool_loop_preserves_backend_override_across_retries() {
    const TOOL_ITERATIONS: usize = 2;

    let runtime = Arc::new(
        Runtime::new(RuntimeConfig::in_memory())
            .await
            .expect("runtime init"),
    );

    let recorder = RequestRecorder::default();
    let override_backend = ToolLoopRoutingBackend::new(recorder.clone(), TOOL_ITERATIONS);
    let default_backend = MockLLMBackend::new().always_fail("default backend should not be used");

    runtime
        .llm_registry()
        .register("default-backend", default_backend.clone())
        .expect("register default backend");
    runtime
        .llm_registry()
        .register("override-backend", override_backend)
        .expect("register override backend");
    runtime
        .llm_registry()
        .set_default("default-backend")
        .expect("set default backend");
    runtime
        .capability_system()
        .register(Arc::new(EchoCapability::new()))
        .expect("register echo capability");

    runtime
        .execute(ask_tool_loop_dag(
            "tool prompt",
            "override-backend",
            "requested-model",
            TOOL_ITERATIONS + 1,
        ))
        .await
        .expect("tool loop execution succeeds");

    assert_eq!(
        default_backend.call_count(),
        0,
        "default backend must not be used"
    );

    let snapshots = recorder.snapshots();
    assert_eq!(
        snapshots.len(),
        TOOL_ITERATIONS + 1,
        "expected one request per tool-loop iteration plus the final completion",
    );
    for (index, request) in snapshots.iter().enumerate() {
        assert_eq!(
            request.backend.as_deref(),
            Some("override-backend"),
            "iteration {index} lost the explicit backend override",
        );
        assert_eq!(
            request.model.as_deref(),
            Some("requested-model"),
            "iteration {index} lost the requested model",
        );
    }
}
