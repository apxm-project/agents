//! ModelRouter — dynamic model selection at runtime.
//!
//! The ModelRouter sits between the executor's LLM dispatch path and the
//! `LLMRegistry`. It adds:
//!
//! - **Circuit breakers** per backend (Closed → Open → HalfOpen)
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
pub mod registry;

pub use health::{BackendHealth, CircuitBreakerConfig, CircuitBreakerRegistry, CircuitState};
pub use registry::{ModelEntry, ModelRegistry, RoutingConfig};

use apxm_backends::{LLMRegistry, LLMRequest, LLMResponse};
use apxm_core::types::AISOperationType;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    /// Router configuration.
    config: ModelRouterConfig,
}

impl ModelRouter {
    /// Create a new router backed by an existing `LLMRegistry`.
    pub fn new(llm_registry: Arc<LLMRegistry>, config: ModelRouterConfig) -> Self {
        let circuit_breakers =
            Arc::new(CircuitBreakerRegistry::new(config.circuit_breaker.clone()));

        // Pre-register circuit breakers for all currently known backends.
        for name in llm_registry.backend_names() {
            circuit_breakers.register(&name);
        }

        let model_registry = Arc::new(ModelRegistry::load_from_default_path());

        ModelRouter {
            llm_registry,
            model_registry,
            circuit_breakers,
            config,
        }
    }

    /// Create with a pre-built `ModelRegistry` (useful for testing).
    pub fn with_model_registry(
        llm_registry: Arc<LLMRegistry>,
        model_registry: Arc<ModelRegistry>,
        config: ModelRouterConfig,
    ) -> Self {
        let circuit_breakers =
            Arc::new(CircuitBreakerRegistry::new(config.circuit_breaker.clone()));
        for name in llm_registry.backend_names() {
            circuit_breakers.register(&name);
        }
        ModelRouter {
            llm_registry,
            model_registry,
            circuit_breakers,
            config,
        }
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

    /// Access the underlying model registry.
    pub fn model_registry(&self) -> &ModelRegistry {
        &self.model_registry
    }

    /// Access the underlying LLM registry.
    pub fn llm_registry(&self) -> &LLMRegistry {
        &self.llm_registry
    }

    /// Execute an LLM request using the router's selection logic.
    ///
    /// Wraps `LLMRegistry::generate_with_backend` and records circuit-breaker
    /// outcomes automatically.
    pub async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse> {
        let decision = self.select(&request)?;
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

    fn make_router() -> ModelRouter {
        let llm_registry = Arc::new(LLMRegistry::new());
        let model_registry = Arc::new(ModelRegistry::new());
        ModelRouter::with_model_registry(
            llm_registry,
            model_registry,
            ModelRouterConfig::default(),
        )
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
        );

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
            ModelRouter::with_model_registry(llm_registry, model_registry, config.clone());

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
}
