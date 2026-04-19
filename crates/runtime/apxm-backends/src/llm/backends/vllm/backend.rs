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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// Whether the server exposes the APXM extension endpoints (`/v1/apxm/*`).
    /// Probed on first `health_check`; if probe returns 404, this flips to `false`
    /// and `register_graph`/`pin_prefix`/`release_graph` become silent no-ops.
    apxm_endpoints_available: AtomicBool,
    /// Tracks whether we've already emitted a one-time WARN about missing
    /// APXM endpoints (so we don't spam the log on every health check).
    health_check_warned: AtomicBool,
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
            apxm_endpoints_available: AtomicBool::new(true),
            health_check_warned: AtomicBool::new(false),
        })
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
        // If the server doesn't expose `/v1/apxm/*`, become a silent no-op.
        if !self.apxm_endpoints_available.load(Ordering::Relaxed) {
            return Ok(GraphRegisterResponse {
                object: "apxm.graph.registration".to_string(),
                graph_id: metadata.graph_id.clone(),
                execution_id: metadata.execution_id.clone(),
                registered_nodes: 0,
            });
        }
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
        if !self.apxm_endpoints_available.load(Ordering::Relaxed) {
            return Ok(GraphReleaseResponse {
                object: "apxm.graph.release".to_string(),
                graph_id: graph_id.to_string(),
                released_handles: 0,
                released_blocks: 0,
            });
        }
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
        if !self.apxm_endpoints_available.load(Ordering::Relaxed) {
            return Ok(PinCreateResponse {
                object: "apxm.pin".to_string(),
                graph_id: graph_id.to_string(),
                node_id,
                reuse_group: reuse_group.map(|s| s.to_string()),
                request_id: String::new(),
                ttl_ms,
                expiry_ts: 0.0,
            });
        }
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
                    let _ = self
                        .pin_prefix(graph_id, node_id, reuse_group, ttl_ms)
                        .await;
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
        // First do the standard inner health check.
        self.inner.health_check().await?;

        // Probe the APXM extension surface so callers can rely on
        // `supports_graph_extensions()` reflecting reality. We only flip the
        // flag to false on a definitive 404 — transient failures don't disable
        // the extensions.
        let url = format!("{}/v1/apxm/pins/stats", self.base_url);
        if let Ok(response) = self.client.get(&url).send().await {
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                self.apxm_endpoints_available
                    .store(false, Ordering::Relaxed);
                if !self.health_check_warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        endpoint = %url,
                        "vLLM server does not expose APXM extensions (/v1/apxm/*); \
                         graph registration, prefix pinning, and graph release will be no-ops"
                    );
                }
            }
        }

        Ok(())
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
        }
        meta
    }

    fn supports_graph_extensions(&self) -> bool {
        self.apxm_endpoints_available.load(Ordering::Relaxed)
    }

    fn next_execution_id(&self) -> String {
        let counter = self.execution_counter.fetch_add(1, Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("exec-{}-{}", timestamp, counter)
    }

    async fn register_graph(&self, metadata: serde_json::Value) -> Result<()> {
        let graph_meta: GraphMetadata =
            serde_json::from_value(metadata).context("Failed to deserialize GraphMetadata")?;
        GraphAwareVllmBackend::register_graph(self, graph_meta).await?;
        Ok(())
    }

    async fn release_graph(&self, graph_id: &str) -> Result<()> {
        GraphAwareVllmBackend::release_graph(self, graph_id).await?;
        Ok(())
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

    #[tokio::test]
    async fn test_inject_hints_into_extra_body() {
        use crate::llm::backends::LLMRequest;
        use crate::llm::backends::vllm::ApxmGraphHints;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8000"})),
        )
        .await
        .unwrap();

        let hints = ApxmGraphHints::critical_path(
            "graph-test",
            "exec-test",
            42,
            "test-node",
            vec![43, 44],
            60_000,
        );

        let request = LLMRequest::new("Test prompt").with_apxm_hints(hints.clone());

        let injected = backend.inject_hints(request);

        // Verify extra_body has apxm field
        assert!(injected.extra_body.is_some());
        let extra = injected.extra_body.unwrap();
        assert!(extra.get("apxm").is_some());

        // Verify the serialized hints
        let apxm = &extra["apxm"];
        assert_eq!(apxm["schema_version"], 1);
        assert_eq!(apxm["graph_id"], "graph-test");
        assert_eq!(apxm["execution_id"], "exec-test");
        assert_eq!(apxm["node_id"], 42);
        assert_eq!(apxm["node_name"], "test-node");
        assert_eq!(apxm["priority_class"], "critical_path");
        assert_eq!(apxm["downstream_nodes"], serde_json::json!([43, 44]));
    }

    #[tokio::test]
    async fn test_inject_hints_preserves_existing_extra_body() {
        use crate::llm::backends::LLMRequest;
        use crate::llm::backends::vllm::ApxmGraphHints;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8000"})),
        )
        .await
        .unwrap();

        let hints = ApxmGraphHints::parallel("graph-1", "exec-1", 10, "parallel-node");

        // Create request with existing extra_body
        let existing_extra = serde_json::json!({
            "custom_field": "custom_value",
            "another_field": 123
        });

        let request = LLMRequest::new("Test")
            .with_apxm_hints(hints)
            .with_extra_body(existing_extra);

        let injected = backend.inject_hints(request);

        let extra = injected.extra_body.unwrap();

        // Verify both existing fields and new apxm field are present
        assert_eq!(extra["custom_field"], "custom_value");
        assert_eq!(extra["another_field"], 123);
        assert!(extra.get("apxm").is_some());
        assert_eq!(extra["apxm"]["node_id"], 10);
    }

    #[tokio::test]
    async fn test_inject_hints_skips_if_already_present() {
        use crate::llm::backends::LLMRequest;
        use crate::llm::backends::vllm::ApxmGraphHints;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8000"})),
        )
        .await
        .unwrap();

        let hints = ApxmGraphHints::default();

        // Create request with apxm already in extra_body
        let existing_extra = serde_json::json!({
            "apxm": {
                "already": "present"
            }
        });

        let request = LLMRequest::new("Test")
            .with_apxm_hints(hints)
            .with_extra_body(existing_extra.clone());

        let injected = backend.inject_hints(request);

        let extra = injected.extra_body.unwrap();

        // Verify original apxm is preserved (not overwritten)
        assert_eq!(extra["apxm"]["already"], "present");
    }

    #[tokio::test]
    async fn test_supports_graph_extensions_flag() {
        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8000"})),
        )
        .await
        .unwrap();

        // A freshly-constructed backend optimistically claims support; the flag
        // is only flipped to false on a definitive 404 from health_check.
        assert!(backend.supports_graph_extensions());

        // Simulate the health_check seeing a 404 on /v1/apxm/pins/stats.
        backend
            .apxm_endpoints_available
            .store(false, Ordering::Relaxed);
        assert!(!backend.supports_graph_extensions());
    }
}
