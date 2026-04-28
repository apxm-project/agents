//! Graph-aware vLLM backend.
//!
//! This backend wraps the OpenAI-compatible vLLM server and adds APXM graph
//! hints to every request. It also provides methods to register, inspect, and
//! release graphs on the server side for graph-aware scheduling state.

use crate::llm::backends::openai::OpenAIBackend;
use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse};
use crate::llm::catalog::DEFAULT_VLLM_BASE_URL;
use crate::llm::wire::{api_paths, backend_metadata, config_keys, headers};
use crate::llm::{ProviderProtocol, normalize_endpoint_for_protocol};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::constants::llm::apxm as apxm_llm;
use apxm_core::types::{
    GraphMetadata, GraphStatusSnapshot, ModelCapabilities, ModelInfo, PriorityClass,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio_stream::Stream;

const DEFAULT_BASE_URL: &str = DEFAULT_VLLM_BASE_URL;
const PROTOCOL: ProviderProtocol = ProviderProtocol::Vllm;
const UNCONFIGURED_MODEL_SENTINEL: &str = "__apxm_vllm_model_required__";

mod request_keys {
    pub const THINKING_TOKEN_BUDGET: &str = "thinking_token_budget";
    pub const STRUCTURED_OUTPUTS: &str = "structured_outputs";
    pub const STRUCTURED_OUTPUT_JSON: &str = "json";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VllmRequestPriority {
    CriticalPath,
    Default,
}

impl From<PriorityClass> for VllmRequestPriority {
    fn from(value: PriorityClass) -> Self {
        match value {
            PriorityClass::CriticalPath => Self::CriticalPath,
            PriorityClass::Parallel => Self::Default,
        }
    }
}

impl From<VllmRequestPriority> for u8 {
    fn from(value: VllmRequestPriority) -> Self {
        match value {
            VllmRequestPriority::CriticalPath => 0,
            VllmRequestPriority::Default => 5,
        }
    }
}

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

/// Response from `GET /v1/apxm/scheduler`.
///
/// The fork's critical-path boost (which rewrites a request's priority to -1)
/// is only consulted when `policy == "priority"`. If the running fork is in
/// FCFS mode, APXM-stamped hints round-trip without effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerInfoResponse {
    pub object: String,
    pub policy: String,
    pub default: Option<String>,
}

/// Graph-aware vLLM backend.
///
/// Wraps an OpenAI-compatible vLLM server and injects APXM graph hints
/// into each request via `vllm_xargs.apxm`.
pub struct GraphAwareVllmBackend {
    /// Inner OpenAI-compatible backend for actual requests.
    inner: OpenAIBackend,
    /// Base URL for APXM-specific endpoints (`/v1/apxm/...`).
    base_url: String,
    /// HTTP client for graph management endpoints.
    client: reqwest::Client,
    /// Counter for generating unique execution IDs.
    execution_counter: AtomicU64,
    /// Whether the server exposes `/v1/apxm/*`. Probed on `health_check`.
    apxm_endpoints_available: AtomicBool,
    /// Whether the server accepts `tool_choice="auto"`. Default `true`.
    auto_tool_choice_supported: AtomicBool,
    /// Whether the backend config included a concrete default model.
    default_model_configured: bool,
    /// Whether vLLM should receive native `structured_outputs`.
    structured_outputs_supported: bool,
    /// One-shot guard so the FCFS-policy WARN only fires once per backend.
    /// Set to `true` after the first time `health_check` reports a non-priority
    /// policy or a missing `/v1/apxm/scheduler` endpoint.
    scheduler_policy_warned: AtomicBool,
}

impl GraphAwareVllmBackend {
    /// Create a new graph-aware vLLM backend.
    ///
    /// Config keys:
    /// - `base_url`: vLLM server URL including `/v1` (default: `http://localhost:8916/v1`)
    /// - `model`: Model name to use
    /// - `extra_headers`: Optional HTTP headers
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        let default_model_configured = config
            .as_ref()
            .and_then(|c| c.get(MODEL))
            .and_then(|m| m.as_str())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);

        let base_url = config
            .as_ref()
            .and_then(|c| c.get(BASE_URL))
            .and_then(|u| u.as_str())
            .unwrap_or(DEFAULT_BASE_URL)
            .to_string();
        let base_url = normalize_endpoint_for_protocol(PROTOCOL, &base_url);

        let auto_tool_choice = config
            .as_ref()
            .and_then(|c| c.get(config_keys::AUTO_TOOL_CHOICE))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let structured_outputs_supported = config
            .as_ref()
            .and_then(|c| c.get(config_keys::SUPPORTS_STRUCTURED_OUTPUTS))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut inner_config_map = config
            .clone()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        if !default_model_configured {
            inner_config_map.insert(
                MODEL.to_string(),
                serde_json::Value::String(UNCONFIGURED_MODEL_SENTINEL.to_string()),
            );
        }
        inner_config_map.insert(
            config_keys::SUPPORTS_STRUCTURED_OUTPUTS.to_string(),
            serde_json::Value::Bool(false),
        );

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
            auto_tool_choice_supported: AtomicBool::new(auto_tool_choice),
            default_model_configured,
            structured_outputs_supported,
            scheduler_policy_warned: AtomicBool::new(false),
        })
    }

    /// URL for the scheduler-info probe (`GET /v1/apxm/scheduler`).
    fn scheduler_info_url(&self) -> String {
        format!("{}{}", self.base_url, api_paths::APXM_SCHEDULER)
    }

    /// Probe `/v1/apxm/scheduler` and warn loudly once if the fork is not in
    /// priority mode. Hints are still sent so the per-request `vllm_xargs.apxm`
    /// payload (graph_id, pin_policy, etc.) keeps reaching the scheduler — only
    /// the priority field is inert under FCFS, which the operator needs to know.
    async fn probe_scheduler_policy(&self) {
        // Cheap short-circuit: once we've warned, don't re-probe every health tick.
        if self.scheduler_policy_warned.load(Ordering::Relaxed) {
            return;
        }

        let url = self.scheduler_info_url();
        let response = self
            .inner
            .apply_transport_headers(self.client.get(&url))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let info = resp.json::<SchedulerInfoResponse>().await;
                match info {
                    Ok(info) if info.policy == super::graph_meta::SCHEDULER_POLICY_PRIORITY => {
                        // Priority mode is active — APXM hints will reorder admission.
                        // Nothing to log; keep `scheduler_policy_warned` false so we
                        // re-check if a future health tick sees a different policy.
                    }
                    Ok(info) => {
                        if !self.scheduler_policy_warned.swap(true, Ordering::Relaxed) {
                            tracing::warn!(
                                policy = %info.policy,
                                expected = super::graph_meta::SCHEDULER_POLICY_PRIORITY,
                                url = %url,
                                "vLLM scheduler is not in priority mode; APXM critical-path \
                                 hints will round-trip without reordering admission. Restart \
                                 the fork with `--scheduling-policy=priority` (the APXM \
                                 launcher default) to make priority hints take effect."
                            );
                        }
                    }
                    Err(err) => {
                        if !self.scheduler_policy_warned.swap(true, Ordering::Relaxed) {
                            tracing::warn!(
                                error = %err,
                                url = %url,
                                "vLLM /v1/apxm/scheduler returned a malformed body; cannot \
                                 confirm priority hints will be honored."
                            );
                        }
                    }
                }
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
                if !self.scheduler_policy_warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        url = %url,
                        "vLLM fork is missing /v1/apxm/scheduler — likely an older fork \
                         build. Cannot verify scheduler policy; APXM priority hints may be \
                         inert. Rebuild the external/vllm submodule to pick up the \
                         scheduler-info endpoint."
                    );
                }
            }
            Ok(resp) => {
                if !self.scheduler_policy_warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        status = %resp.status(),
                        url = %url,
                        "vLLM scheduler-info probe returned an unexpected status; \
                         priority hint behavior is unverified."
                    );
                }
            }
            Err(err) => {
                if !self.scheduler_policy_warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        error = %err,
                        url = %url,
                        "Failed to probe vLLM /v1/apxm/scheduler; priority hint behavior \
                         is unverified."
                    );
                }
            }
        }
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
        let url = self.graph_registration_url();
        let response = self
            .inner
            .apply_transport_headers(
                self.client
                    .post(&url)
                    .header(headers::CONTENT_TYPE, headers::CONTENT_TYPE_JSON),
            )
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
        let url = self.graph_status_url(graph_id);
        let response = self
            .inner
            .apply_transport_headers(self.client.delete(&url))
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

    /// Get the current status for a registered graph (typed).
    pub async fn get_graph_status_typed(&self, graph_id: &str) -> Result<GraphStatusResponse> {
        let url = self.graph_status_url(graph_id);

        let response = self
            .inner
            .apply_transport_headers(self.client.get(&url))
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
                let vllm_xargs = map
                    .entry(super::graph_meta::REQUEST_XARGS.to_owned())
                    .or_insert_with(|| serde_json::json!({}));
                if let serde_json::Value::Object(vllm_xargs_map) = vllm_xargs {
                    let rendered_hints = serde_json::to_value(hints).unwrap_or_default();
                    match vllm_xargs_map.get_mut(apxm_llm::HINTS_FIELD) {
                        Some(existing) => match (existing, rendered_hints) {
                            (
                                serde_json::Value::Object(existing_map),
                                serde_json::Value::Object(rendered_map),
                            ) => {
                                for (key, value) in rendered_map {
                                    existing_map.insert(key, value);
                                }
                            }
                            (slot, rendered_hints) => {
                                *slot = rendered_hints;
                            }
                        },
                        None => {
                            vllm_xargs_map.insert(apxm_llm::HINTS_FIELD.to_owned(), rendered_hints);
                        }
                    }
                }

                if !map.contains_key(apxm_llm::REQUEST_PRIORITY)
                    && let Some(priority_class) = &hints.priority_class
                {
                    let priority = u8::from(VllmRequestPriority::from(*priority_class));
                    map.insert(
                        apxm_llm::REQUEST_PRIORITY.to_owned(),
                        serde_json::json!(priority),
                    );
                }
            }

            if self.structured_outputs_supported
                && !map.contains_key(request_keys::STRUCTURED_OUTPUTS)
                && let Some(output_schema) = &request.output_schema
            {
                map.insert(
                    request_keys::STRUCTURED_OUTPUTS.to_string(),
                    serde_json::json!({
                        request_keys::STRUCTURED_OUTPUT_JSON: output_schema,
                    }),
                );
            }

            if let Some(thinking_budget) = request.thinking_token_budget
                && !map.contains_key(request_keys::THINKING_TOKEN_BUDGET)
            {
                map.insert(
                    request_keys::THINKING_TOKEN_BUDGET.to_string(),
                    serde_json::json!(thinking_budget),
                );
            }
            if let Some(enable_thinking) = request.enable_thinking {
                let kwargs = map
                    .entry(config_keys::CHAT_TEMPLATE_KWARGS.to_string())
                    .or_insert_with(|| serde_json::json!({}));
                if let serde_json::Value::Object(kwargs_map) = kwargs
                    && !kwargs_map.contains_key(config_keys::ENABLE_THINKING)
                {
                    kwargs_map.insert(
                        config_keys::ENABLE_THINKING.to_string(),
                        serde_json::json!(enable_thinking),
                    );
                }
            }
        }

        request.extra_body = Some(extra);
        request
    }

    fn ensure_model_selected(&self, request: &LLMRequest) -> Result<()> {
        if request.model.is_none() && !self.default_model_configured {
            anyhow::bail!(
                "No model is configured for this vLLM backend. \
Register a model on the backend configuration or set an explicit graph/default model before execution."
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
        super::graph_meta::BACKEND_NAME
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    async fn health_check(&self) -> Result<()> {
        // First do the standard inner health check.
        self.inner.health_check().await?;

        // Probe the APXM extension surface. A definitive 404 means the server
        // is stock vLLM, not the APXM fork. APXM requires the graph-aware
        // contract for `protocol = "vllm"` so scheduling hints cannot be
        // silently ignored.
        let url = self.graph_status_url(super::graph_meta::PROBE_GRAPH_ID);
        match self
            .inner
            .apply_transport_headers(self.client.get(&url))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                self.apxm_endpoints_available.store(true, Ordering::Relaxed);
            }
            Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
                self.apxm_endpoints_available
                    .store(false, Ordering::Relaxed);
                anyhow::bail!(
                    "vLLM server at {} does not expose /v1/apxm/* endpoints. \
                     This is stock vLLM, which silently drops vllm_xargs.apxm \
                     scheduling hints. Install and run the graph-aware fork.",
                    url
                );
            }
            Ok(response) => {
                let status = response.status();
                self.apxm_endpoints_available
                    .store(false, Ordering::Relaxed);
                anyhow::bail!(
                    "vLLM server at {} exposes the APXM graph route but it is not ready \
                     (status {}). Check the fork server logs and rerun the vLLM probe.",
                    url,
                    status
                );
            }
            Err(err) => {
                self.apxm_endpoints_available
                    .store(false, Ordering::Relaxed);
                return Err(err)
                    .context("failed to probe vLLM APXM graph route during health_check");
            }
        }

        // Probe the scheduler-policy endpoint so the operator gets a loud
        // warning if the fork is running in FCFS mode (in which case
        // APXM-stamped per-request priorities are ignored). The probe is
        // best-effort and warn-once: it never aborts health_check, because
        // vllm_xargs.apxm hints (graph_id, pin_policy, …) remain useful for
        // KV pinning regardless of scheduler policy.
        self.probe_scheduler_policy().await;

        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.inner.list_models().await
    }

    fn capabilities(&self) -> apxm_core::types::ModelCapabilities {
        let mut capabilities: ModelCapabilities = self.inner.capabilities();
        capabilities.structured_outputs = self.structured_outputs_supported;
        capabilities
    }

    fn metadata(&self) -> serde_json::Value {
        let mut meta = self.inner.metadata();
        if let serde_json::Value::Object(ref mut map) = meta {
            map.insert(
                backend_metadata::BACKEND_TYPE.to_string(),
                super::graph_meta::BACKEND_NAME.into(),
            );
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

    async fn register_graph(&self, metadata: GraphMetadata) -> Result<()> {
        GraphAwareVllmBackend::register_graph(self, metadata).await?;
        Ok(())
    }

    async fn release_graph(&self, graph_id: &str) -> Result<()> {
        GraphAwareVllmBackend::release_graph(self, graph_id).await?;
        Ok(())
    }

    async fn get_graph_status(
        &self,
        graph_id: &str,
    ) -> anyhow::Result<Option<GraphStatusSnapshot>> {
        if !self.apxm_endpoints_available.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match self.get_graph_status_typed(graph_id).await {
            Ok(status) => Ok(Some(
                GraphStatusSnapshot::graph_aware(status.graph_id)
                    .with_registered(status.registered)
                    .with_pin_counts(status.pinned_handles, status.pinned_blocks)
                    .with_shape(status.node_count, status.critical_path_length),
            )),
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::backends::vllm::graph_meta;

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
            Some(serde_json::json!({"base_url": "http://localhost:8916/v1"})),
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

        // Verify extra_body has vLLM extra args with APXM hints.
        assert!(injected.extra_body.is_some());
        let extra = injected.extra_body.unwrap();
        assert!(
            extra
                .get(graph_meta::REQUEST_XARGS)
                .and_then(|value| value.get(apxm_llm::HINTS_FIELD))
                .is_some()
        );

        // Verify the serialized hints
        let apxm = &extra[graph_meta::REQUEST_XARGS][apxm_llm::HINTS_FIELD];
        assert_eq!(
            extra[apxm_llm::REQUEST_PRIORITY],
            u8::from(VllmRequestPriority::CriticalPath)
        );
        assert_eq!(apxm[apxm_llm::SCHEMA_VERSION], 1);
        assert_eq!(apxm[apxm_llm::GRAPH_ID], "graph-test");
        assert_eq!(apxm[apxm_llm::EXECUTION_ID], "exec-test");
        assert_eq!(apxm[apxm_llm::NODE_ID], 42);
        assert_eq!(apxm[apxm_llm::NODE_NAME], "test-node");
        assert_eq!(
            apxm[apxm_llm::PRIORITY_CLASS],
            apxm_llm::PRIORITY_CRITICAL_PATH
        );
        assert_eq!(
            apxm[apxm_llm::DOWNSTREAM_NODES],
            serde_json::json!([43, 44])
        );
    }

    #[tokio::test]
    async fn test_inject_hints_preserves_existing_extra_body() {
        use crate::llm::backends::LLMRequest;
        use crate::llm::backends::vllm::ApxmGraphHints;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8916/v1"})),
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

        // Verify both existing fields and new APXM hint field are present.
        assert_eq!(extra["custom_field"], "custom_value");
        assert_eq!(extra["another_field"], 123);
        assert_eq!(
            extra[apxm_llm::REQUEST_PRIORITY],
            u8::from(VllmRequestPriority::Default)
        );
        assert!(
            extra
                .get(graph_meta::REQUEST_XARGS)
                .and_then(|value| value.get(apxm_llm::HINTS_FIELD))
                .is_some()
        );
        assert_eq!(
            extra[graph_meta::REQUEST_XARGS][apxm_llm::HINTS_FIELD][apxm_llm::NODE_ID],
            10
        );
    }

    #[tokio::test]
    async fn test_inject_hints_merges_existing_apxm_field_runtime_wins() {
        use crate::llm::backends::LLMRequest;
        use crate::llm::backends::vllm::ApxmGraphHints;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8916/v1"})),
        )
        .await
        .unwrap();

        let hints = ApxmGraphHints::parallel("graph-fresh", "exec-fresh", 99, "fresh-node");

        let existing_key = "already";
        let existing_value = "present";
        let existing_extra = {
            let mut hints_map = serde_json::Map::new();
            hints_map.insert(existing_key.to_owned(), serde_json::json!(existing_value));
            hints_map.insert(apxm_llm::NODE_ID.to_owned(), serde_json::json!(1));
            let mut vllm_xargs_map = serde_json::Map::new();
            vllm_xargs_map.insert(
                apxm_llm::HINTS_FIELD.to_owned(),
                serde_json::Value::Object(hints_map),
            );
            let mut extra_map = serde_json::Map::new();
            extra_map.insert(
                graph_meta::REQUEST_XARGS.to_owned(),
                serde_json::Value::Object(vllm_xargs_map),
            );
            serde_json::Value::Object(extra_map)
        };

        let request = LLMRequest::new("Test")
            .with_apxm_hints(hints)
            .with_extra_body(existing_extra.clone());

        let injected = backend.inject_hints(request);

        let extra = injected.extra_body.unwrap();

        // Verify unknown caller fields are preserved, while APXM-owned runtime
        // fields win over stale embedded values.
        assert_eq!(
            extra[graph_meta::REQUEST_XARGS][apxm_llm::HINTS_FIELD][existing_key],
            existing_value
        );
        assert_eq!(
            extra[graph_meta::REQUEST_XARGS][apxm_llm::HINTS_FIELD][apxm_llm::NODE_ID],
            99
        );
        assert_eq!(
            extra[graph_meta::REQUEST_XARGS][apxm_llm::HINTS_FIELD][apxm_llm::GRAPH_ID],
            "graph-fresh"
        );
    }

    #[tokio::test]
    async fn test_inject_hints_adds_thinking_controls() {
        use crate::llm::backends::LLMRequest;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({
                "base_url": "http://localhost:8916/v1",
                "model": "served-test-model"
            })),
        )
        .await
        .unwrap();

        let injected = backend.inject_hints(
            LLMRequest::new("Think")
                .with_thinking_token_budget(1024)
                .with_enable_thinking(true),
        );
        let extra = injected.extra_body.unwrap();

        assert_eq!(extra[request_keys::THINKING_TOKEN_BUDGET], 1024);
        assert_eq!(
            extra[config_keys::CHAT_TEMPLATE_KWARGS][config_keys::ENABLE_THINKING],
            serde_json::json!(true)
        );
    }

    #[tokio::test]
    async fn test_inject_hints_lowers_output_schema_to_vllm_structured_outputs() {
        use crate::llm::backends::LLMRequest;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({
                "base_url": "http://localhost:8916/v1",
                "model": "test-model"
            })),
        )
        .await
        .unwrap();

        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {"type": "string"}
            },
            "required": ["summary"]
        });
        let injected =
            backend.inject_hints(LLMRequest::new("Return JSON").with_output_schema(schema.clone()));
        let extra = injected.extra_body.unwrap();

        assert_eq!(
            extra[request_keys::STRUCTURED_OUTPUTS][request_keys::STRUCTURED_OUTPUT_JSON],
            schema
        );
    }

    #[tokio::test]
    async fn test_inject_hints_respects_structured_outputs_capability() {
        use crate::llm::backends::LLMRequest;

        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({
                "base_url": "http://localhost:8916/v1",
                "model": "test-model",
                config_keys::SUPPORTS_STRUCTURED_OUTPUTS: false
            })),
        )
        .await
        .unwrap();

        let injected = backend.inject_hints(
            LLMRequest::new("Return JSON")
                .with_output_schema(serde_json::json!({"type": "object"})),
        );
        let extra = injected.extra_body.unwrap();

        assert!(extra.get(request_keys::STRUCTURED_OUTPUTS).is_none());
    }

    #[tokio::test]
    async fn test_capabilities_report_vllm_structured_outputs_policy() {
        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({
                BASE_URL: "http://localhost:8916/v1",
                MODEL: "test-model"
            })),
        )
        .await
        .unwrap();

        assert!(backend.capabilities().structured_outputs);

        let disabled = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({
                BASE_URL: "http://localhost:8916/v1",
                MODEL: "test-model",
                config_keys::SUPPORTS_STRUCTURED_OUTPUTS: false
            })),
        )
        .await
        .unwrap();

        assert!(!disabled.capabilities().structured_outputs);
    }

    #[tokio::test]
    async fn test_supports_graph_extensions_flag() {
        let backend = GraphAwareVllmBackend::new(
            "test-key",
            Some(serde_json::json!({"base_url": "http://localhost:8916/v1"})),
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
            Some(serde_json::json!({"base_url": "http://localhost:8916/v1"})),
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
