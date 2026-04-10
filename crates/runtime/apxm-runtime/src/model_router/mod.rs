//! ModelRouter — dynamic model selection at runtime.
//!
//! The ModelRouter sits between the executor's LLM dispatch path and the
//! `LLMRegistry`. It adds:
//!
//! - **Circuit breakers** per backend (Closed → Open → HalfOpen)
//! - **Per-backend rate limiting** with in-memory token buckets
//! - **Policy-driven routing** (prefer tags, cost target, latency target)
//! - **Config-driven model registry** (`~/.apxm/models.toml`)
//! - **Automatic failover** to the next available backend
//!
//! ## Integration
//!
//! `ModelRouter` wraps an `Arc<LLMRegistry>` and should be stored on the
//! `ExecutionContext`. The LLM handler calls `router.select_backend(request)`
//! to get the concrete backend name, then delegates to the registry as before.
//! On success/failure the handler calls `router.record_success/failure(name)`.
//!
//! ## Routing priority
//!
//! 1. Explicit `request.backend` (bypasses router logic)
//! 2. Explicit `request.model` → model-to-backend mapping in registry
//! 3. Operation-type policy
//! 4. Tag-based policy from `RoutingConfig`
//! 5. Default model/backend from `DefaultsConfig`
//! 6. First available healthy backend

pub mod health;
pub mod profile_registry;
pub mod profile_router;
pub mod rate_limit;
pub mod registry;

pub use health::{BackendHealth, CircuitBreakerConfig, CircuitBreakerRegistry, CircuitState};
pub use profile_registry::ProfileRegistry;
pub use profile_router::ProfileRouter;
pub use rate_limit::{RateLimitConfig, RateLimitConfigError, RateLimitError};
pub use registry::{ModelEntry, ModelRegistry, RoutingConfig};

use self::rate_limit::{RateLimiter, SystemClock};
use apxm_backends::{LLMRegistry, LLMRequest, LLMResponse};
use apxm_core::types::AISOperationType;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Optimization target that influences model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RoutingTarget {
    /// Optimize for cost (prefer cheapest models).
    Cost,
    /// Optimize for latency (prefer fastest models).
    Latency,
    /// Optimize for capability (prefer most capable models).
    Quality,
    /// Balanced (default behaviour, respects config tags).
    #[default]
    Balanced,
}

/// Per-operation-type routing override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationPolicy {
    /// Operation type this policy applies to.
    pub operation: AISOperationType,
    /// Preferred model name (overrides global default).
    pub model: Option<String>,
    /// Preferred backend name.
    pub backend: Option<String>,
    /// Routing target for this operation.
    pub target: RoutingTarget,
}

/// Configuration for the ModelRouter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouterConfig {
    /// Circuit breaker settings.
    pub circuit_breaker: CircuitBreakerConfig,
    /// Optional global rate limit config (applied to all backends).
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    /// If true, use per-backend keying only. If false, use per-backend-per-caller when caller is available.
    #[serde(default)]
    pub per_backend_only: bool,
    /// Global routing target.
    pub target: RoutingTarget,
    /// Per-operation policies.
    #[serde(default)]
    pub operation_policies: Vec<OperationPolicy>,
}

impl Default for ModelRouterConfig {
    fn default() -> Self {
        ModelRouterConfig {
            circuit_breaker: CircuitBreakerConfig::default(),
            rate_limit: None,
            per_backend_only: true,
            target: RoutingTarget::Balanced,
            operation_policies: Vec::new(),
        }
    }
}

/// Result of a routing decision.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    /// Chosen backend name.
    pub backend: String,
    /// Resolved model name (may differ from request if alias applied).
    pub model: Option<String>,
    /// Whether this decision was constrained by circuit-breaker state.
    pub was_failover: bool,
}

/// The ModelRouter — dynamic model and backend selector.
///
/// Wraps `LLMRegistry` + `ModelRegistry` + `CircuitBreakerRegistry` to provide
/// policy-aware, resilient LLM dispatch.
#[derive(Clone)]
pub struct ModelRouter {
    /// Underlying LLM backend registry.
    llm_registry: Arc<LLMRegistry>,
    /// Model definitions loaded from config.
    model_registry: Arc<ModelRegistry>,
    /// Circuit breakers per backend.
    circuit_breakers: Arc<CircuitBreakerRegistry>,
    /// In-memory token buckets keyed by backend id.
    rate_limiter: Arc<RateLimiter<SystemClock>>,
    /// Router configuration.
    config: ModelRouterConfig,
}

impl ModelRouter {
    /// Create a new router backed by an existing `LLMRegistry`.
    pub fn new(llm_registry: Arc<LLMRegistry>, config: ModelRouterConfig) -> anyhow::Result<Self> {
        let model_registry = Arc::new(ModelRegistry::load_from_default_path());
        Self::with_model_registry(llm_registry, model_registry, config)
    }

    /// Create with a pre-built `ModelRegistry` (useful for testing).
    pub fn with_model_registry(
        llm_registry: Arc<LLMRegistry>,
        model_registry: Arc<ModelRegistry>,
        config: ModelRouterConfig,
    ) -> anyhow::Result<Self> {
        let circuit_breakers =
            Arc::new(CircuitBreakerRegistry::new(config.circuit_breaker.clone()));
        for name in llm_registry.backend_names() {
            circuit_breakers.register(&name);
        }

        let rate_limiter = Arc::new(RateLimiter::new(config.rate_limit.clone().unwrap_or(
            RateLimitConfig {
                capacity: 60,
                refill_tokens: 60,
                refill_interval: Duration::from_secs(60),
            },
        ))?);

        Ok(ModelRouter {
            llm_registry,
            model_registry,
            circuit_breakers,
            rate_limiter,
            config,
        })
    }

    /// Select a backend + model for the given request, applying policy and circuit breakers.
    ///
    /// Returns a `RoutingDecision` describing the chosen backend/model.
    /// Returns an error if no backend is available.
    pub fn select(&self, request: &LLMRequest) -> anyhow::Result<RoutingDecision> {
        // 1. Explicit backend in request → honour it if the breaker allows.
        if let Some(ref backend) = request.backend {
            if self.circuit_breakers.is_available(backend) {
                return Ok(RoutingDecision {
                    backend: backend.clone(),
                    model: request.model.clone(),
                    was_failover: false,
                });
            }
            // Breaker tripped — fall through to policy routing.
            tracing::warn!(
                backend = %backend,
                "Requested backend tripped; falling back to policy routing"
            );
        }

        // 2. Explicit model in request → resolve to backend via model registry.
        if let Some(ref model_name) = request.model {
            if let Some(entry) = self.model_registry.get(model_name) {
                if self.circuit_breakers.is_available(&entry.backend) {
                    return Ok(RoutingDecision {
                        backend: entry.backend.clone(),
                        model: Some(model_name.clone()),
                        was_failover: false,
                    });
                }
                tracing::warn!(
                    model = %model_name,
                    backend = %entry.backend,
                    "Model's backend tripped; falling back to policy routing"
                );
            }
        }

        // 3. Operation-type policy.
        let op_policy = request
            .operation_type
            .as_ref()
            .and_then(|op| self.operation_policy(op));

        if let Some(policy) = &op_policy {
            if let Some(ref backend) = policy.backend {
                if self.circuit_breakers.is_available(backend) {
                    return Ok(RoutingDecision {
                        backend: backend.clone(),
                        model: policy.model.clone().or_else(|| request.model.clone()),
                        was_failover: false,
                    });
                }
            }
        }

        // 4. Tag-based routing from RoutingConfig.
        let routing = self.model_registry.routing();
        for tag in &routing.prefer_tags {
            if let Some(decision) = self.find_by_tag(tag) {
                return Ok(decision);
            }
        }

        // 5. Default model/backend from ModelRegistry config.
        if let Some(backend) = self.model_registry.default_backend() {
            if self.circuit_breakers.is_available(&backend) {
                return Ok(RoutingDecision {
                    backend: backend.clone(),
                    model: self.model_registry.default_model(),
                    was_failover: false,
                });
            }
        }

        // 6. First available healthy backend from LLMRegistry.
        let all_backends = self.llm_registry.backend_names();
        for backend in &all_backends {
            if self.circuit_breakers.is_available(backend) {
                return Ok(RoutingDecision {
                    backend: backend.clone(),
                    model: request.model.clone(),
                    was_failover: all_backends.len() > 1,
                });
            }
        }

        // 7. Fallback tags from RoutingConfig.
        for tag in &routing.fallback_tags {
            if let Some(decision) = self.find_by_tag(tag) {
                return Ok(RoutingDecision {
                    was_failover: true,
                    ..decision
                });
            }
        }

        anyhow::bail!("ModelRouter: no available backend (all circuit breakers open)")
    }

    /// Select a backend and consume one rate-limit token before dispatch.
    pub(crate) async fn select_for_dispatch(
        &self,
        request: &LLMRequest,
    ) -> anyhow::Result<RoutingDecision> {
        let decision = self.select(request)?;
        let backend_id = &decision.backend;
        let key = format!("backend:{}", backend_id);
        self.rate_limiter.check(key).await?;
        Ok(decision)
    }

    #[cfg(test)]
    async fn select_for_dispatch_with_rate_limiter<C: rate_limit::Clock>(
        &self,
        request: &LLMRequest,
        rate_limiter: &RateLimiter<C>,
    ) -> anyhow::Result<RoutingDecision> {
        let decision = self.select(request)?;
        let backend_id = &decision.backend;
        let key = format!("backend:{}", backend_id);
        rate_limiter.check(key).await?;
        Ok(decision)
    }

    /// Record a successful LLM call to a backend.
    pub fn record_success(&self, backend: &str) {
        self.circuit_breakers.record_success(backend);
    }

    /// Record a failed LLM call to a backend.
    pub fn record_failure(&self, backend: &str) {
        self.circuit_breakers.record_failure(backend);
    }

    /// Get circuit-breaker health for all backends.
    pub fn all_health(&self) -> Vec<BackendHealth> {
        self.circuit_breakers.all_health()
    }

    /// Get circuit-breaker health for a single backend.
    pub fn health(&self, backend: &str) -> Option<BackendHealth> {
        self.circuit_breakers.health(backend)
    }

    /// Manually reset the circuit breaker for a backend.
    pub fn reset_circuit(&self, backend: &str) {
        self.circuit_breakers.reset(backend);
        tracing::info!(backend = %backend, "Circuit breaker manually reset");
    }

    /// Sync health status from LLM registry's health monitor to circuit breakers.
    ///
    /// Queries the backends' health monitor and trips/resets circuit breakers
    /// based on health status. Should be called periodically or after significant
    /// operations to keep circuit breakers in sync with backend health.
    pub fn sync_health(&self) {
        use apxm_backends::HealthStatus;

        for backend_name in self.llm_registry.backend_names() {
            let health_status = self.llm_registry.backend_health(&backend_name);

            match health_status {
                HealthStatus::Unhealthy => {
                    // Backend is unhealthy, ensure circuit breaker reflects this
                    if self.circuit_breakers.is_available(&backend_name) {
                        tracing::warn!(
                            backend = %backend_name,
                            "Health monitor reports unhealthy status, recording failure in circuit breaker"
                        );
                        self.circuit_breakers.record_failure(&backend_name);
                    }
                }
                HealthStatus::Healthy => {
                    // Backend is healthy, ensure circuit is not tripped due to stale failures
                    // Note: We don't force-close circuits here to avoid bypassing the
                    // circuit breaker's own logic (e.g., half-open state, probe requests)
                }
                HealthStatus::Degraded | HealthStatus::Unknown => {
                    // Degraded or unknown: don't change circuit state
                }
            }
        }
    }

    /// Access the underlying model registry.
    pub fn model_registry(&self) -> &ModelRegistry {
        &self.model_registry
    }

    /// Access the underlying LLM registry.
    pub fn llm_registry(&self) -> &LLMRegistry {
        &self.llm_registry
    }

    /// Check if a model is healthy (circuit breaker closed for its backend).
    ///
    /// Returns true if the model's backend has a closed circuit breaker,
    /// false if open or model not found in registry.
    ///
    /// This is used by ProfileRouter to select healthy candidates.
    pub fn is_model_healthy(&self, model_name: &str) -> bool {
        // Look up the model in the registry to find its backend
        if let Some(entry) = self.model_registry.get(model_name) {
            // Check if the backend's circuit breaker is available
            self.circuit_breakers.is_available(&entry.backend)
        } else {
            // Model not found in registry, consider it unavailable
            false
        }
    }

    /// Execute an LLM request using the router's selection logic.
    ///
    /// Wraps `LLMRegistry::generate_with_backend` and records circuit-breaker
    /// outcomes automatically.
    pub async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse> {
        let decision = self.select_for_dispatch(&request).await?;
        let backend_name = decision.backend.clone();

        // Apply resolved model to request if different.
        let prepared = if let Some(ref model) = decision.model {
            if request.model.as_deref() != Some(model) {
                request.with_model(model.clone())
            } else {
                request
            }
        } else {
            request
        };

        match self
            .llm_registry
            .generate_with_backend(&backend_name, prepared)
            .await
        {
            Ok(response) => {
                self.record_success(&backend_name);
                Ok(response)
            }
            Err(e) => {
                self.record_failure(&backend_name);
                Err(e)
            }
        }
    }

    // ── helpers ──────────────────────────────────────────────────────────

    fn operation_policy(&self, op: &AISOperationType) -> Option<&OperationPolicy> {
        self.config
            .operation_policies
            .iter()
            .find(|p| &p.operation == op)
    }

    fn find_by_tag(&self, tag: &str) -> Option<RoutingDecision> {
        let models = self.model_registry.models_with_tags(&[tag]);
        for model in models {
            if self.circuit_breakers.is_available(&model.backend) {
                return Some(RoutingDecision {
                    backend: model.backend.clone(),
                    model: Some(model.name.clone()),
                    was_failover: false,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_backends::LLMRegistry;
    use registry::ModelEntry;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[derive(Debug)]
    struct TestClock {
        now: Mutex<Instant>,
    }

    impl TestClock {
        fn new(start: Instant) -> Self {
            Self {
                now: Mutex::new(start),
            }
        }

        fn advance(&self, duration: Duration) {
            let mut guard = self.now.lock().unwrap();
            *guard += duration;
        }
    }

    impl super::rate_limit::Clock for TestClock {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }
    }

    fn make_router() -> ModelRouter {
        let llm_registry = Arc::new(LLMRegistry::new());
        let model_registry = Arc::new(ModelRegistry::new());
        ModelRouter::with_model_registry(llm_registry, model_registry, ModelRouterConfig::default())
            .unwrap()
    }

    #[test]
    fn test_no_backends_returns_error() {
        let router = make_router();
        let request = LLMRequest::new("hello");
        assert!(router.select(&request).is_err());
    }

    #[test]
    fn test_model_registry_lookup() {
        let llm_registry = Arc::new(LLMRegistry::new());
        let model_registry = Arc::new(ModelRegistry::new());
        model_registry.register(ModelEntry {
            name: "fast-model".to_string(),
            backend: "openai".to_string(),
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            context_window: 8_000,
            tags: vec!["fast".to_string()],
            supports_thinking: false,
            max_output_tokens: None,
        });

        let router = ModelRouter::with_model_registry(
            llm_registry,
            model_registry,
            ModelRouterConfig::default(),
        )
        .unwrap();

        // Register circuit breaker for the backend
        router.circuit_breakers.register("openai");

        let request = LLMRequest::new("hi").with_model("fast-model");
        let decision = router.select(&request).unwrap();
        assert_eq!(decision.backend, "openai");
        assert_eq!(decision.model.as_deref(), Some("fast-model"));
    }

    #[test]
    fn test_circuit_breaker_failover() {
        let llm_registry = Arc::new(LLMRegistry::new());
        let model_registry = Arc::new(ModelRegistry::new());
        model_registry.register(ModelEntry {
            name: "model-a".to_string(),
            backend: "backend-a".to_string(),
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            context_window: 8_000,
            tags: vec!["primary".to_string()],
            supports_thinking: false,
            max_output_tokens: None,
        });

        let config = ModelRouterConfig {
            circuit_breaker: CircuitBreakerConfig {
                failure_threshold: 1,
                open_duration: std::time::Duration::from_secs(60),
                min_requests: 1,
            },
            ..Default::default()
        };

        let router =
            ModelRouter::with_model_registry(llm_registry, model_registry, config.clone()).unwrap();

        // Register the breaker and trip it
        router.circuit_breakers.register("backend-a");
        router.circuit_breakers.record_failure("backend-a");

        // backend-a is now open; request for model-a should fail (no fallback)
        let request = LLMRequest::new("hi").with_model("model-a");
        let result = router.select(&request);
        assert!(result.is_err());
    }

    #[test]
    fn test_record_success_closes_circuit() {
        let router = make_router();
        router.circuit_breakers.register("mybackend");

        // Trip it
        for _ in 0..5 {
            router.record_failure("mybackend");
        }
        assert!(!router.circuit_breakers.is_available("mybackend"));

        // Reset and record success
        router.reset_circuit("mybackend");
        router.record_success("mybackend");
        assert!(router.circuit_breakers.is_available("mybackend"));
    }

    #[test]
    fn test_all_health_returns_entries() {
        let router = make_router();
        router.circuit_breakers.register("b1");
        router.circuit_breakers.register("b2");
        let health = router.all_health();
        assert_eq!(health.len(), 2);
    }

    #[test]
    fn test_explicit_backend_honoured() {
        let router = make_router();
        router.circuit_breakers.register("explicit");

        let request = LLMRequest::new("hello").with_backend("explicit");
        let decision = router.select(&request).unwrap();
        assert_eq!(decision.backend, "explicit");
        assert!(!decision.was_failover);
    }

    fn make_router_with_backends(backends: &[&str]) -> ModelRouter {
        let router = make_router();
        for backend in backends {
            router.circuit_breakers.register(backend);
        }
        router
    }

    #[tokio::test]
    async fn test_request_allowed_when_under_limit() {
        let router = make_router_with_backends(&["backend-a"]);
        let clock = Arc::new(TestClock::new(Instant::now()));
        let limiter = RateLimiter::with_clock(
            RateLimitConfig {
                capacity: 1,
                refill_tokens: 1,
                refill_interval: Duration::from_secs(60),
            },
            clock,
        )
        .unwrap();

        let request = LLMRequest::new("hello").with_backend("backend-a");
        let decision = router
            .select_for_dispatch_with_rate_limiter(&request, &limiter)
            .await
            .unwrap();

        assert_eq!(decision.backend, "backend-a");
    }

    #[tokio::test]
    async fn test_request_rejected_when_over_limit() {
        let router = make_router_with_backends(&["backend-a"]);
        let clock = Arc::new(TestClock::new(Instant::now()));
        let limiter = RateLimiter::with_clock(
            RateLimitConfig {
                capacity: 1,
                refill_tokens: 1,
                refill_interval: Duration::from_secs(60),
            },
            clock,
        )
        .unwrap();

        let request = LLMRequest::new("hello").with_backend("backend-a");

        assert!(
            router
                .select_for_dispatch_with_rate_limiter(&request, &limiter)
                .await
                .is_ok()
        );

        let err = router
            .select_for_dispatch_with_rate_limiter(&request, &limiter)
            .await
            .unwrap_err();

        assert_eq!(
            err.downcast::<RateLimitError>().unwrap(),
            RateLimitError::Exceeded {
                key: "backend:backend-a".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn test_separate_backend_ids_use_separate_buckets() {
        let router = make_router_with_backends(&["backend-a", "backend-b"]);
        let clock = Arc::new(TestClock::new(Instant::now()));
        let limiter = RateLimiter::with_clock(
            RateLimitConfig {
                capacity: 1,
                refill_tokens: 1,
                refill_interval: Duration::from_secs(60),
            },
            clock,
        )
        .unwrap();

        let request_a = LLMRequest::new("hello").with_backend("backend-a");
        let request_b = LLMRequest::new("hello").with_backend("backend-b");

        assert!(
            router
                .select_for_dispatch_with_rate_limiter(&request_a, &limiter)
                .await
                .is_ok()
        );
        assert!(
            router
                .select_for_dispatch_with_rate_limiter(&request_b, &limiter)
                .await
                .is_ok()
        );

        let err_a = router
            .select_for_dispatch_with_rate_limiter(&request_a, &limiter)
            .await
            .unwrap_err();
        let err_b = router
            .select_for_dispatch_with_rate_limiter(&request_b, &limiter)
            .await
            .unwrap_err();

        assert_eq!(
            err_a.downcast::<RateLimitError>().unwrap(),
            RateLimitError::Exceeded {
                key: "backend:backend-a".to_string(),
            }
        );
        assert_eq!(
            err_b.downcast::<RateLimitError>().unwrap(),
            RateLimitError::Exceeded {
                key: "backend:backend-b".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn test_refill_allows_requests_again_after_enough_time_passes() {
        let router = make_router_with_backends(&["backend-a"]);
        let clock = Arc::new(TestClock::new(Instant::now()));
        let limiter = RateLimiter::with_clock(
            RateLimitConfig {
                capacity: 1,
                refill_tokens: 1,
                refill_interval: Duration::from_secs(5),
            },
            clock.clone(),
        )
        .unwrap();

        let request = LLMRequest::new("hello").with_backend("backend-a");

        assert!(
            router
                .select_for_dispatch_with_rate_limiter(&request, &limiter)
                .await
                .is_ok()
        );
        assert!(matches!(
            router
                .select_for_dispatch_with_rate_limiter(&request, &limiter)
                .await
                .unwrap_err()
                .downcast::<RateLimitError>()
                .unwrap(),
            RateLimitError::Exceeded { .. }
        ));

        clock.advance(Duration::from_secs(4));
        assert!(matches!(
            router
                .select_for_dispatch_with_rate_limiter(&request, &limiter)
                .await
                .unwrap_err()
                .downcast::<RateLimitError>()
                .unwrap(),
            RateLimitError::Exceeded { .. }
        ));

        clock.advance(Duration::from_secs(1));
        assert!(
            router
                .select_for_dispatch_with_rate_limiter(&request, &limiter)
                .await
                .is_ok()
        );
    }
}
