//! Graph-aware vLLM backend.
//!
//! This backend wraps the OpenAI-compatible vLLM server and adds APXM graph
//! hints to every request. It also provides methods to register, inspect, and
//! release graphs on the server side for graph-aware scheduling state.

use crate::llm::backends::http::llm_http_client;
use crate::llm::backends::openai::OpenAIBackend;
use crate::llm::backends::openai::backend::validate_provider_dispatch;
use crate::llm::backends::required_config_string;
use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse};
use crate::llm::wire::{api_paths, backend_metadata, config_keys, headers};
use crate::llm::{ProviderProtocol, normalize_endpoint_for_protocol};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::constants::llm::apxm as apxm_llm;
use apxm_core::types::{
    BackendGraphCapabilities, GraphMetadata, GraphStatusSnapshot, ModelCapabilities, ModelInfo,
    PriorityClass,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio_stream::Stream;

const PROTOCOL: ProviderProtocol = ProviderProtocol::Vllm;

/// Env var that, when set to `"1"`, suppresses APXM-specific request shaping
/// and graph registration so the backend behaves as a flat-HTTP control arm.
const DISABLE_HINTS_ENV: &str = "APXM_DISABLE_HINTS";
const DISABLE_HINTS_ENV_ENABLED: &str = "1";

/// Marker placed in `GraphRegisterResponse.object` when the wire send is
/// suppressed by `DISABLE_HINTS_ENV`, so logs and any consumer can
/// distinguish a suppressed registration from a real one.
const SUPPRESSED_REGISTRATION_OBJECT: &str = "apxm.disabled";

fn apxm_disable_hints() -> bool {
    static DISABLED: OnceLock<bool> = OnceLock::new();
    *DISABLED.get_or_init(|| {
        std::env::var(DISABLE_HINTS_ENV).ok().as_deref() == Some(DISABLE_HINTS_ENV_ENABLED)
    })
}

/// Env var selecting a single APXM mechanism for isolation experiments.
/// Values: `priority`, `prefix`, `registration`. Unset (the default) leaves the
/// full envelope untouched, so normal runs are unaffected.
const ISOLATE_ENV: &str = "APXM_ISOLATE";

fn apxm_isolate() -> Option<String> {
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    MODE.get_or_init(|| {
        std::env::var(ISOLATE_ENV)
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| matches!(s.as_str(), "priority" | "prefix" | "registration"))
    })
    .clone()
}

/// Drop the hint-field families not selected by `mode` from a rendered
/// `ApxmGraphHints` object so one mechanism can be measured in isolation.
/// Registration fields (graph/execution/node ids) and structural fields
/// (schema_version, downstream_nodes, compiler_hints) are always retained; the
/// `registration` control mode keeps only those.
fn prune_isolated_hints(value: serde_json::Value, mode: &str) -> serde_json::Value {
    let serde_json::Value::Object(mut map) = value else {
        return value;
    };
    let drop: &[&str] = match mode {
        "priority" => &["reuse_group", "pin_policy", "graph_metrics"],
        "prefix" => &["priority_class"],
        "registration" => &[
            "priority_class",
            "reuse_group",
            "pin_policy",
            "graph_metrics",
        ],
        _ => &[],
    };
    for key in drop {
        map.remove(*key);
    }
    serde_json::Value::Object(map)
}

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
/// Tag the APXM fork advertises in `SchedulerInfoResponse.dispatch_ir_version`
/// to confirm the running router understands the v1 dispatch-IR schema.
/// The fork-side emitter ships the value; APXM gates
/// `supports_dispatch_ir_v1_internal` on it when present.
pub const DISPATCH_IR_V1_VERSION_TAG: &str = "v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerInfoResponse {
    pub object: String,
    pub policy: String,
    pub default: Option<String>,
    /// Set by the APXM fork's `/v1/apxm/scheduler`
    /// handler to advertise the dispatch-IR schema version it
    /// understands (currently `"v1"`). Forward-compatible: older
    /// fork builds omit the field and APXM falls back to "routes
    /// exist" as the gating signal. Once the fork advertises, this
    /// field is the authoritative source.
    #[serde(default)]
    pub dispatch_ir_version: Option<String>,
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
    /// Whether the server accepts `tool_choice="auto"`. Default `true`.
    auto_tool_choice_supported: AtomicBool,
    /// Whether vLLM should receive native `structured_outputs`.
    structured_outputs_supported: bool,
    /// One-shot guard so the FCFS-policy WARN only fires once per backend.
    /// Set to `true` after the first time `health_check` reports a non-priority
    /// policy or a missing `/v1/apxm/scheduler` endpoint.
    scheduler_policy_warned: AtomicBool,
    /// Last successfully observed scheduler policy from `/v1/apxm/scheduler`.
    scheduler_policy: parking_lot::RwLock<Option<String>>,
    /// Last successfully observed `dispatch_ir_version` from
    /// `/v1/apxm/scheduler`. `None` when the fork build predates the
    /// dispatch-IR advertising commit, in which case
    /// `supports_dispatch_ir_v1_internal` falls back to "routes exist"
    /// gating (set on successful synchronous probe).
    dispatch_ir_version: parking_lot::RwLock<Option<String>>,
}

impl GraphAwareVllmBackend {
    /// Create a new graph-aware vLLM backend.
    ///
    /// Config keys:
    /// - `base_url`: registered vLLM server URL including `/v1`
    /// - `model`: registered model name
    /// - `extra_headers`: Optional HTTP headers
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        let model = required_config_string(config.as_ref(), PROTOCOL, MODEL)?;
        let base_url = required_config_string(config.as_ref(), PROTOCOL, BASE_URL)?;
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
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .ok_or(
                crate::llm::backends::BackendConfigurationError::MissingConfiguration {
                    protocol: PROTOCOL,
                },
            )?;
        inner_config_map.insert(MODEL.to_string(), serde_json::Value::String(model));
        inner_config_map.insert(
            BASE_URL.to_string(),
            serde_json::Value::String(base_url.clone()),
        );
        inner_config_map.insert(
            config_keys::SUPPORTS_STRUCTURED_OUTPUTS.to_string(),
            serde_json::Value::Bool(false),
        );

        // Pass config to inner OpenAI backend (vLLM is OpenAI-compatible)
        let inner =
            OpenAIBackend::new(api_key, Some(serde_json::Value::Object(inner_config_map))).await?;
        let client = llm_http_client();

        Ok(Self {
            inner,
            base_url,
            client,
            execution_counter: AtomicU64::new(0),
            auto_tool_choice_supported: AtomicBool::new(auto_tool_choice),
            structured_outputs_supported,
            scheduler_policy_warned: AtomicBool::new(false),
            scheduler_policy: parking_lot::RwLock::new(None),
            dispatch_ir_version: parking_lot::RwLock::new(None),
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
                        *self.scheduler_policy.write() = Some(info.policy);
                        *self.dispatch_ir_version.write() = info.dispatch_ir_version;
                        // Priority mode is active — APXM hints will reorder admission.
                        // Nothing to log; keep `scheduler_policy_warned` false so we
                        // re-check if a future health tick sees a different policy.
                    }
                    Ok(info) => {
                        *self.scheduler_policy.write() = Some(info.policy.clone());
                        (*self.dispatch_ir_version.write()).clone_from(&info.dispatch_ir_version);
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
                        *self.scheduler_policy.write() = None;
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
                *self.scheduler_policy.write() = None;
                if !self.scheduler_policy_warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        url = %url,
                        "vLLM fork is missing /v1/apxm/scheduler — likely an older fork \
                         build. Cannot verify scheduler policy; APXM priority hints may be \
                         inert. Rebuild the workspace vLLM fork to pick up the \
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
                *self.scheduler_policy.write() = None;
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
        if apxm_disable_hints() {
            return Ok(GraphRegisterResponse {
                object: SUPPRESSED_REGISTRATION_OBJECT.to_string(),
                graph_id: metadata.graph_id.clone(),
                execution_id: metadata.execution_id.clone(),
                registered_nodes: 0,
                critical_path_length: None,
                max_parallelism: None,
                default_pin_ttl_ms: None,
            });
        }
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

        let registered = response
            .json()
            .await
            .context("Failed to parse graph registration response")?;
        self.probe_scheduler_policy().await;
        Ok(registered)
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
            if !apxm_disable_hints()
                && let Some(ref hints) = request.apxm_hints
            {
                let vllm_xargs = map
                    .entry(super::graph_meta::REQUEST_XARGS.to_owned())
                    .or_insert_with(|| serde_json::json!({}));
                if let serde_json::Value::Object(vllm_xargs_map) = vllm_xargs {
                    let rendered_hints = serde_json::to_value(hints).unwrap_or_default();
                    // Mechanism-isolation experiments (APXM_ISOLATE) keep only one
                    // hint family; unset leaves the full envelope unchanged.
                    let rendered_hints = match apxm_isolate() {
                        Some(mode) => prune_isolated_hints(rendered_hints, &mode),
                        None => rendered_hints,
                    };
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
                    && apxm_isolate().is_none_or(|m| m == "priority")
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
}

#[async_trait]
impl LLMBackend for GraphAwareVllmBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        validate_provider_dispatch(&request)?;
        let injected_request = self.inject_hints(request);
        self.inner.generate(injected_request).await
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        if let Err(error) = validate_provider_dispatch(&request) {
            return Box::pin(tokio_stream::iter(vec![Err(error)]));
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

    fn context_window_for_model(&self, model: &str) -> Option<usize> {
        self.inner.context_window_for_model(model)
    }

    async fn health_check(&self) -> Result<()> {
        // First do the standard inner health check.
        self.inner.health_check().await?;

        // Probe the APXM extension surface. A definitive 404 means the server
        // is stock vLLM, not the APXM fork. APXM requires the graph-aware
        // contract for `protocol = "vllm"` so scheduling hints cannot be
        let url = self.graph_status_url(super::graph_meta::PROBE_GRAPH_ID);
        match self
            .inner
            .apply_transport_headers(self.client.get(&url))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
                anyhow::bail!(
                    "vLLM server at {} does not expose /v1/apxm/* endpoints. \
                     This is stock vLLM, which silently drops vllm_xargs.apxm \
                     scheduling hints. Install and run the graph-aware fork, \
                     or register vanilla vLLM under the OpenAIBackend type instead.",
                    url
                );
            }
            Ok(response) => {
                anyhow::bail!(
                    "vLLM server at {} exposes the APXM graph route but it is not ready \
                     (status {}). Check the fork server logs and rerun the vLLM probe.",
                    url,
                    response.status(),
                );
            }
            Err(err) => {
                return Err(err)
                    .context("failed to probe vLLM APXM graph route during health_check");
            }
        }

        // Probe the scheduler-policy endpoint synchronously. A missing
        // `/v1/apxm/scheduler` route means this is not the APXM fork at all
        // (the route is part of the fork patch set). Hard-fail rather than
        // warn — registration must fully verify the graph-aware contract.
        let scheduler_url = self.scheduler_info_url();
        match self
            .inner
            .apply_transport_headers(self.client.get(&scheduler_url))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                if let Ok(body) = response.json::<SchedulerInfoResponse>().await {
                    let policy = body.policy.clone();
                    *self.scheduler_policy.write() = Some(policy);
                    (*self.dispatch_ir_version.write()).clone_from(&body.dispatch_ir_version);
                    // Warn-once on non-priority policy (the manifest disallows
                    // FCFS, so reaching here indicates the fork was started
                    // with the wrong --scheduling-policy flag). Still a soft
                    // warning: per-request `vllm_xargs.apxm` hints continue to
                    // do KV pinning regardless of scheduler policy.
                    if !self.scheduler_policy_warned.swap(true, Ordering::Relaxed) {
                        let policy = self.scheduler_policy.read().clone();
                        if policy.as_deref() != Some(super::graph_meta::SCHEDULER_POLICY_PRIORITY) {
                            tracing::warn!(
                                "vLLM fork at {} reports scheduler policy {:?}; \
                                 APXM ships with priority enabled — per-request \
                                 priority hints are inert outside priority mode.",
                                scheduler_url,
                                policy,
                            );
                        }
                    }
                }
            }
            Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
                anyhow::bail!(
                    "vLLM server at {} is missing /v1/apxm/scheduler. The APXM \
                     fork exposes this route to advertise scheduler policy; its \
                     absence means this is stock vLLM. Install and run the fork.",
                    scheduler_url,
                );
            }
            Ok(response) => {
                anyhow::bail!(
                    "vLLM server at {} returned status {} for /v1/apxm/scheduler. \
                     Check the fork server logs.",
                    scheduler_url,
                    response.status(),
                );
            }
            Err(err) => {
                return Err(err)
                    .context("failed to probe vLLM /v1/apxm/scheduler during health_check");
            }
        }

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
            map.insert(
                "graph_capabilities".to_string(),
                serde_json::to_value(self.graph_capabilities()).unwrap_or_default(),
            );
            if let Some(policy) = self.scheduler_policy.read().clone() {
                map.insert("scheduler_policy".to_string(), policy.into());
            }
        }
        meta
    }

    fn graph_capabilities(&self) -> BackendGraphCapabilities {
        // A registered GraphAwareVllmBackend is guaranteed to have passed
        // the synchronous /v1/apxm/* probe in `health_check`. If the fork
        // process later drops the routes, health_check downgrades the backend
        // instead of silently degrading the capability surface.
        let scheduler_is_priority = self
            .scheduler_policy
            .read()
            .as_deref()
            .is_some_and(|policy| policy == super::graph_meta::SCHEDULER_POLICY_PRIORITY);
        // If the fork advertises a `dispatch_ir_version`,
        // gate `supports_dispatch_ir_v1_internal` on the tag matching
        // `DISPATCH_IR_V1_VERSION_TAG` exactly. Older fork builds omit
        // the field; for those, "registration succeeded" is the gating
        // signal (the synchronous probe already proved /v1/apxm/* exist).
        let dispatch_ir_v1_supported = match self.dispatch_ir_version.read().as_deref() {
            Some(tag) => tag == DISPATCH_IR_V1_VERSION_TAG,
            None => true,
        };
        BackendGraphCapabilities {
            supports_graph_registration: true,
            supports_request_hints: true,
            supports_priority: scheduler_is_priority,
            supports_prefix_cohorts: true,
            supports_pin_release: true,
            supports_structured_outputs: self.structured_outputs_supported,
            supports_backend_queue_state: false,
            supports_backend_cache_state: true,
            supports_cancel_groups: false,
            supports_dispatch_ir_v1_internal: dispatch_ir_v1_supported,
            supports_admin_reset_prefix_cache: true,
        }
    }

    fn response_memoization_policy(
        &self,
    ) -> crate::llm::backends::traits::ResponseMemoizationPolicy {
        crate::llm::backends::traits::ResponseMemoizationPolicy::BackendPrefix
    }

    fn supports_graph_extensions(&self) -> bool {
        true
    }

    fn supports_auto_tool_choice(&self) -> bool {
        self.auto_tool_choice_supported.load(Ordering::Relaxed)
    }

    fn next_execution_id(&self) -> String {
        let counter = self.execution_counter.fetch_add(1, Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
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
