//! Graph-aware vLLM backend.
//!
//! This backend wraps the OpenAI-compatible vLLM server and adds APXM graph
//! hints to every request. It also provides methods to register and release
//! graphs on the server side for KV-cache pinning optimizations.

use super::graph_meta::GraphMetadata;
use crate::llm::backends::openai::OpenAIBackend;
use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse};
use anyhow::{Context, Result};
use apxm_core::types::ModelInfo;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_stream::Stream;

const DEFAULT_BASE_URL: &str = "http://localhost:8000";

/// Response from `POST /v1/apxm/graphs/register`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRegisterResponse {
    pub object: String,
    pub graph_id: String,
    pub execution_id: Option<String>,
    pub registered_nodes: u32,
}

/// Response from `DELETE /v1/apxm/graphs/{graph_id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphReleaseResponse {
    pub object: String,
    pub graph_id: String,
    pub released_handles: u32,
    pub released_blocks: u32,
}

/// Graph-aware vLLM backend.
///
/// Wraps an OpenAI-compatible vLLM server and injects APXM graph hints into
/// each request's `extra_body.apxm` field, enabling:
///
/// - Critical-path priority scheduling
/// - KV-cache pinning for prefix reuse
/// - Graph-level pin TTL defaults
/// - Eager prefill hints
///
/// # Example
///
/// ```ignore
/// let backend = GraphAwareVllmBackend::new("", Some(json!({
///     "base_url": "http://vllm-server:8000",
///     "model": "meta-llama/Llama-3.1-8B-Instruct"
/// }))).await?;
///
/// // Register graph before sending requests
/// backend.register_graph(GraphMetadata::new("dag-123", "exec-abc")).await?;
///
/// // Send request with hints
/// let request = LLMRequest::new("Hello")
///     .with_apxm_hints(ApxmGraphHints::critical_path(
///         "dag-123", "exec-abc", 1, "greet", vec![2], 30_000
///     ));
/// let response = backend.generate(request).await?;
///
/// // Release graph when done
/// backend.release_graph("dag-123").await?;
/// ```
pub struct GraphAwareVllmBackend {
    /// Inner OpenAI-compatible backend for actual requests.
    inner: OpenAIBackend,
    /// Base URL for APXM-specific endpoints (`/v1/apxm/...`).
    base_url: String,
    /// HTTP client for graph management endpoints.
    client: reqwest::Client,
    /// Counter for generating unique execution IDs.
    execution_counter: AtomicU64,
}

impl GraphAwareVllmBackend {
    /// Create a new graph-aware vLLM backend.
    ///
    /// Config keys:
    /// - `base_url`: vLLM server URL (default: `http://localhost:8000`)
    /// - `model`: Model name to use
    /// - `extra_headers`: Optional HTTP headers
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        let base_url = config
            .as_ref()
            .and_then(|c| c.get("base_url"))
            .and_then(|u| u.as_str())
            .unwrap_or(DEFAULT_BASE_URL)
            .trim_end_matches('/')
            .to_string();

        // Pass config to inner OpenAI backend (vLLM is OpenAI-compatible)
        let inner = OpenAIBackend::new(api_key, config).await?;
        let client = reqwest::Client::new();

        Ok(Self {
            inner,
            base_url,
            client,
            execution_counter: AtomicU64::new(0),
        })
    }

    /// Generate a unique execution ID for a graph run.
    pub fn next_execution_id(&self) -> String {
        let counter = self.execution_counter.fetch_add(1, Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("exec-{}-{}", timestamp, counter)
    }

    /// Register a graph with the vLLM server for scheduling hints.
    ///
    /// Call this once per graph execution before sending individual node
    /// requests. The server stores the metadata and uses it for KV-cache
    /// pinning decisions.
    pub async fn register_graph(&self, metadata: GraphMetadata) -> Result<GraphRegisterResponse> {
        let url = format!("{}/v1/apxm/graphs/register", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&metadata)
            .send()
            .await
            .context("Failed to send graph registration request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Graph registration failed: {} - {}", status, body);
        }

        response
            .json()
            .await
            .context("Failed to parse graph registration response")
    }

    /// Release a graph's KV-cache pins on the vLLM server.
    ///
    /// Call this when a graph execution completes or is cancelled to free
    /// pinned KV blocks.
    pub async fn release_graph(&self, graph_id: &str) -> Result<GraphReleaseResponse> {
        let url = format!("{}/v1/apxm/graphs/{}", self.base_url, graph_id);
        let response = self
            .client
            .delete(&url)
            .send()
            .await
            .context("Failed to send graph release request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Graph release failed: {} - {}", status, body);
        }

        response
            .json()
            .await
            .context("Failed to parse graph release response")
    }

    /// Inject APXM hints into a request if present.
    fn inject_hints(&self, mut request: LLMRequest) -> LLMRequest {
        // Check if request already has APXM hints in extra_body
        if let Some(ref extra) = request.extra_body {
            if extra.get("apxm").is_some() {
                return request; // Already has hints
            }
        }

        // Check if there are graph hints attached to the request
        if let Some(ref hints) = request.apxm_hints {
            let hints_json = serde_json::to_value(hints).unwrap_or_default();
            let mut extra = request.extra_body.take().unwrap_or_default();
            if let serde_json::Value::Object(ref mut map) = extra {
                map.insert("apxm".to_string(), hints_json);
            }
            request.extra_body = Some(extra);
        }

        request
    }
}

#[async_trait]
impl LLMBackend for GraphAwareVllmBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        let request = self.inject_hints(request);
        self.inner.generate(request).await
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        let request = self.inject_hints(request);
        self.inner.generate_stream(request)
    }

    fn name(&self) -> &str {
        "vllm-graph-aware"
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    async fn health_check(&self) -> Result<()> {
        self.inner.health_check().await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.inner.list_models().await
    }

    fn capabilities(&self) -> apxm_core::types::ModelCapabilities {
        self.inner.capabilities()
    }

    fn metadata(&self) -> serde_json::Value {
        let mut meta = self.inner.metadata();
        if let serde_json::Value::Object(ref mut map) = meta {
            map.insert("backend_type".to_string(), "vllm-graph-aware".into());
            map.insert("apxm_extensions".to_string(), true.into());
        }
        meta
    }
}

/// Arc-wrapped backend for shared use across threads.
pub type SharedGraphAwareVllmBackend = Arc<GraphAwareVllmBackend>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_execution_id() {
        // Test the counter logic directly without constructing the full backend
        let counter = AtomicU64::new(0);

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        let id1 = format!("exec-{}-{}", ts, counter.fetch_add(1, Ordering::Relaxed));
        let id2 = format!("exec-{}-{}", ts, counter.fetch_add(1, Ordering::Relaxed));

        assert!(id1.starts_with("exec-"));
        assert!(id2.starts_with("exec-"));
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_graph_register_response_serde() {
        let response = GraphRegisterResponse {
            object: "apxm.graph.registration".to_string(),
            graph_id: "test-graph".to_string(),
            execution_id: Some("exec-123".to_string()),
            registered_nodes: 5,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("test-graph"));
        assert!(json.contains("registered_nodes"));
    }
}
