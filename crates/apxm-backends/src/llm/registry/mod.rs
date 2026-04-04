//! Multi-backend registry with intelligent routing.
//!
//! Provides backend registration, intelligent routing, health monitoring,
//! and fallback chain execution for robust LLM request handling.

#[cfg(feature = "metrics")]
use crate::llm::RequestMetrics;
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse};
use crate::llm::rate_limit::{RateLimitConfig, RateLimiter, SystemClock};
use anyhow::{Context as AnyhowContext, Result};
use apxm_core::types::AISOperationType;
#[cfg(feature = "metrics")]
use apxm_core::types::TokenUsage;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

mod health;
mod resolver;

pub use health::{HealthMonitor, HealthStatus};
pub use resolver::{RoutingStrategy, SelectionCriteria};

/// LLM Registry manages multiple backends with routing and fallback.
///
/// Backends are stored as `Arc<dyn LLMBackend>`, allowing both enum-based
/// `Provider` instances and custom trait implementations to be registered.
#[derive(Clone)]
pub struct LLMRegistry {
    /// Registered backends by name
    backends: Arc<DashMap<String, Arc<dyn LLMBackend>>>,
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
            backends: Arc::new(DashMap::new()),
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
        })
    }

    /// Create registry with custom routing strategy.
    pub fn with_strategy(routing_strategy: RoutingStrategy) -> Self {
        let rate_limiter =
            RateLimiter::new(HashMap::new(), Arc::new(SystemClock)).expect("empty config is valid");

        LLMRegistry {
            backends: Arc::new(DashMap::new()),
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
        }
    }

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> &crate::llm::MetricsTracker {
        &self.metrics
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

        self.backends.insert(name.clone(), backend);
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
        self.backends.insert(name.clone(), backend);
        self.health_monitor.register_backend(&name);
        Ok(())
    }

    /// Unregister a backend.
    pub fn unregister(&self, name: &str) -> Result<()> {
        self.backends
            .remove(name)
            .with_context(|| format!("Backend '{}' not found", name))?;

        self.health_monitor.unregister_backend(name);

        Ok(())
    }

    /// Set the default backend for requests.
    pub fn set_default(&self, name: impl Into<String>) -> Result<()> {
        let name = name.into();

        // Verify backend exists
        if !self.backends.contains_key(&name) {
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
        if !self.backends.contains_key(&backend_name) {
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
        if !self.backends.contains_key(&backend_name) {
            anyhow::bail!("Backend '{}' not registered", backend_name);
        }

        for fallback in &fallbacks {
            if !self.backends.contains_key(fallback) {
                anyhow::bail!("Fallback backend '{}' not registered", fallback);
            }
        }

        self.fallback_chains.insert(backend_name, fallbacks);
        Ok(())
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
        if !self.backends.contains_key(&backend_name) {
            anyhow::bail!("Backend '{}' not registered", backend_name);
        }
        self.model_routes
            .insert(model_or_alias.into(), backend_name);
        Ok(())
    }

    /// Get registered backend names.
    pub fn backend_names(&self) -> Vec<String> {
        self.backends
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get backend by name.
    pub fn get_backend(&self, name: &str) -> Option<Arc<dyn LLMBackend>> {
        self.backends.get(name).map(|entry| entry.value().clone())
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
            .get(backend_name)
            .with_context(|| format!("Backend '{}' not found", backend_name))?
            .clone();

        // Check health status
        let health = self.health_monitor.status(backend_name);
        if health == HealthStatus::Unhealthy {
            anyhow::bail!("Backend '{}' is unhealthy", backend_name);
        }

        // Check rate limit before dispatching to backend
        self.rate_limiter
            .check_and_consume(backend_name)
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
            .get(&backend_name)
            .with_context(|| format!("Backend '{}' not found", backend_name))?
            .clone();

        let health = self.health_monitor.status(&backend_name);
        if health == HealthStatus::Unhealthy {
            anyhow::bail!("Backend '{}' is unhealthy", backend_name);
        }

        Ok(backend)
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
        )
    }

    /// Perform health checks on all backends.
    pub async fn check_all_backends(&self) -> HashMap<String, HealthStatus> {
        let mut results = HashMap::new();

        for entry in self.backends.iter() {
            let name = entry.key().clone();
            let backend = entry.value();

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

        // Register using Provider enum (backward compatible)
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
}
