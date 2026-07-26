//! Backend registry keyed by exact model reference.
//!
//! Provides backend registration, one exact model-reference-to-backend
//! binding, health recording, rate limiting, and dispatch. A request names
//! exactly one model reference; the registry returns the single backend bound
//! to it or fails. The registry never ranks, orders, or chooses among
//! candidate backends.

#[cfg(feature = "metrics")]
use crate::llm::RequestMetrics;
use crate::llm::backends::{
    CorrelatedBatchingCapability, CorrelatedLLMOutcome, CorrelatedLLMRequest, LLMBackend,
    LLMRequest, LLMResponse, StreamChunk,
};
use crate::llm::rate_limit::{RateLimitConfig, RateLimiter, SystemClock};
use crate::llm::wire::response_metadata;
use anyhow::{Context as AnyhowContext, Result};
use apxm_core::types::BackendGraphCapabilities;
use apxm_core::types::TokenUsage;
use dashmap::DashMap;
use futures::stream::{Stream, StreamExt};
use std::collections::{BTreeSet, HashMap};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

mod health;

pub use health::{HealthMonitor, HealthStatus};

/// LLM Registry manages backends bound to exact model references.
///
/// Backends are stored as `Arc<dyn LLMBackend>`, allowing both enum-based
/// `Provider` instances and custom trait implementations to be registered.
///
/// NOTE: `backends` uses `RwLock<HashMap<...>>` instead of `DashMap` because
/// `DashMap<K, Arc<dyn Trait>>` triggers lifetime invariance errors when the
/// containing type is used across async boundaries (DashMap `Map` trait issue).
#[derive(Clone)]
pub struct LLMRegistry {
    /// Registered backends by name
    backends: Arc<parking_lot::RwLock<HashMap<String, Arc<dyn LLMBackend>>>>,
    /// Exact model reference to the one backend that serves it. A model
    /// reference binds to exactly one backend; a conflicting rebind is
    /// rejected rather than resolved.
    model_backends: Arc<DashMap<String, String>>,
    /// Health monitor
    health_monitor: Arc<HealthMonitor>,
    /// Metrics tracker
    #[cfg(feature = "metrics")]
    metrics: crate::llm::MetricsTracker,
    /// Rate limiter
    rate_limiter: Arc<RateLimiter<SystemClock>>,
    /// graph_id → (handles_peak, blocks_peak), populated by `start_pin_polling`
    /// and read by `pre_release_status_all`.
    #[allow(clippy::type_complexity)]
    pin_peaks: Arc<DashMap<String, (Arc<AtomicU64>, Arc<AtomicU64>)>>,
}

/// Atomically bump an `AtomicU64` slot to the larger of its current value and `val`.
fn bump_max(slot: &AtomicU64, val: u64) {
    let mut prev = slot.load(Ordering::Relaxed);
    while val > prev {
        match slot.compare_exchange_weak(prev, val, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(p) => prev = p,
        }
    }
}

/// RAII wrapper around the pin-polling background task.
///
/// Aborts the task on `Drop` so a polling loop cannot outlive the executor
/// scope that started it (e.g. on a panic or early-return code path).
/// Call [`Self::abort`] explicitly when ordering matters, e.g. before
/// `pre_release_status_all` so the final fold sees the full peak.
pub struct PinPollHandle {
    inner: Option<tokio::task::JoinHandle<()>>,
}

impl PinPollHandle {
    /// Abort the background polling task immediately.
    pub fn abort(mut self) {
        if let Some(handle) = self.inner.take() {
            handle.abort();
        }
    }
}

impl Drop for PinPollHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.inner.take() {
            handle.abort();
        }
    }
}

struct StreamingAttempt {
    backend_name: String,
    backend_model: String,
    backend: Arc<dyn LLMBackend>,
    estimated_cost: f64,
}

const STREAM_FAILURE_BACKEND_ERROR: &str = "backend_error";
const STREAM_FAILURE_CHUNK_ERROR: &str = "chunk_error";
const STREAM_FAILURE_MISSING_DONE: &str = "missing_done";
const STREAM_ENDED_WITHOUT_DONE: &str = "stream ended without terminal Done chunk";

/// Classifies failures that happen after a streaming backend has emitted data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingFailureKind {
    /// The backend stream returned an error item after commitment.
    BackendError,
    /// The backend emitted a `StreamChunk::Error` after commitment.
    ChunkError,
    /// The backend stream ended without a terminal `Done` chunk after commitment.
    MissingDone,
}

impl std::fmt::Display for StreamingFailureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::BackendError => STREAM_FAILURE_BACKEND_ERROR,
            Self::ChunkError => STREAM_FAILURE_CHUNK_ERROR,
            Self::MissingDone => STREAM_FAILURE_MISSING_DONE,
        };
        f.write_str(label)
    }
}

/// Error returned when a committed stream fails and the backend is known.
#[derive(Debug, thiserror::Error)]
#[error("Backend '{backend_name}' stream failed after commit ({kind}): {message}")]
pub struct StreamingBackendError {
    backend_name: String,
    kind: StreamingFailureKind,
    message: String,
}

impl StreamingBackendError {
    pub fn new(
        backend_name: impl Into<String>,
        kind: StreamingFailureKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            backend_name: backend_name.into(),
            kind,
            message: message.into(),
        }
    }

    pub fn backend_name(&self) -> &str {
        &self.backend_name
    }

    pub fn kind(&self) -> StreamingFailureKind {
        self.kind
    }
}

/// A request does not carry an exact model reference bound to a backend.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ModelReferenceError {
    /// The request carries no model reference.
    #[error("LLM request has no model reference; every request names exactly one model")]
    MissingModel,

    /// The referenced model is bound to no backend.
    #[error("LLM model '{model}' is not bound to a backend")]
    UnknownModel { model: String },

    /// An explicit backend contradicts the model's registered binding.
    #[error(
        "LLM model '{model}' is bound to backend '{bound_backend}', not '{requested_backend}'"
    )]
    ModelBackendMismatch {
        model: String,
        bound_backend: String,
        requested_backend: String,
    },

    /// A model reference is already bound to a different backend.
    #[error("LLM model '{model}' is already bound to backend '{bound_backend}'")]
    ConflictingBinding {
        model: String,
        bound_backend: String,
    },
}

/// Runtime capability evidence for one exact backend/model binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelatedBatchRoute {
    pub backend_name: String,
    pub model: String,
    pub max_batch_size: usize,
}

impl LLMRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::with_rate_limits(HashMap::new()).expect("empty rate limit config should be valid")
    }

    /// Create a new registry with backend-specific rate limits.
    pub fn with_rate_limits(rate_limit_configs: HashMap<String, RateLimitConfig>) -> Result<Self> {
        let rate_limiter = RateLimiter::new(rate_limit_configs, Arc::new(SystemClock))
            .map_err(|e| anyhow::anyhow!("Invalid rate limit config: {}", e))?;

        Ok(LLMRegistry {
            backends: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            model_backends: Arc::new(DashMap::new()),
            health_monitor: Arc::new(HealthMonitor::new()),
            #[cfg(feature = "metrics")]
            metrics: crate::llm::MetricsTracker::new(),
            rate_limiter: Arc::new(rate_limiter),
            pin_peaks: Arc::new(DashMap::new()),
        })
    }

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> &crate::llm::MetricsTracker {
        &self.metrics
    }

    /// Return correlated-batch capability evidence for the request's exact
    /// model reference. `None` means callers must retain per-node dispatch.
    pub fn correlated_batch_route(
        &self,
        request: &LLMRequest,
    ) -> Result<Option<CorrelatedBatchRoute>> {
        let backend_name = self.resolve_backend(request)?;
        let model = self.require_model_reference(request)?.to_string();
        let backend = self
            .backends
            .read()
            .get(&backend_name)
            .cloned()
            .with_context(|| format!("Backend '{backend_name}' not found"))?;
        let CorrelatedBatchingCapability::CorrelatedOutcomes { max_batch_size } =
            backend.correlated_batching_capability()
        else {
            return Ok(None);
        };
        if max_batch_size == 0 {
            return Ok(None);
        }
        Ok(Some(CorrelatedBatchRoute {
            backend_name,
            model,
            max_batch_size,
        }))
    }

    /// Record a streaming-path outcome against the same metrics + health
    /// surface that `try_generate` populates for non-streaming calls.
    ///
    /// The streaming dispatcher invokes `backend.generate_stream` directly
    /// instead of going through `try_generate`, so neither the metrics
    /// tracker nor the health monitor saw stream completions before this
    /// hook existed. Without it, `runtime.llm.{total_requests,
    /// avg_latency_ms, p50_latency_ms, p99_latency_ms}` stayed at zero in
    /// every session metrics report even though prefill/decode timings were
    /// being tracked per node.
    pub fn record_streaming_outcome(
        &self,
        backend_name: &str,
        #[cfg_attr(not(feature = "metrics"), allow(unused_variables))] backend_model: &str,
        latency: Duration,
        #[cfg_attr(not(feature = "metrics"), allow(unused_variables))] usage: Option<TokenUsage>,
        success: bool,
    ) {
        #[cfg(feature = "metrics")]
        {
            let usage = usage.unwrap_or_else(|| TokenUsage::new(0, 0));
            self.metrics.record(RequestMetrics::new(
                backend_name.to_string(),
                backend_model.to_string(),
                latency,
                usage,
                success,
                0,
            ));
        }
        if success {
            self.health_monitor.record_success(backend_name);
        } else {
            self.health_monitor.record_failure(backend_name);
        }
    }

    /// Register a backend with a given name.
    ///
    /// Accepts any type implementing `LLMBackend` (including `Provider` enum).
    pub fn register(
        &self,
        name: impl Into<String>,
        backend: impl LLMBackend + 'static,
    ) -> Result<()> {
        let name = name.into();
        let backend: Arc<dyn LLMBackend> = Arc::new(backend);

        self.backends.write().insert(name.clone(), backend);
        self.health_monitor.register_backend(&name);

        Ok(())
    }

    /// Register a pre-wrapped `Arc<dyn LLMBackend>`.
    pub fn register_arc(
        &self,
        name: impl Into<String>,
        backend: Arc<dyn LLMBackend>,
    ) -> Result<()> {
        let name = name.into();
        self.backends.write().insert(name.clone(), backend);
        self.health_monitor.register_backend(&name);
        Ok(())
    }

    /// Unregister a backend and every model reference bound to it.
    pub fn unregister(&self, name: &str) -> Result<()> {
        self.backends
            .write()
            .remove(name)
            .with_context(|| format!("Backend '{}' not found", name))?;

        self.model_backends
            .retain(|_, backend_name| backend_name != name);
        self.health_monitor.unregister_backend(name);

        Ok(())
    }

    /// Bind one exact model reference to the backend that serves it.
    ///
    /// A model reference has exactly one backend. Rebinding it to a different
    /// backend is rejected: the caller must unregister the current backend
    /// first, so no request can ever observe two backends for one reference.
    pub fn bind_model(
        &self,
        model: impl Into<String>,
        backend: impl Into<String>,
    ) -> Result<()> {
        let backend_name = backend.into();
        if !self.backends.read().contains_key(&backend_name) {
            anyhow::bail!("Backend '{}' not registered", backend_name);
        }
        let model = model.into();
        if let Some(existing) = self.model_backends.get(&model) {
            let bound_backend = existing.value().clone();
            if bound_backend != backend_name {
                return Err(ModelReferenceError::ConflictingBinding {
                    model,
                    bound_backend,
                }
                .into());
            }
            return Ok(());
        }
        self.model_backends.insert(model, backend_name);
        Ok(())
    }

    /// Get registered backend names.
    pub fn backend_names(&self) -> Vec<String> {
        self.backends.read().keys().cloned().collect()
    }

    /// Get backend by name.
    pub fn get_backend(&self, name: &str) -> Option<Arc<dyn LLMBackend>> {
        self.backends.read().get(name).cloned()
    }

    /// Get health status of a backend.
    pub fn backend_health(&self, name: &str) -> HealthStatus {
        self.health_monitor.status(name)
    }

    /// Execute one compiler-approved request batch against one exact model
    /// reference.
    ///
    /// Every request in the batch names the same model reference and therefore
    /// resolves to the same backend. Missing capability evidence, a
    /// disagreeing model reference, malformed outcomes, and transport failures
    /// all return a typed error so the scheduler can retain per-node dispatch.
    pub async fn generate_correlated_batch(
        &self,
        requests: Vec<CorrelatedLLMRequest>,
    ) -> Result<Vec<CorrelatedLLMOutcome>> {
        if requests.is_empty() {
            anyhow::bail!("correlated batch must contain at least one request");
        }

        let mut prepared = Vec::with_capacity(requests.len());
        let mut expected_ids = BTreeSet::new();
        let mut resolved_backend: Option<String> = None;
        let mut resolved_model: Option<String> = None;

        for CorrelatedLLMRequest {
            correlation_id,
            request,
        } in requests
        {
            if correlation_id.trim().is_empty() || !expected_ids.insert(correlation_id.clone()) {
                anyhow::bail!("correlated batch requires unique non-empty correlation ids");
            }

            let mut request = request;
            let backend_name = self.resolve_backend(&request)?;
            let model = self.require_model_reference(&request)?.to_string();
            request.backend = Some(backend_name.clone());

            match (&resolved_backend, &resolved_model) {
                (Some(expected_backend), Some(expected_model)) => {
                    if expected_backend != &backend_name || expected_model != &model {
                        anyhow::bail!(
                            "correlated batch requests must name one exact backend/model reference"
                        );
                    }
                }
                (None, None) => {
                    resolved_backend = Some(backend_name);
                    resolved_model = Some(model);
                }
                _ => anyhow::bail!("correlated batch model reference state is incomplete"),
            }

            prepared.push(CorrelatedLLMRequest {
                correlation_id,
                request,
            });
        }

        let backend_name = resolved_backend.expect("non-empty batch resolves a backend");
        let backend = self
            .backends
            .read()
            .get(&backend_name)
            .cloned()
            .with_context(|| format!("Backend '{backend_name}' not found"))?;

        if self.health_monitor.status(&backend_name) == HealthStatus::Unhealthy {
            anyhow::bail!("Backend '{backend_name}' is unhealthy");
        }

        let max_batch_size = match backend.correlated_batching_capability() {
            CorrelatedBatchingCapability::CorrelatedOutcomes { max_batch_size }
                if max_batch_size > 0 =>
            {
                max_batch_size
            }
            _ => anyhow::bail!(
                "Backend '{backend_name}' does not declare correlated batch outcome support"
            ),
        };
        if prepared.len() > max_batch_size {
            anyhow::bail!(
                "correlated batch has {} requests but backend '{backend_name}' declares a maximum of {max_batch_size}",
                prepared.len()
            );
        }

        for item in &prepared {
            Self::enforce_context_window(&backend_name, backend.as_ref(), &item.request)?;
        }

        let mut estimated_costs = HashMap::with_capacity(prepared.len());
        for item in &prepared {
            let estimated = self
                .rate_limiter
                .check_and_consume_request(
                    &backend_name,
                    item.request.max_tokens.map(|tokens| tokens as f64),
                )
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            estimated_costs.insert(item.correlation_id.clone(), estimated);
        }

        let result = backend.generate_correlated_batch(prepared).await;

        let outcomes = match result {
            Ok(outcomes) => outcomes,
            Err(error) => {
                self.health_monitor.record_failure(&backend_name);
                return Err(error).context(format!(
                    "correlated batch dispatch failed on backend '{backend_name}'"
                ));
            }
        };

        let received_ids = outcomes
            .iter()
            .map(CorrelatedLLMOutcome::correlation_id)
            .collect::<BTreeSet<_>>();
        if outcomes.len() != expected_ids.len()
            || received_ids.len() != expected_ids.len()
            || !received_ids.iter().all(|id| expected_ids.contains(*id))
        {
            self.health_monitor.record_failure(&backend_name);
            anyhow::bail!(
                "backend '{backend_name}' returned incomplete, duplicate, or unknown correlated batch outcomes"
            );
        }

        for outcome in &outcomes {
            if let CorrelatedLLMOutcome::Response {
                correlation_id,
                response,
            } = outcome
                && let Some(estimated) = estimated_costs.get(correlation_id)
            {
                self.rate_limiter.reconcile_request(
                    &backend_name,
                    *estimated,
                    Some(response.usage.total_tokens as f64),
                );
            }
        }
        self.health_monitor.record_success(&backend_name);
        Ok(outcomes)
    }

    /// Generate against the named backend, which must be the exact backend the
    /// request's model reference is bound to.
    pub async fn generate_with_backend(
        &self,
        backend_name: &str,
        request: LLMRequest,
    ) -> Result<LLMResponse> {
        let mut request = request;
        request.backend = Some(backend_name.to_string());
        self.resolve_backend(&request)?;
        self.try_generate(backend_name, request).await
    }

    /// Try to generate using a specific backend, tracking health and cost.
    async fn try_generate(&self, backend_name: &str, request: LLMRequest) -> Result<LLMResponse> {
        let backend = self
            .backends
            .read()
            .get(backend_name)
            .cloned()
            .with_context(|| format!("Backend '{}' not found", backend_name))?;

        // Check health status
        let health = self.health_monitor.status(backend_name);
        if health == HealthStatus::Unhealthy {
            anyhow::bail!("Backend '{}' is unhealthy", backend_name);
        }

        Self::enforce_context_window(backend_name, backend.as_ref(), &request)?;

        // Check rate limit before dispatching to backend
        let estimated_cost = self
            .rate_limiter
            .check_and_consume_request(backend_name, request.max_tokens.map(|tokens| tokens as f64))
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let start = Instant::now();
        let result = backend.generate(request).await;
        let latency = start.elapsed();

        #[cfg(feature = "metrics")]
        {
            let usage = match &result {
                Ok(response) => response.usage.clone(),
                Err(_) => TokenUsage::new(0, 0),
            };
            let metrics = RequestMetrics::new(
                backend_name.to_string(),
                backend.model().to_string(),
                latency,
                usage,
                result.is_ok(),
                0,
            );
            self.metrics.record(metrics);
        }

        match result {
            Ok(response) => {
                self.rate_limiter.reconcile_request(
                    backend_name,
                    estimated_cost,
                    Some(response.usage.total_tokens as f64),
                );

                // Record success
                self.health_monitor.record_success(backend_name);
                Ok(response)
            }
            Err(e) => {
                // Record failure
                self.health_monitor.record_failure(backend_name);
                Err(e)
            }
        }
    }

    fn enforce_context_window(
        backend_name: &str,
        backend: &dyn LLMBackend,
        request: &LLMRequest,
    ) -> Result<()> {
        let Some(input_tokens) = request.context_input_tokens else {
            return Ok(());
        };
        let output_tokens = request
            .max_tokens
            .context("context-window admission requires an explicit output reservation")?;
        let model = request
            .model
            .as_deref()
            .context("context-window admission requires an exact model reference")?;
        let Some(context_window) = backend.context_window_for_model(model) else {
            return Ok(());
        };
        let required_tokens = input_tokens.saturating_add(output_tokens);
        if required_tokens > context_window {
            anyhow::bail!(
                "configured context window rejected request for backend '{backend_name}' model '{model}': {input_tokens} input + {output_tokens} output tokens exceed {context_window}"
            );
        }
        Ok(())
    }

    /// Resolve the backend bound to the request's model reference for streaming.
    ///
    /// Returns the resolved backend `Arc` so the caller can call
    /// `generate_stream()` on it directly. The caller keeps the Arc
    /// alive for the stream's lifetime, avoiding self-referential
    /// borrow issues.
    pub fn resolve_backend_for_streaming(
        &self,
        request: &LLMRequest,
    ) -> Result<Arc<dyn LLMBackend>> {
        let backend_name = self.resolve_backend(request)?;

        let backend = self
            .backends
            .read()
            .get(&backend_name)
            .cloned()
            .with_context(|| format!("Backend '{}' not found", backend_name))?;

        let health = self.health_monitor.status(&backend_name);
        if health == HealthStatus::Unhealthy {
            anyhow::bail!("Backend '{}' is unhealthy", backend_name);
        }

        Self::enforce_context_window(&backend_name, backend.as_ref(), request)?;

        Ok(backend)
    }

    /// Stream one request against the backend bound to its exact model
    /// reference.
    ///
    /// A pre-commit failure is an error the caller handles; the registry never
    /// re-dispatches the request to another backend.
    pub fn generate_stream<'a>(
        &'a self,
        request: &'a LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + 'a>> {
        Box::pin(async_stream::try_stream! {
            let backend_name = match self.resolve_backend(request) {
                Ok(name) => name,
                Err(e) => {
                    Err(e)?;
                    return;
                }
            };

            let attempt = match self.begin_streaming_attempt(&backend_name, request) {
                Ok(attempt) => attempt,
                Err(error) => {
                    Err(error)?;
                    return;
                }
            };

            let started_at = Instant::now();
            let mut stream = attempt.backend.generate_stream(request.clone());
            let mut committed = false;

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        self.finish_streaming_attempt(&attempt, started_at.elapsed(), None, false);
                        Err(anyhow::Error::new(StreamingBackendError::new(
                            attempt.backend_name.as_str(),
                            StreamingFailureKind::BackendError,
                            error.to_string(),
                        )))?;
                        return;
                    }
                };

                if let StreamChunk::Error(message) = &chunk {
                    self.finish_streaming_attempt(&attempt, started_at.elapsed(), None, false);
                    Err(anyhow::Error::new(StreamingBackendError::new(
                        attempt.backend_name.as_str(),
                        StreamingFailureKind::ChunkError,
                        message.as_str(),
                    )))?;
                    return;
                }

                committed = true;

                match chunk {
                    StreamChunk::Done(mut response) => {
                        self.finish_streaming_attempt(
                            &attempt,
                            started_at.elapsed(),
                            Some(response.usage.clone()),
                            true,
                        );
                        response = response
                            .with_metadata(
                                response_metadata::APXM_BACKEND_NAME,
                                serde_json::json!(attempt.backend_name.as_str()),
                            )
                            .with_metadata(
                                response_metadata::APXM_BACKEND_MODEL,
                                serde_json::json!(attempt.backend_model.as_str()),
                            );
                        yield StreamChunk::Done(response);
                        return;
                    }
                    chunk => yield chunk,
                }
            }

            self.finish_streaming_attempt(&attempt, started_at.elapsed(), None, false);
            if committed {
                Err(anyhow::Error::new(StreamingBackendError::new(
                    attempt.backend_name.as_str(),
                    StreamingFailureKind::MissingDone,
                    STREAM_ENDED_WITHOUT_DONE,
                )))?;
            } else {
                Err(anyhow::anyhow!(
                    "Backend '{}' returned an empty stream",
                    attempt.backend_name
                ))?;
            }
        })
    }

    fn begin_streaming_attempt(
        &self,
        backend_name: &str,
        request: &LLMRequest,
    ) -> Result<StreamingAttempt> {
        let backend = self
            .backends
            .read()
            .get(backend_name)
            .cloned()
            .with_context(|| format!("Backend '{}' not found", backend_name))?;

        let health = self.health_monitor.status(backend_name);
        if health == HealthStatus::Unhealthy {
            anyhow::bail!("Backend '{}' is unhealthy", backend_name);
        }

        Self::enforce_context_window(backend_name, backend.as_ref(), request)?;

        let estimated_cost = self
            .rate_limiter
            .check_and_consume_request(backend_name, request.max_tokens.map(|tokens| tokens as f64))
            .map_err(|error| anyhow::anyhow!("{}", error))?;

        Ok(StreamingAttempt {
            backend_name: backend_name.to_string(),
            backend_model: backend.model().to_string(),
            backend,
            estimated_cost,
        })
    }

    fn finish_streaming_attempt(
        &self,
        attempt: &StreamingAttempt,
        latency: Duration,
        usage: Option<TokenUsage>,
        success: bool,
    ) {
        self.record_streaming_outcome(
            &attempt.backend_name,
            &attempt.backend_model,
            latency,
            usage.clone(),
            success,
        );

        if success {
            self.rate_limiter.reconcile_request(
                &attempt.backend_name,
                attempt.estimated_cost,
                usage.map(|usage| usage.total_tokens as f64),
            );
        }
    }

    /// The exact model reference a request names.
    ///
    /// A request without a model reference is rejected; the registry has no
    /// default, ambient, or inferred model.
    fn require_model_reference<'a>(&self, request: &'a LLMRequest) -> Result<&'a str> {
        request
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .ok_or_else(|| ModelReferenceError::MissingModel.into())
    }

    /// Return the one backend bound to the request's exact model reference.
    ///
    /// There is no candidate set, no ordering, and no health-based choice: the
    /// reference either has a binding or the request is rejected. A request
    /// that also names a backend must name the bound one.
    pub fn resolve_backend(&self, request: &LLMRequest) -> Result<String> {
        let model = self.require_model_reference(request)?;
        let bound_backend = self
            .model_backends
            .get(model)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| ModelReferenceError::UnknownModel {
                model: model.to_string(),
            })?;

        if let Some(requested_backend) = request.backend.as_deref()
            && requested_backend != bound_backend.as_str()
        {
            return Err(ModelReferenceError::ModelBackendMismatch {
                model: model.to_string(),
                bound_backend,
                requested_backend: requested_backend.to_string(),
            }
            .into());
        }

        Ok(bound_backend)
    }

    /// Snapshot all backends (clones name + Arc pairs out of the lock).
    fn backend_snapshot(&self) -> Vec<(String, Arc<dyn LLMBackend>)> {
        self.backends
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Register graph metadata with all backends (best-effort).
    ///
    /// Backends that don't support graph registration (default impl) silently succeed.
    pub async fn register_graph_all(&self, metadata: apxm_core::types::GraphMetadata) {
        for (name, backend) in self.backend_snapshot() {
            if let Err(e) = backend.register_graph(metadata.clone()).await {
                tracing::warn!(
                    backend = %name,
                    error = %e,
                    "Failed to register graph with backend (non-fatal)"
                );
            }
        }
    }

    /// Release graph from all backends (best-effort).
    pub async fn release_graph_all(&self, graph_id: &str) {
        for (name, backend) in self.backend_snapshot() {
            if let Err(e) = backend.release_graph(graph_id).await {
                tracing::warn!(
                    backend = %name,
                    error = %e,
                    "Failed to release graph from backend (non-fatal)"
                );
            }
        }
    }

    /// Snapshot only the backends that opt into graph-aware extensions
    /// (`LLMBackend::supports_graph_extensions()`).
    pub fn find_graph_aware_backends(&self) -> Vec<(String, Arc<dyn LLMBackend>)> {
        self.backend_snapshot()
            .into_iter()
            .filter(|(_, b)| b.supports_graph_extensions())
            .collect()
    }

    /// Snapshot graph-aware capability evidence for every registered backend.
    pub fn graph_capabilities(&self) -> HashMap<String, BackendGraphCapabilities> {
        self.backend_snapshot()
            .into_iter()
            .map(|(name, backend)| (name, backend.graph_capabilities()))
            .collect()
    }

    /// Collect graph status from all graph-aware backends before releasing.
    ///
    /// Folds in pin peaks recorded by `start_pin_polling` for the same
    /// `graph_id` if any are present. Failures are logged and skipped.
    pub async fn pre_release_status_all(
        &self,
        graph_id: &str,
    ) -> Vec<apxm_core::types::GraphStatusSnapshot> {
        let mut results = Vec::new();
        for (name, backend) in self.find_graph_aware_backends() {
            match backend.get_graph_status(graph_id).await {
                Ok(Some(value)) => results.push(value.with_backend_name(name)),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        backend = %name,
                        graph_id = %graph_id,
                        error = %e,
                        "pre_release_status_all: get_graph_status failed (skipping)"
                    );
                }
            }
        }
        if let Some((_, (h_peak, b_peak))) = self.pin_peaks.remove(graph_id) {
            let h = h_peak.load(Ordering::Relaxed);
            let b = b_peak.load(Ordering::Relaxed);
            if h > 0 || b > 0 {
                results = results
                    .into_iter()
                    .map(|s| s.with_pin_peaks(h, b))
                    .collect();
            }
        }
        results
    }

    /// Spawn a background task that polls every graph-aware backend at
    /// `interval`, recording peak `pinned_handles` / `pinned_blocks` for
    /// `graph_id`. Returns `None` if no graph-aware backends are registered
    /// (no point waking a no-op poll loop). The returned [`PinPollHandle`]
    /// aborts the task on `Drop`, so callers cannot leak it; explicit
    /// `.abort()` before `pre_release_status_all` is still preferred for
    /// ordering clarity.
    pub fn start_pin_polling(
        self: &Arc<Self>,
        graph_id: String,
        interval: Duration,
    ) -> Option<PinPollHandle> {
        if self.find_graph_aware_backends().is_empty() {
            return None;
        }
        let handles_peak = Arc::new(AtomicU64::new(0));
        let blocks_peak = Arc::new(AtomicU64::new(0));
        self.pin_peaks.insert(
            graph_id.clone(),
            (handles_peak.clone(), blocks_peak.clone()),
        );
        let registry = Arc::clone(self);
        let inner = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Skip the immediate first tick so we don't race the register call.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                for (_name, backend) in registry.find_graph_aware_backends() {
                    if let Ok(Some(snap)) = backend.get_graph_status(&graph_id).await {
                        bump_max(&handles_peak, snap.pinned_handles);
                        bump_max(&blocks_peak, snap.pinned_blocks);
                    }
                }
            }
        });
        Some(PinPollHandle { inner: Some(inner) })
    }

    /// Build a `BackendMetricsSource` from the tracker's aggregates and
    /// the supplied vLLM graph status values.
    #[cfg(feature = "metrics")]
    pub fn collect_backend_metrics(
        &self,
        graph_status_snapshots: Vec<apxm_core::types::GraphStatusSnapshot>,
    ) -> Option<crate::llm::observability::BackendMetricsSource> {
        let aggregate = self.metrics.aggregate();
        let per_backend = self.metrics.aggregate_per_backend();
        let graph_capabilities = self.graph_capabilities();
        if aggregate.total_requests == 0
            && per_backend.is_empty()
            && graph_status_snapshots.is_empty()
            && graph_capabilities.is_empty()
        {
            return None;
        }
        Some(crate::llm::observability::BackendMetricsSource {
            aggregate,
            per_backend,
            graph_status_snapshots,
            graph_capabilities,
        })
    }

    /// Perform health checks on all backends.
    pub async fn check_all_backends(&self) -> HashMap<String, HealthStatus> {
        let mut results = HashMap::new();

        for (name, backend) in self.backend_snapshot() {
            let status = match backend.health_check().await {
                Ok(()) => HealthStatus::Healthy,
                Err(_) => HealthStatus::Unhealthy,
            };

            self.health_monitor.set_status(&name, status);
            results.insert(name, status);
        }

        results
    }
}

impl Default for LLMRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod request_recording_tests {
    use super::*;
    use crate::llm::backends::mock::MockLLMBackend;

    /// request recording (`RequestMetrics` via `MetricsTracker`) must
    /// be part of the default build — no `--features metrics` opt-in should
    /// be required for the observed path to equal the default build. This
    /// test only compiles at all if the `metrics` feature (and therefore
    /// `LLMRegistry::metrics()`, itself `#[cfg(feature = "metrics")]`) is
    /// active under a plain `cargo test` with no extra `--features` flag.
    #[tokio::test]
    async fn request_recording_is_active_without_opting_into_a_feature_flag() {
        let registry = LLMRegistry::new();
        registry
            .register("mock", MockLLMBackend::static_response("hello"))
            .expect("register mock backend");
        registry
            .bind_model("fixture-model", "mock")
            .expect("bind fixture model");

        registry
            .generate_with_backend("mock", LLMRequest::new("hi").with_model("fixture-model"))
            .await
            .expect("mock generate succeeds");

        let aggregate = registry.metrics().aggregate();
        assert!(
            aggregate.total_requests > 0,
            "a generate() call on the default build must be recorded by the metrics tracker \
             with no feature flag required"
        );
    }

    #[tokio::test]
    async fn configured_context_window_rejects_an_admitted_oversized_request() {
        let registry = LLMRegistry::new();
        let backend = MockLLMBackend::static_response("must not run").model_name("fixture-model");
        registry
            .register("mock", backend.clone())
            .expect("register mock backend");
        registry
            .bind_model("fixture-model", "mock")
            .expect("bind fixture model");

        let error = registry
            .generate_with_backend(
                "mock",
                LLMRequest::new("context evidence")
                    .with_model("fixture-model")
                    .with_max_tokens(1)
                    .with_context_input_tokens(128_000),
            )
            .await
            .expect_err("configured context window must reject the request");

        assert!(
            error
                .to_string()
                .contains("configured context window rejected")
        );
        assert_eq!(backend.call_count(), 0);
    }

    #[tokio::test]
    async fn correlated_batch_uses_the_declared_contract_and_preserves_ids() {
        let registry = LLMRegistry::new();
        let backend = MockLLMBackend::static_response("batched").model_name("fixture-model");
        registry
            .register("mock", backend.clone())
            .expect("register mock backend");
        registry
            .bind_model("fixture-model", "mock")
            .expect("bind fixture model");

        let outcomes = registry
            .generate_correlated_batch(vec![
                CorrelatedLLMRequest {
                    correlation_id: "node-2".to_string(),
                    request: LLMRequest::new("second").with_model("fixture-model"),
                },
                CorrelatedLLMRequest {
                    correlation_id: "node-1".to_string(),
                    request: LLMRequest::new("first").with_model("fixture-model"),
                },
            ])
            .await
            .expect("correlated batch succeeds");

        assert_eq!(backend.batch_submission_count(), 1);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].correlation_id(), "node-2");
        assert_eq!(outcomes[1].correlation_id(), "node-1");
    }

    #[tokio::test]
    async fn correlated_batch_rejects_mixed_model_references_before_backend_dispatch() {
        let registry = LLMRegistry::new();
        let left = MockLLMBackend::static_response("left").model_name("left-model");
        let right = MockLLMBackend::static_response("right").model_name("right-model");
        registry
            .register("left", left.clone())
            .expect("register left");
        registry
            .register("right", right.clone())
            .expect("register right");
        registry
            .bind_model("left-model", "left")
            .expect("bind left model");
        registry
            .bind_model("right-model", "right")
            .expect("bind right model");

        let error = registry
            .generate_correlated_batch(vec![
                CorrelatedLLMRequest {
                    correlation_id: "left".to_string(),
                    request: LLMRequest::new("left").with_model("left-model"),
                },
                CorrelatedLLMRequest {
                    correlation_id: "right".to_string(),
                    request: LLMRequest::new("right").with_model("right-model"),
                },
            ])
            .await
            .expect_err("mixed model references are not a batch");

        assert!(
            error
                .to_string()
                .contains("one exact backend/model reference")
        );
        assert_eq!(left.batch_submission_count(), 0);
        assert_eq!(right.batch_submission_count(), 0);
    }

    /// An unbound model reference is rejected even when the registry holds
    /// healthy, serviceable backends — including one that already serves a
    /// different model reference.
    ///
    /// The registered backends are the whole point: they are exactly the
    /// candidate set any first-available, provider-inferred, or
    /// health-ranked selection would draw from. If resolution ever grows a
    /// selection step, `unknown-model` resolves to `mock` or `other` and
    /// this test fails. Resolving against an empty registry would not
    /// distinguish "this reference has no binding" from "there was nothing
    /// to select".
    #[test]
    fn registry_rejects_unbound_model_without_provider_inference() {
        let registry = LLMRegistry::new();
        registry
            .register("mock", MockLLMBackend::static_response("hello"))
            .expect("register mock backend");
        registry
            .register("other", MockLLMBackend::static_response("hello"))
            .expect("register other backend");
        registry
            .bind_model("fixture-model", "mock")
            .expect("bind fixture model");

        assert_eq!(
            registry.backend_names().len(),
            2,
            "the rejection must be observed with a non-empty candidate set"
        );

        match registry.resolve_backend(&LLMRequest::new("hi").with_model("unknown-model")) {
            Ok(backend) => panic!(
                "unbound model resolved to backend {backend:?}: resolution selected a \
                 backend the reference is not bound to"
            ),
            Err(error) => assert!(
                matches!(
                    error.downcast_ref::<ModelReferenceError>(),
                    Some(ModelReferenceError::UnknownModel { model }) if model == "unknown-model"
                ),
                "expected UnknownModel for the unbound reference, got: {error}"
            ),
        }
    }

    #[test]
    fn registry_rejects_a_request_without_a_model_reference() {
        let registry = LLMRegistry::new();
        registry
            .register("mock", MockLLMBackend::static_response("hello"))
            .expect("register mock backend");

        let error = registry
            .resolve_backend(&LLMRequest::new("hi"))
            .expect_err("a request without a model reference must fail closed");

        assert!(matches!(
            error.downcast_ref::<ModelReferenceError>(),
            Some(ModelReferenceError::MissingModel)
        ));
    }

    #[test]
    fn registry_rejects_a_backend_that_is_not_the_bound_one() {
        let registry = LLMRegistry::new();
        registry
            .register("mock", MockLLMBackend::static_response("hello"))
            .expect("register mock backend");
        registry
            .register("other", MockLLMBackend::static_response("hello"))
            .expect("register other backend");
        registry
            .bind_model("fixture-model", "mock")
            .expect("bind fixture model");

        let error = registry
            .resolve_backend(&LLMRequest::new("hi").with_model("fixture-model").with_backend("other"))
            .expect_err("a non-bound backend must fail closed");

        assert!(matches!(
            error.downcast_ref::<ModelReferenceError>(),
            Some(ModelReferenceError::ModelBackendMismatch { model, bound_backend, requested_backend })
                if model == "fixture-model" && bound_backend == "mock" && requested_backend == "other"
        ));
    }

    #[test]
    fn a_model_reference_binds_to_exactly_one_backend() {
        let registry = LLMRegistry::new();
        registry
            .register("mock", MockLLMBackend::static_response("hello"))
            .expect("register mock backend");
        registry
            .register("other", MockLLMBackend::static_response("hello"))
            .expect("register other backend");
        registry
            .bind_model("fixture-model", "mock")
            .expect("bind fixture model");

        let error = registry
            .bind_model("fixture-model", "other")
            .expect_err("rebinding a model reference must fail closed");

        assert!(matches!(
            error.downcast_ref::<ModelReferenceError>(),
            Some(ModelReferenceError::ConflictingBinding { model, bound_backend })
                if model == "fixture-model" && bound_backend == "mock"
        ));
    }
}
