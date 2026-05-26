use std::sync::{Arc, Mutex};

use apxm_backends::llm::backends::{LLMBackend, LLMRequest, LLMResponse, TokenUsage};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{
    AISOperationType, BackendGraphCapabilities, ExecutionDag, FinishReason, GraphBackendKind,
    GraphMetadata, GraphStatusSnapshot, ModelInfo, Node, Value,
};
use apxm_runtime::{Runtime, RuntimeConfig};
use async_trait::async_trait;

#[derive(Clone, Default)]
struct RecordingGraphBackend {
    registered_graph_ids: Arc<Mutex<Vec<String>>>,
    registered_execution_ids: Arc<Mutex<Vec<String>>>,
    request_graph_ids: Arc<Mutex<Vec<String>>>,
    request_execution_ids: Arc<Mutex<Vec<String>>>,
    released_graph_ids: Arc<Mutex<Vec<String>>>,
}

impl RecordingGraphBackend {
    fn registered_graph_ids(&self) -> Vec<String> {
        self.registered_graph_ids.lock().unwrap().clone()
    }

    fn registered_execution_ids(&self) -> Vec<String> {
        self.registered_execution_ids.lock().unwrap().clone()
    }

    fn request_graph_ids(&self) -> Vec<String> {
        self.request_graph_ids.lock().unwrap().clone()
    }

    fn request_execution_ids(&self) -> Vec<String> {
        self.request_execution_ids.lock().unwrap().clone()
    }

    fn released_graph_ids(&self) -> Vec<String> {
        self.released_graph_ids.lock().unwrap().clone()
    }
}

#[async_trait]
impl LLMBackend for RecordingGraphBackend {
    async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse> {
        let graph_id = request
            .apxm_hints
            .as_ref()
            .and_then(|hints| hints.graph_id.clone())
            .unwrap_or_default();
        let execution_id = request
            .apxm_hints
            .as_ref()
            .and_then(|hints| hints.execution_id.clone())
            .unwrap_or_default();
        self.request_graph_ids.lock().unwrap().push(graph_id);
        self.request_execution_ids
            .lock()
            .unwrap()
            .push(execution_id);

        Ok(LLMResponse::new(
            "ok",
            self.model(),
            TokenUsage::new(1, 1),
            FinishReason::Stop,
        ))
    }

    fn name(&self) -> &str {
        "recording-graph-backend"
    }

    fn model(&self) -> &str {
        "test-model"
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }

    async fn register_graph(&self, metadata: GraphMetadata) -> anyhow::Result<()> {
        let execution_id = metadata.execution_id.clone().unwrap_or_default();
        self.registered_graph_ids
            .lock()
            .unwrap()
            .push(metadata.graph_id);
        self.registered_execution_ids
            .lock()
            .unwrap()
            .push(execution_id);
        Ok(())
    }

    async fn release_graph(&self, graph_id: &str) -> anyhow::Result<()> {
        self.released_graph_ids
            .lock()
            .unwrap()
            .push(graph_id.to_string());
        Ok(())
    }

    fn supports_graph_extensions(&self) -> bool {
        true
    }

    fn graph_capabilities(&self) -> BackendGraphCapabilities {
        BackendGraphCapabilities {
            supports_graph_registration: true,
            supports_request_hints: true,
            ..BackendGraphCapabilities::default()
        }
    }

    async fn get_graph_status(
        &self,
        graph_id: &str,
    ) -> anyhow::Result<Option<GraphStatusSnapshot>> {
        Ok(Some(
            GraphStatusSnapshot::new(GraphBackendKind::Generic, graph_id).with_registered(true),
        ))
    }
}

fn single_ask_dag(graph_id: &str) -> ExecutionDag {
    let mut node = Node::new(7, AISOperationType::Ask);
    node.metadata.name = Some("ask".to_string());
    node.set_attribute(
        graph_attrs::PROMPT.to_string(),
        Value::String("hello".to_string()),
    );
    node.add_output_token(70);

    let mut dag = ExecutionDag::new();
    dag.metadata.name = Some(graph_id.to_string());
    dag.add_node(node).expect("add ask node");
    dag.entry_nodes.push(7);
    dag.exit_nodes.push(7);
    dag
}

#[tokio::test]
async fn runtime_graph_lifecycle_and_request_hints_share_graph_id() {
    let backend = RecordingGraphBackend::default();
    let runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("runtime");
    runtime
        .llm_registry()
        .register("recording", backend.clone())
        .expect("register backend");
    runtime
        .llm_registry()
        .set_default("recording")
        .expect("set default backend");

    let graph_id = "registered-runtime-graph";
    runtime
        .execute(single_ask_dag(graph_id))
        .await
        .expect("execute graph");

    assert_eq!(backend.registered_graph_ids(), vec![graph_id.to_string()]);
    assert_eq!(backend.request_graph_ids(), vec![graph_id.to_string()]);
    assert_eq!(backend.released_graph_ids(), vec![graph_id.to_string()]);

    let registered_execution_ids = backend.registered_execution_ids();
    let request_execution_ids = backend.request_execution_ids();
    assert_eq!(registered_execution_ids.len(), 1);
    assert_eq!(request_execution_ids.len(), 1);
    assert_ne!(registered_execution_ids[0], graph_id);
    assert_ne!(request_execution_ids[0], graph_id);
    assert!(!registered_execution_ids[0].is_empty());
    assert!(!request_execution_ids[0].is_empty());
}
