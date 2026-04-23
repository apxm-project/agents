//! Graph-aware vLLM backend.
//!
//! This backend wraps the OpenAI-compatible vLLM server and adds APXM graph
//! hints to every request. It also provides methods to register, inspect, and
//! release graphs on the server side for graph-aware scheduling state.

use crate::llm::backends::openai::OpenAIBackend;
use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse};
use anyhow::{Context, Result};
use apxm_core::constants::llm::{api_paths, apxm as apxm_llm, config_keys, vllm as vllm_keys};
use apxm_core::types::{GraphMetadata, ModelInfo, PriorityClass};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio_stream::Stream;

const DEFAULT_BASE_URL: &str = "http://localhost:8000/v1";
const UNCONFIGURED_MODEL_SENTINEL: &str = "__apxm_vllm_model_required__";
const VLLM_PRIORITY_CRITICAL_PATH: u8 = 0;
const VLLM_PRIORITY_DEFAULT: u8 = 5;
const VLLM_PRIORITY_LEGACY_SPECULATIVE: u8 = 10;

/// Response from `POST /v1/apxm/graphs/register`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRegisterResponse {
    pub object: String,
    pub graph_id: String,
    pub execution_id: Option<String>,
    pub registered_nodes: u32,
    pub critical_path_length: Option<u32>,
    pub max_parallelism: Option<u32>,
    pub default_pin_ttl_ms: Option<u32>,
}

/// Response from `DELETE /v1/apxm/graphs/{graph_id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphReleaseResponse {
    pub object: String,
    pub graph_id: String,
    pub released_handles: u32,
    pub released_blocks: u32,
    pub remaining_handles: Option<u32>,
    pub remaining_blocks: Option<u32>,
}

/// Response from `GET /v1/apxm/graphs/{graph_id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStatusResponse {
    pub object: String,
    pub graph_id: String,
    pub registered: bool,
    pub pinned_handles: u64,
    pub pinned_blocks: u64,
    pub node_count: Option<u64>,
    pub critical_path_length: Option<u64>,
}

/// Graph-aware vLLM backend.
///
/// Wraps an OpenAI-compatible vLLM server and injects APXM graph hints into
/// each request's `extra_body.apxm` field, enabling:
///
/// - Critical-path priority scheduling
/// - Graph-aware KV retention via per-request hinting
/// - Graph-level pin TTL defaults
/// - Eager prefill hints
///
/// # Example
///
/// ```ignore
/// let backend = GraphAwareVllmBackend::new("", Some(json!({
///     "base_url": "http://vllm-server:8000/v1",
///     "model": "meta-llama/Llama-3.1-8B-Instruct"
/// }))).await?;
///
/// // Register graph before sending requests
/// backend.register_graph(GraphMetadata::new("graph-123", "exec-abc")).await?;
///
/// // Send request with hints
/// let request = LLMRequest::new("Hello")
///     .with_apxm_hints(ApxmGraphHints::critical_path(
///         "graph-123", "exec-abc", 1, "greet", vec![2], 30_000
///     ));
/// let response = backend.generate(request).await?;
///
/// // Release graph when done
/// backend.release_graph("graph-123").await?;
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
    /// and graph extension calls become silent no-ops.
    apxm_endpoints_available: AtomicBool,
    /// Tracks whether we've already emitted a one-time WARN about missing
    /// APXM endpoints (so we don't spam the log on every health check).
    health_check_warned: AtomicBool,
    /// Whether the server accepts `tool_choice="auto"`. Driven by
    /// `BackendConfig.auto_tool_choice` (TOML `auto_tool_choice = false` in
    /// `~/.apxm/config.toml`). Default `true`. Stock vLLM without
    /// `--enable-auto-tool-choice` should configure this to `false`.
    auto_tool_choice_supported: AtomicBool,
    /// Whether the backend config included a concrete default model.
    default_model_configured: bool,
    /// Models that should suppress vLLM chat-template thinking output.
    non_thinking_models: HashSet<String>,
}

impl GraphAwareVllmBackend {
    /// Create a new graph-aware vLLM backend.
    ///
    /// Config keys:
    /// - `base_url`: vLLM server URL including `/v1` (default: `http://localhost:8000/v1`)
    /// - `model`: Model name to use
    /// - `extra_headers`: Optional HTTP headers
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        let default_model_configured = config
            .as_ref()
            .and_then(|c| c.get("model"))
            .and_then(|m| m.as_str())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);

        let base_url = config
            .as_ref()
            .and_then(|c| c.get("base_url"))
            .and_then(|u| u.as_str())
            .unwrap_or(DEFAULT_BASE_URL)
            .trim_end_matches('/')
            .to_string();

        // `auto_tool_choice` defaults to `true` (matches OpenAI/Anthropic).
        // Stock vLLM without `--enable-auto-tool-choice` should set this to
        // `false` in `~/.apxm/config.toml` so the runtime won't send
        // `tool_choice="auto"` and trigger a 400.
        let auto_tool_choice = config
            .as_ref()
            .and_then(|c| c.get(config_keys::AUTO_TOOL_CHOICE))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let non_thinking_models: HashSet<String> = config
            .as_ref()
            .and_then(|c| c.get(config_keys::MODELS))
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|entry| {
                        let id = entry.get("id").and_then(|v| v.as_str())?;
                        let supports = entry
                            .get(config_keys::SUPPORTS_THINKING)
                            .and_then(|v| v.as_bool())?;
                        if supports { None } else { Some(id.to_string()) }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut inner_config_map = config
            .clone()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        if !default_model_configured {
            inner_config_map.insert(
                "model".to_string(),
                serde_json::Value::String(UNCONFIGURED_MODEL_SENTINEL.to_string()),
            );
        }

        // Pass config to inner OpenAI backend (vLLM is OpenAI-compatible)
        let inner =
            OpenAIBackend::new(api_key, Some(serde_json::Value::Object(inner_config_map))).await?;
        let client = reqwest::Client::new();

        Ok(Self {
            inner,
            base_url,
            client,
            execution_counter: AtomicU64::new(0),
            apxm_endpoints_available: AtomicBool::new(true),
            health_check_warned: AtomicBool::new(false),
            auto_tool_choice_supported: AtomicBool::new(auto_tool_choice),
            default_model_configured,
            non_thinking_models,
        })
    }

    /// Return the registration endpoint used for graph metadata uploads.
    ///
    /// Convention: `base_url` already includes the `/v1` prefix, so the
    /// resulting wire URL is `{base}/apxm/graphs/register` which resolves
    /// to `/v1/apxm/graphs/register` on the server.
    pub fn graph_registration_url(&self) -> String {
        format!("{}{}", self.base_url, api_paths::APXM_GRAPHS_REGISTER)
    }

    /// Return the graph-status endpoint for one graph id.
    pub fn graph_status_url(&self, graph_id: &str) -> String {
        format!("{}{}/{}", self.base_url, api_paths::APXM_GRAPHS, graph_id)
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
                object: apxm_llm::OBJECT_GRAPH_REGISTRATION.to_string(),
                graph_id: metadata.graph_id.clone(),
                execution_id: metadata.execution_id.clone(),
                registered_nodes: 0,
                critical_path_length: metadata.critical_path_length,
                max_parallelism: metadata.max_parallelism,
                default_pin_ttl_ms: metadata.default_pin_ttl_ms,
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
                object: apxm_llm::OBJECT_GRAPH_RELEASE.to_string(),
                graph_id: graph_id.to_string(),
                released_handles: 0,
                released_blocks: 0,
                remaining_handles: None,
                remaining_blocks: None,
            });
        }
        let url = self.graph_status_url(graph_id);
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

    /// Get the current status for a registered graph.
    pub async fn get_graph_status(&self, graph_id: &str) -> Result<GraphStatusResponse> {
        if !self.apxm_endpoints_available.load(Ordering::Relaxed) {
            return Ok(GraphStatusResponse {
                object: apxm_llm::OBJECT_GRAPH_STATUS.to_string(),
                graph_id: graph_id.to_string(),
                registered: false,
                pinned_handles: 0,
                pinned_blocks: 0,
                node_count: None,
                critical_path_length: None,
            });
        }
        let url = self.graph_status_url(graph_id);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send graph status request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Graph status request failed: {} - {}", status, body);
        }

        response
            .json()
            .await
            .context("Failed to parse graph status response")
    }

    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request
            .model
            .as_deref()
            .unwrap_or_else(|| self.inner.model())
    }

    /// Inject graph-aware vLLM request shaping into the provider-neutral request.
    fn inject_hints(&self, mut request: LLMRequest) -> LLMRequest {
        let mut extra = request
            .extra_body
            .take()
            .unwrap_or_else(|| serde_json::json!({}));

        if !extra.is_object() {
            request.extra_body = Some(extra);
            return request;
        }

        if let serde_json::Value::Object(ref mut map) = extra {
            if let Some(ref hints) = request.apxm_hints {
                if !map.contains_key("apxm") {
                    map.insert(
                        "apxm".to_string(),
                        serde_json::to_value(hints).unwrap_or_default(),
                    );
                }

                if !map.contains_key("priority")
                    && let Some(priority_class) = &hints.priority_class
                {
                    let priority = match priority_class {
                        PriorityClass::CriticalPath => VLLM_PRIORITY_CRITICAL_PATH,
                        PriorityClass::Parallel => VLLM_PRIORITY_DEFAULT,
                        PriorityClass::Speculative => VLLM_PRIORITY_LEGACY_SPECULATIVE,
                    };
                    map.insert("priority".to_string(), serde_json::json!(priority));
                }
            }

            let model = self.request_model(&request);
            if self.non_thinking_models.contains(model)
                && !map.contains_key(config_keys::CHAT_TEMPLATE_KWARGS)
            {
                map.insert(
                    config_keys::CHAT_TEMPLATE_KWARGS.to_string(),
                    serde_json::json!({
                        config_keys::ENABLE_THINKING: false,
                    }),
                );
            }
        }

        request.extra_body = Some(extra);
        request
    }

    fn ensure_model_selected(&self, request: &LLMRequest) -> Result<()> {
        if request.model.is_none() && !self.default_model_configured {
            anyhow::bail!(
                "No model is configured for this vLLM backend. \
Register one with `dekk apxm backend add-model <backend> <model-id>` \
or set an explicit graph/default model before execution."
            );
        }
        Ok(())
    }
}

#[async_trait]
impl LLMBackend for GraphAwareVllmBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        self.ensure_model_selected(&request)?;
        let injected_request = self.inject_hints(request);
        self.inner.generate(injected_request).await
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        if let Err(error) = self.ensure_model_selected(&request) {
            return Box::pin(futures::stream::once(async move { Err(error) }));
        }
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
        let url = self.graph_status_url(vllm_keys::APXM_PROBE_GRAPH_ID);
        if let Ok(response) = self.client.get(&url).send().await {
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                self.apxm_endpoints_available
                    .store(false, Ordering::Relaxed);
                if !self.health_check_warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        endpoint = %url,
                        "vLLM server does not expose APXM extensions (/v1/apxm/*); \
                         graph registration, graph status, and graph release will be no-ops"
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

    fn supports_auto_tool_choice(&self) -> bool {
        self.auto_tool_choice_supported.load(Ordering::Relaxed)
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
            object: apxm_llm::OBJECT_GRAPH_REGISTRATION.to_string(),
            graph_id: "test-graph".to_string(),
            execution_id: Some("exec-123".to_string()),
            registered_nodes: 5,
            critical_path_length: Some(3),
            max_parallelism: Some(2),
            default_pin_ttl_ms: Some(30_000),
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
            Some(serde_json::json!({"base_url": "http://localhost:8000/v1"})),
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
        assert_eq!(extra["priority"], 0);
        assert_eq!(apxm["schema_version"], 1);
        assert_eq!(apxm["graph_id"], "graph-test");
        assert_eq!(apxm["execution_id"], "exec-test");
        assert_eq!(apxm["node_id"], 42);
        assert_eq!(apxm["node_name"], "test-node");
        assert_eq!(apxm["priority_class"], apxm_llm::PRIORITY_CRITICAL_PATH);
        assert_eq!(apxm["downstream_nodes"], serde_json::json!([43, 44]));
    }

    #[tokio::test]
    async fn test_inject_hints_preserves_existing_extra_body() {
        use crate::llm::backends::LLMRequest;
        use crate::llm::backends::vllm::ApxmGraphHints;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8000/v1"})),
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
        assert_eq!(extra["priority"], 5);
        assert!(extra.get("apxm").is_some());
        assert_eq!(extra["apxm"]["node_id"], 10);
    }

    #[tokio::test]
    async fn test_inject_hints_skips_if_already_present() {
        use crate::llm::backends::LLMRequest;
        use crate::llm::backends::vllm::ApxmGraphHints;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8000/v1"})),
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
    async fn test_inject_hints_adds_thinking_suppression_for_flagged_model() {
        use crate::llm::backends::LLMRequest;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({
                "base_url": "http://localhost:8000/v1",
                "model": "Qwen/Qwen3.5-4B",
                "models": [
                    {"id": "Qwen/Qwen3.5-4B", "supports_thinking": false}
                ]
            })),
        )
        .await
        .unwrap();

        let injected = backend.inject_hints(LLMRequest::new("Test prompt"));
        let extra = injected.extra_body.unwrap();
        assert_eq!(
            extra[config_keys::CHAT_TEMPLATE_KWARGS][config_keys::ENABLE_THINKING],
            serde_json::json!(false)
        );
    }

    #[tokio::test]
    async fn test_supports_graph_extensions_flag() {
        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8000/v1"})),
        )
        .await
        .unwrap();

        // A freshly-constructed backend optimistically claims support; the flag
        // is only flipped to false on a definitive 404 from health_check.
        assert!(backend.supports_graph_extensions());

        // Simulate the health_check seeing a missing APXM graph-status route.
        backend
            .apxm_endpoints_available
            .store(false, Ordering::Relaxed);
        assert!(!backend.supports_graph_extensions());
    }

    #[tokio::test]
    async fn test_generate_requires_explicit_model_when_backend_has_no_default_model() {
        use crate::llm::backends::LLMRequest;

        let backend = GraphAwareVllmBackend::new(
            "",
            Some(serde_json::json!({"base_url": "http://localhost:8000/v1"})),
        )
        .await
        .unwrap();

        let error = backend
            .generate(LLMRequest::new("Test prompt"))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("No model is configured for this vLLM backend")
        );
    }
}
