//! Multi-backend registry with intelligent routing.
//!
//! Provides backend registration, intelligent routing, health monitoring,
//! and fallback chain execution for robust LLM request handling.

use crate::llm::ProviderProtocol;
#[cfg(feature = "metrics")]
use crate::llm::RequestMetrics;
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse, StreamChunk};
use crate::llm::catalog::default_model_for_protocol;
use crate::llm::rate_limit::{RateLimitConfig, RateLimiter, SystemClock};
use anyhow::{Context as AnyhowContext, Result};
use apxm_core::types::AISOperationType;
use apxm_core::types::TokenUsage;
use dashmap::DashMap;
use futures::stream::{Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

mod health;
mod latency_profile;
mod resolver;

pub use health::{HealthMonitor, HealthStatus};
#[allow(unused_imports)]
pub use latency_profile::{
    BackendLatencyProfile, LatencyProfileStore, DEFAULT_LATENCY_EWMA_ALPHA,
    LATENCY_PROFILE_MIN_SAMPLES,
};
pub use resolver::{RoutingStrategy, SelectionCriteria};

/// LLM Registry manages multiple backends with routing and fallback.
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
    /// Default backend name
    default_backend: Arc<parking_lot::RwLock<Option<String>>>,
    /// Default model name
    default_model: Arc<parking_lot::RwLock<Option<String>>>,
    /// Operation-specific backend defaults
    operation_defaults: Arc<DashMap<AISOperationType, String>>,
    /// Operation-specific model defaults
    operation_models: Arc<DashMap<AISOperationType, String>>,
    /// Fallback chains: backend -> list of fallback backends
    fallback_chains: Arc<DashMap<String, Vec<String>>>,
    /// Named model aliases (e.g. "fast" -> "gpt-4o-mini")
    model_aliases: Arc<DashMap<String, String>>,
    /// Explicit model to backend routing (e.g. "claude-3-7" -> "anthropic")
    model_routes: Arc<DashMap<String, String>>,
    /// Health monitor
    health_monitor: Arc<HealthMonitor>,
    /// Routing strategy
    routing_strategy: RoutingStrategy,
    /// Metrics tracker
    #[cfg(feature = "metrics")]
    metrics: crate::llm::MetricsTracker,
    /// Rate limiter
    rate_limiter: Arc<RateLimiter<SystemClock>>,
    /// Round-robin counter for RoutingStrategy::RoundRobin
    round_robin_counter: Arc<AtomicUsize>,
    /// Backend name → typed provider protocol.
    backend_providers: Arc<DashMap<String, ProviderProtocol>>,
    /// graph_id → (handles_peak, blocks_peak), populated by `start_pin_polling`
    /// and consumed by `pre_release_status_all`.
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

impl LLMRegistry {
    /// Create a new empty registry with default routing.
    pub fn new() -> Self {
        Self::with_rate_limits(HashMap::new()).expect("empty rate limit config should be valid")
    }

    /// Create a new registry with backend-specific rate limits.
    pub fn with_rate_limits(rate_limit_configs: HashMap<String, RateLimitConfig>) -> Result<Self> {
        let rate_limiter = RateLimiter::new(rate_limit_configs, Arc::new(SystemClock))
            .map_err(|e| anyhow::anyhow!("Invalid rate limit config: {}", e))?;

        Ok(LLMRegistry {
            backends: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            default_backend: Arc::new(parking_lot::RwLock::new(None)),
            default_model: Arc::new(parking_lot::RwLock::new(None)),
            operation_defaults: Arc::new(DashMap::new()),
            operation_models: Arc::new(DashMap::new()),
            fallback_chains: Arc::new(DashMap::new()),
            model_aliases: Arc::new(DashMap::new()),
            model_routes: Arc::new(DashMap::new()),
            health_monitor: Arc::new(HealthMonitor::new()),
            routing_strategy: RoutingStrategy::default(),
            #[cfg(feature = "metrics")]
            metrics: crate::llm::MetricsTracker::new(),
            rate_limiter: Arc::new(rate_limiter),
            round_robin_counter: Arc::new(AtomicUsize::new(0)),
            backend_providers: Arc::new(DashMap::new()),
            pin_peaks: Arc::new(DashMap::new()),
        })
    }

    /// Create registry with custom routing strategy.
    pub fn with_strategy(routing_strategy: RoutingStrategy) -> Self {
        let rate_limiter =
            RateLimiter::new(HashMap::new(), Arc::new(SystemClock)).expect("empty config is valid");

        LLMRegistry {
            backends: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            default_backend: Arc::new(parking_lot::RwLock::new(None)),
            default_model: Arc::new(parking_lot::RwLock::new(None)),
            operation_defaults: Arc::new(DashMap::new()),
            operation_models: Arc::new(DashMap::new()),
            fallback_chains: Arc::new(DashMap::new()),
            model_aliases: Arc::new(DashMap::new()),
            model_routes: Arc::new(DashMap::new()),
            health_monitor: Arc::new(HealthMonitor::new()),
            routing_strategy,
            #[cfg(feature = "metrics")]
            metrics: crate::llm::MetricsTracker::new(),
            rate_limiter: Arc::new(rate_limiter),
            round_robin_counter: Arc::new(AtomicUsize::new(0)),
            backend_providers: Arc::new(DashMap::new()),
            pin_peaks: Arc::new(DashMap::new()),
        }
    }

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> &crate::llm::MetricsTracker {
        &self.metrics
    }

    /// Resolve the backend name a request would dispatch to, without taking a
    /// backend handle. Used by the streaming path so it can record health and
    /// metrics outcomes against the same backend the stream actually used.
    pub fn resolved_backend_name(&self, request: &LLMRequest) -> Result<String> {
        self.resolve_backend(request)
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
        backend_model: &str,
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
            self.health_monitor.record_success(backend_name, latency);
        } else {
            self.health_monitor.record_failure(backend_name, latency);
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

    /// Unregister a backend.
    pub fn unregister(&self, name: &str) -> Result<()> {
        self.backends
            .write()
            .remove(name)
            .with_context(|| format!("Backend '{}' not found", name))?;

        self.health_monitor.unregister_backend(name);

        Ok(())
    }

    /// Set the default backend for requests.
    pub fn set_default(&self, name: impl Into<String>) -> Result<()> {
        let name = name.into();

        // Verify backend exists
        if !self.backends.read().contains_key(&name) {
            anyhow::bail!("Backend '{}' not registered", name);
        }

        *self.default_backend.write() = Some(name);
        Ok(())
    }

    /// Set the default model to apply when a request omits `model`.
    pub fn set_default_model(&self, model: impl Into<String>) {
        *self.default_model.write() = Some(model.into());
    }

    /// Set operation-specific backend default.
    pub fn set_operation_default(
        &self,
        operation: AISOperationType,
        backend: impl Into<String>,
    ) -> Result<()> {
        let backend_name = backend.into();

        // Verify backend exists
        if !self.backends.read().contains_key(&backend_name) {
            anyhow::bail!("Backend '{}' not registered", backend_name);
        }

        self.operation_defaults.insert(operation, backend_name);
        Ok(())
    }

    /// Set operation-specific model default.
    pub fn set_operation_model(&self, operation: AISOperationType, model: impl Into<String>) {
        self.operation_models.insert(operation, model.into());
    }

    /// Set fallback chain for a backend.
    pub fn set_fallback(&self, backend: impl Into<String>, fallbacks: Vec<String>) -> Result<()> {
        let backend_name = backend.into();

        // Verify all backends exist
        {
            let backends = self.backends.read();
            if !backends.contains_key(&backend_name) {
                anyhow::bail!("Backend '{}' not registered", backend_name);
            }
            for fallback in &fallbacks {
                if !backends.contains_key(fallback) {
                    anyhow::bail!("Fallback backend '{}' not registered", fallback);
                }
            }
        }

        self.fallback_chains.insert(backend_name, fallbacks);
        Ok(())
    }

    /// Record which provider protocol a backend uses.
    ///
    /// Used for per-provider builtin model fallback when no model is specified.
    pub fn register_backend_provider(
        &self,
        backend: impl Into<String>,
        provider: ProviderProtocol,
    ) {
        self.backend_providers.insert(backend.into(), provider);
    }

    /// Register a named model alias.
    pub fn register_model_alias(&self, alias: impl Into<String>, model: impl Into<String>) {
        self.model_aliases.insert(alias.into(), model.into());
    }

    /// Route a model name or alias to a specific backend.
    pub fn set_model_route(
        &self,
        model_or_alias: impl Into<String>,
        backend: impl Into<String>,
    ) -> Result<()> {
        let backend_name = backend.into();
        if !self.backends.read().contains_key(&backend_name) {
            anyhow::bail!("Backend '{}' not registered", backend_name);
        }
        self.model_routes
            .insert(model_or_alias.into(), backend_name);
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

    /// Normalize model/backend selection for a request using the configured policy.
    pub fn prepare_request(&self, request: &LLMRequest) -> LLMRequest {
        let mut prepared = request.clone();

        if prepared.model.is_none() {
            if let Some(operation) = prepared.operation_type
                && let Some(entry) = self.operation_models.get(&operation)
            {
                prepared.model = Some(entry.value().clone());
            } else if let Some(default_model) = self.default_model.read().clone() {
                prepared.model = Some(default_model);
            } else if let Some(ref backend_name) = prepared.backend {
                // Per-provider builtin fallback
                if let Some(provider) = self.backend_providers.get(backend_name) {
                    if let Some(builtin) = default_model_for_protocol(*provider.value()) {
                        prepared.model = Some(builtin.to_string());
                    }
                }
            }
        }

        if let Some(model) = prepared.model.clone() {
            let canonical_model = self
                .model_aliases
                .get(&model)
                .map(|entry| entry.value().clone())
                .unwrap_or(model.clone());

            if prepared.backend.is_none() {
                if let Some(entry) = self.model_routes.get(&model) {
                    prepared.backend = Some(entry.value().clone());
                } else if let Some(entry) = self.model_routes.get(&canonical_model) {
                    prepared.backend = Some(entry.value().clone());
                }
            }

            prepared.model = Some(canonical_model);
        }

        prepared
    }

    /// Generate a response using intelligent routing.
    pub async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        let request = self.prepare_request(&request);
        // Resolve which backend to use
        let backend_name = self.resolve_backend(&request)?;

        // Try primary backend
        match self.try_generate(&backend_name, request.clone()).await {
            Ok(response) => Ok(response),
            Err(e) => {
                // Check for fallback chain
                if let Some(fallbacks) = self.fallback_chains.get(&backend_name) {
                    for fallback_name in fallbacks.value() {
                        match self.try_generate(fallback_name, request.clone()).await {
                            Ok(response) => {
                                tracing::info!(
                                    "Fallback successful: {} -> {}",
                                    backend_name,
                                    fallback_name
                                );
                                return Ok(response);
                            }
                            Err(fallback_err) => {
                                tracing::warn!(
                                    "Fallback '{}' failed: {}",
                                    fallback_name,
                                    fallback_err
                                );
                                continue;
                            }
                        }
                    }
                }

                tracing::error!(
                    backend = %backend_name,
                    error = ?e,
                    "LLM backend request failed"
                );
                Err(e).context(format!(
                    "Request failed on '{}' with no successful fallback",
                    backend_name
                ))
            }
        }
    }

    /// Generate using a specific backend by name.
    pub async fn generate_with_backend(
        &self,
        backend_name: &str,
        request: LLMRequest,
    ) -> Result<LLMResponse> {
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

        // Estimate token cost from max_tokens (defaults to 1.0 for backward compatibility)
        let estimated_cost = request.max_tokens.map(|t| t as f64).unwrap_or(1.0);

        // Check rate limit before dispatching to backend
        self.rate_limiter
            .check_and_consume(backend_name, estimated_cost)
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
                // Reconcile rate limit based on actual token usage
                let actual_cost = response.usage.total_tokens as f64;
                self.rate_limiter
                    .reconcile(backend_name, estimated_cost, actual_cost);

                // Record success
                self.health_monitor.record_success(backend_name, latency);
                Ok(response)
            }
            Err(e) => {
                // Record failure
                self.health_monitor.record_failure(backend_name, latency);
                Err(e)
            }
        }
    }

    /// Resolve a backend for streaming.
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

        Ok(backend)
    }

    /// Generate streaming response with fallback on first-chunk error.
    ///
    /// Tries primary backend stream. If the first chunk errors, drops the stream
    /// and retries with the fallback backend. Once the first chunk succeeds,
    /// commits to that backend (no mid-stream switching).
    pub fn generate_stream_with_fallback<'a>(
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

            let backend = {
                let guard = self.backends.read();
                guard.get(&backend_name).cloned()
            };
            let backend = match backend {
                Some(b) => b,
                None => {
                    Err(anyhow::anyhow!("Backend '{}' not found", backend_name))?;
                    return;
                }
            };

            // Try primary backend stream
            let mut stream = backend.generate_stream(request.clone());

            match stream.next().await {
                Some(Ok(first_chunk)) => {
                    // First chunk succeeded, commit to this stream
                    yield first_chunk;

                    // Stream remaining chunks
                    while let Some(chunk) = stream.next().await {
                        yield chunk?;
                    }
                }
                Some(Err(e)) => {
                    // First chunk failed, try fallback
                    tracing::warn!("Primary backend '{}' streaming failed: {}. Attempting fallback.", backend_name, e);
                    self.health_monitor.record_failure(&backend_name, std::time::Duration::from_secs(0));

                    // Get fallback backends from fallback chain
                    let mut fallback_succeeded = false;
                    if let Some(fallback_chain) = self.fallback_chains.get(&backend_name) {
                        for fallback_name in fallback_chain.value() {
                            let fallback_backend = {
                                let guard = self.backends.read();
                                guard.get(fallback_name).cloned()
                            };
                            if let Some(fallback_backend) = fallback_backend {
                                let health = self.health_monitor.status(fallback_name);
                                if health == HealthStatus::Unhealthy {
                                    continue;
                                }

                                tracing::info!("Retrying with fallback backend: {}", fallback_name);
                                let mut fallback_stream = fallback_backend.generate_stream(request.clone());

                                match fallback_stream.next().await {
                                    Some(Ok(first_chunk)) => {
                                        yield first_chunk;
                                        while let Some(chunk) = fallback_stream.next().await {
                                            yield chunk?;
                                        }
                                        fallback_succeeded = true;
                                        break;
                                    }
                                    Some(Err(fe)) => {
                                        tracing::warn!("Fallback backend '{}' also failed: {}", fallback_name, fe);
                                        self.health_monitor.record_failure(fallback_name, std::time::Duration::from_secs(0));
                                        continue;
                                    }
                                    None => {
                                        tracing::warn!("Fallback backend '{}' returned empty stream", fallback_name);
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    if !fallback_succeeded {
                        Err(anyhow::anyhow!("All backends failed for streaming request: {}", e))?;
                    }
                }
                None => {
                    Err(anyhow::anyhow!("Backend '{}' returned empty stream", backend_name))?;
                }
            }
        })
    }

    /// Resolve the backend name that would handle this request after policy normalization.
    pub fn resolve_backend_name(&self, request: &LLMRequest) -> Result<String> {
        let prepared = self.prepare_request(request);
        self.resolve_backend(&prepared)
    }

    /// Resolve which backend to use for a request.
    fn resolve_backend(&self, request: &LLMRequest) -> Result<String> {
        // Use resolver to determine backend
        let criteria = SelectionCriteria::from_request(request);
        resolver::resolve(
            &criteria,
            &self.backends,
            &self.operation_defaults,
            &self.default_backend,
            &self.health_monitor,
            &self.routing_strategy,
            &self.round_robin_counter,
        )
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
        if aggregate.total_requests == 0
            && per_backend.is_empty()
            && graph_status_snapshots.is_empty()
        {
            return None;
        }
        Some(crate::llm::observability::BackendMetricsSource {
            aggregate,
            per_backend,
            graph_status_snapshots,
        })
    }

    /// Perform health checks on all backends.
    pub async fn check_all_backends(&self) -> HashMap<String, HealthStatus> {
        let mut results = HashMap::new();

        for (name, backend) in self.backend_snapshot() {
            let status = match backend.health_check().await {
                Ok(_) => HealthStatus::Healthy,
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
mod tests {
    use super::*;
    use crate::llm::backends::openai::OpenAIBackend;
    use crate::llm::provider::Provider;

    #[tokio::test]
    async fn test_registry_registration() -> Result<(), Box<dyn std::error::Error>> {
        let registry = LLMRegistry::new();

        // Register using the typed provider enum.
        let backend = Provider::OpenAI(OpenAIBackend::new("test-key", None).await?);
        registry.register("test", backend)?;

        assert_eq!(registry.backend_names(), vec!["test"]);
        assert!(registry.get_backend("test").is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_registry_register_arc() -> Result<(), Box<dyn std::error::Error>> {
        let registry = LLMRegistry::new();

        // Register using Arc<dyn LLMBackend>
        let backend: Arc<dyn LLMBackend> = Arc::new(OpenAIBackend::new("test-key", None).await?);
        registry.register_arc("test", backend)?;

        assert_eq!(registry.backend_names(), vec!["test"]);
        assert!(registry.get_backend("test").is_some());

        Ok(())
    }

    #[test]
    fn test_registry_defaults() {
        let registry = LLMRegistry::new();

        // Can't set default for non-existent backend
        assert!(registry.set_default("nonexistent").is_err());
    }

    #[test]
    fn test_fallback_chain_validation() {
        let registry = LLMRegistry::new();

        // Can't set fallback for non-existent backend
        assert!(
            registry
                .set_fallback("nonexistent", vec!["other".to_string()])
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_prepare_request_applies_alias_and_route() -> Result<(), Box<dyn std::error::Error>>
    {
        let registry = LLMRegistry::new();
        let backend = Provider::OpenAI(OpenAIBackend::new("test-key", None).await?);
        registry.register("primary", backend)?;
        registry.register_model_alias("fast", "gpt-4o-mini");
        registry.set_model_route("fast", "primary")?;

        let request = LLMRequest::new("hello").with_model("fast");
        let prepared = registry.prepare_request(&request);

        assert_eq!(prepared.backend.as_deref(), Some("primary"));
        assert_eq!(prepared.model.as_deref(), Some("gpt-4o-mini"));
        Ok(())
    }

    #[tokio::test]
    async fn test_prepare_request_applies_operation_model() -> Result<(), Box<dyn std::error::Error>>
    {
        let registry = LLMRegistry::new();
        let backend = Provider::OpenAI(OpenAIBackend::new("test-key", None).await?);
        registry.register("primary", backend)?;
        registry.set_default("primary")?;
        registry.set_operation_model(AISOperationType::Plan, "gpt-4.1");

        let request = LLMRequest::new("hello").with_operation_type(AISOperationType::Plan);
        let prepared = registry.prepare_request(&request);

        assert_eq!(prepared.model.as_deref(), Some("gpt-4.1"));
        Ok(())
    }

    #[tokio::test]
    async fn test_resolve_backend_name_uses_registered_route()
    -> Result<(), Box<dyn std::error::Error>> {
        let registry = LLMRegistry::new();
        let backend = Provider::OpenAI(OpenAIBackend::new("test-key", None).await?);
        registry.register("primary", backend)?;
        registry.register_model_alias("fast", "gpt-4o-mini");
        registry.set_model_route("fast", "primary")?;

        let request = LLMRequest::new("hello").with_model("fast");
        let backend_name = registry.resolve_backend_name(&request)?;

        assert_eq!(backend_name, "primary");
        Ok(())
    }

    #[tokio::test]
    async fn test_rate_limiting_with_token_based_config() -> Result<(), Box<dyn std::error::Error>>
    {
        use crate::llm::rate_limit::RateLimitConfig;
        use std::collections::HashMap;

        let mut rate_limits = HashMap::new();
        rate_limits.insert(
            "test".to_string(),
            RateLimitConfig {
                capacity: 1000,
                tokens_per_second: 100.0,
                token_based: true,
                default_token_estimate: 100.0,
            },
        );

        let registry = LLMRegistry::with_rate_limits(rate_limits)?;
        let backend = Provider::OpenAI(OpenAIBackend::new("test-key", None).await?);
        registry.register("test", backend)?;

        // This test verifies that the registry accepts token-based rate limit configs
        // Actual behavior is tested in rate_limit.rs
        assert!(registry.get_backend("test").is_some());
        Ok(())
    }
}
