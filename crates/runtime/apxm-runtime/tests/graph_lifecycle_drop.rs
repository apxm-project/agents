//! Drop-guard integration tests for backend graph lifecycles.
//!
//! Runtime owns the lifecycle protocol in terms of the backend trait only:
//! register graph metadata, optionally collect status, and release the graph.
//! Backend-specific HTTP contracts live in `apxm-backends`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use apxm_backends::llm::backends::{LLMBackend, LLMRequest, LLMResponse, TokenUsage};
use apxm_core::types::{
    AISOperationType, ExecutionDag, FinishReason, GraphBackendKind, GraphMetadata,
    GraphStatusSnapshot, ModelInfo, Node,
};
use apxm_runtime::BackendGraphLifecycle;
use async_trait::async_trait;

#[derive(Clone, Default)]
struct RecordingGraphBackend {
    registered_graph_ids: Arc<Mutex<Vec<String>>>,
    released_graph_ids: Arc<Mutex<Vec<String>>>,
}

impl RecordingGraphBackend {
    fn released_graph_ids(&self) -> Vec<String> {
        self.released_graph_ids.lock().unwrap().clone()
    }
}

#[async_trait]
impl LLMBackend for RecordingGraphBackend {
    async fn generate(&self, _request: LLMRequest) -> anyhow::Result<LLMResponse> {
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
        self.registered_graph_ids
            .lock()
            .unwrap()
            .push(metadata.graph_id);
        Ok(())
    }

    async fn release_graph(&self, graph_id: &str) -> anyhow::Result<()> {
        self.released_graph_ids
            .lock()
            .unwrap()
            .push(graph_id.to_string());
        Ok(())
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

fn single_node_dag(name: &str) -> ExecutionDag {
    let mut dag = ExecutionDag::new();
    dag.metadata.name = Some(name.to_string());
    dag.add_node(Node::new(0, AISOperationType::Ask))
        .expect("add node");
    dag.entry_nodes.push(0);
    dag.exit_nodes.push(0);
    dag
}

#[tokio::test]
async fn graph_lifecycle_explicit_release_fires_once() {
    let graph_id = "graph-happy-0001";
    let exec_id = "exec-happy-0001";
    let backend = RecordingGraphBackend::default();
    let dag = single_node_dag("happy");

    let lifecycle = BackendGraphLifecycle::register(
        Arc::new(backend.clone()),
        graph_id.to_string(),
        exec_id.to_string(),
        &dag,
    )
    .await
    .expect("register lifecycle");

    lifecycle.release().await.expect("explicit release");

    drop(lifecycle);
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(backend.released_graph_ids(), vec![graph_id.to_string()]);
}

#[tokio::test]
async fn graph_lifecycle_drop_guard_releases_when_dropped() {
    let graph_id = "graph-drop-0001";
    let exec_id = "exec-drop-0001";
    let backend = RecordingGraphBackend::default();
    let dag = single_node_dag("drop");

    {
        let lifecycle = BackendGraphLifecycle::register(
            Arc::new(backend.clone()),
            graph_id.to_string(),
            exec_id.to_string(),
            &dag,
        )
        .await
        .expect("register lifecycle");
        drop(lifecycle);
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(backend.released_graph_ids(), vec![graph_id.to_string()]);
}

#[tokio::test]
async fn graph_lifecycle_drop_guard_releases_on_task_cancel() {
    let graph_id = "graph-cancel-0001";
    let exec_id = "exec-cancel-0001";
    let backend = RecordingGraphBackend::default();
    let dag = single_node_dag("cancel");

    let lifecycle = BackendGraphLifecycle::register(
        Arc::new(backend.clone()),
        graph_id.to_string(),
        exec_id.to_string(),
        &dag,
    )
    .await
    .expect("register lifecycle");

    let handle = tokio::spawn(async move {
        let _lifecycle = lifecycle;
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    handle.abort();
    let _ = handle.await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(backend.released_graph_ids(), vec![graph_id.to_string()]);
}
