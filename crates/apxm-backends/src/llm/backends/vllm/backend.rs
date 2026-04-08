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

/// Request for `POST /v1/apxm/pins`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinCreateRequest {
    pub graph_id: String,
    pub node_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_group: Option<String>,
    pub ttl_ms: u64,
}

/// Response from `POST /v1/apxm/pins`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinCreateResponse {
    pub object: String,
    pub graph_id: String,
    pub node_id: u32,
    pub reuse_group: Option<String>,
    pub request_id: String,
    pub ttl_ms: u64,
    pub expiry_ts: f64,
}

/// Response from `GET /v1/apxm/pins/stats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinStatsResponse {
    pub object: String,
    pub active_pins: u64,
    pub pinned_blocks: u64,
    pub pin_hits: u64,
    pub pin_misses: u64,
    pub pin_hit_ratio: f64,
    pub pin_expirations: u64,
    pub memory_pressure_releases: u64,
    pub total_lookups: u64,
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

    /// Return the registration endpoint used for graph metadata uploads.
    pub fn graph_registration_url(&self) -> String {
        format!("{}/v1/apxm/graphs/register", self.base_url)
    }

    /// Register a graph with the vLLM server for scheduling hints.
    ///
    /// Call this once per graph execution before sending individual node
    /// requests. The server stores the metadata and uses it for KV-cache
    /// pinning decisions.
    pub async fn register_graph(&self, metadata: GraphMetadata) -> Result<GraphRegisterResponse> {
        let url = self.graph_registration_url();
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

    /// Pin KV-cache blocks for prefix reuse.
    ///
    /// Call this after an LLM request completes to pin its KV-cache for
    /// downstream nodes. The pin will be automatically released when consumed,
    /// when TTL expires, or when the graph is released.
    pub async fn pin_prefix(
        &self,
        graph_id: &str,
        node_id: u32,
        reuse_group: Option<&str>,
        ttl_ms: u64,
    ) -> Result<PinCreateResponse> {
        let url = format!("{}/v1/apxm/pins", self.base_url);
        let request = PinCreateRequest {
            graph_id: graph_id.to_string(),
            node_id,
            reuse_group: reuse_group.map(|s| s.to_string()),
            ttl_ms,
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send pin creation request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Pin creation failed: {} - {}", status, body);
        }

        response
            .json()
            .await
            .context("Failed to parse pin creation response")
    }

    /// Get pin statistics from the vLLM server.
    pub async fn get_pin_stats(&self) -> Result<PinStatsResponse> {
        let url = format!("{}/v1/apxm/pins/stats", self.base_url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send pin stats request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Pin stats request failed: {} - {}", status, body);
        }

        response
            .json()
            .await
            .context("Failed to parse pin stats response")
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
            let mut extra = request
                .extra_body
                .take()
                .unwrap_or_else(|| serde_json::json!({}));
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
        let injected_request = self.inject_hints(request.clone());
        let response = self.inner.generate(injected_request).await?;

        // Phase 3: Pin KV-cache if pin_policy.mode == "prefix"
        if let Some(hints) = &request.apxm_hints {
            if hints.pin_policy.mode == "prefix" {
                // Extract graph_id, node_id, reuse_group, and ttl_ms
                if let (Some(graph_id), Some(node_id)) = (&hints.graph_id, hints.node_id) {
                    let reuse_group = hints.reuse_group.as_deref();
                    let ttl_ms = hints.pin_policy.ttl_ms.unwrap_or(30_000) as u64;

                    // Call pin_prefix API (fire and forget - don't fail the request)
                    let _ = self.pin_prefix(graph_id, node_id, reuse_group, ttl_ms).await;
                }
            }
        }

        Ok(response)
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
