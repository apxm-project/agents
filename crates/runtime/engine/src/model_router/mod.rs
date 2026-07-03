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
use apxm_backends::{ContentPart, LLMRegistry, LLMRequest, LLMResponse};
use apxm_core::types::AISOperationType;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Default assumed output-token budget when a request does not set
/// `max_tokens`, used for context-fit and cost estimation.
const DEFAULT_OUTPUT_TOKENS: usize = 1024;

/// Optimization target that influences model selection.
///
/// Serialized in `snake_case` ("cost", "latency", "quality", "balanced") so it
/// reads naturally in `models.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
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

/// Hard constraints derived from a request, used to prune the candidate pool
/// before ranking by [`RoutingTarget`].
///
/// Every field is read from a concrete `LLMRequest` field (no inference):
/// `needs_tools` ← `request.tools`, `needs_json` ← `request.output_schema`,
/// `needs_thinking` ← `enable_thinking`/`thinking_token_budget`,
/// `needs_vision` ← an image content part in `messages`, `needs_local` ← a
/// truthy `metadata["require_local"]`.
#[derive(Debug, Clone, Default)]
struct RoutingRequirements {
    /// Estimated input tokens (prompt + system + message text).
    est_input: usize,
    /// Estimated output tokens (`max_tokens` or [`DEFAULT_OUTPUT_TOKENS`]).
    est_output: usize,
    needs_tools: bool,
    needs_json: bool,
    needs_thinking: bool,
    needs_vision: bool,
    needs_local: bool,
}

impl RoutingRequirements {
    fn from_request(request: &LLMRequest) -> Self {
        RoutingRequirements {
            est_input: estimate_input_tokens(request),
            est_output: request.max_tokens.unwrap_or(DEFAULT_OUTPUT_TOKENS),
            needs_tools: request.tools.as_ref().is_some_and(|t| !t.is_empty()),
            needs_json: request.output_schema.is_some(),
            needs_thinking: request.enable_thinking == Some(true)
                || request.thinking_token_budget.is_some(),
            needs_vision: request_needs_vision(request),
            needs_local: request
                .metadata
                .get("require_local")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }
    }

    /// Returns true if `entry` satisfies every hard constraint.
    fn satisfied_by(&self, entry: &ModelEntry) -> bool {
        entry.fits_context(self.est_input, self.est_output)
            && (!self.needs_tools || entry.supports_tools)
            && (!self.needs_json || entry.supports_json)
            && (!self.needs_thinking || entry.supports_thinking)
            && (!self.needs_vision || entry.supports_vision)
            && (!self.needs_local || entry.local)
    }
}

/// Rough token estimate from request text (≈4 chars/token). Deliberately
/// cheap and provider-agnostic; only relative magnitude matters for routing.
fn estimate_input_tokens(request: &LLMRequest) -> usize {
    let mut chars = request.prompt.len();
    if let Some(system) = &request.system_prompt {
        chars += system.len();
    }
    for message in &request.messages {
        for part in &message.content {
            if let ContentPart::Text { text } = part {
                chars += text.len();
            }
        }
    }
    (chars / 4).max(1)
}

/// True if any message carries an image content part (vision required).
fn request_needs_vision(request: &LLMRequest) -> bool {
    request.messages.iter().any(|m| {
        m.content
            .iter()
            .any(|p| matches!(p, ContentPart::Image { .. }))
    })
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
        let known_backends: std::collections::HashSet<String> =
            llm_registry.backend_names().into_iter().collect();
        for name in &known_backends {
            circuit_breakers.register(name);
        }

        // Reconciliation: warn about price-table rows that reference a backend
        // no `LLMRegistry` knows about. Such a model can never be routed (its
        // breaker is never registered, so `is_available` will never pass via
        // the table), and today this divergence is otherwise silent until
        // dispatch. Skip the warning when no backends are registered (the
        // common case in unit tests that wire breakers up by hand).
        if !known_backends.is_empty() {
            for entry in model_registry.list() {
                if !known_backends.contains(&entry.backend) {
                    tracing::warn!(
                        model = %entry.name,
                        backend = %entry.backend,
                        "models.toml references an unregistered backend; this model can never be routed"
                    );
                }
            }
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

        // 4. Policy-driven selection from the price/capability table.
        //
        // Effective target precedence: per-operation policy → `[routing]
        // target` in models.toml → global `ModelRouterConfig.target`.
        let routing = self.model_registry.routing();
        let target = op_policy
            .as_ref()
            .map(|p| p.target)
            .or(routing.target)
            .unwrap_or(self.config.target);

        if target == RoutingTarget::Balanced {
            // Balanced preserves the tag-preference behaviour.
            for tag in &routing.prefer_tags {
                if let Some(decision) = self.find_by_tag(tag) {
                    return Ok(decision);
                }
            }
        } else if let Some(decision) = self.select_from_table(request, target) {
            // Cost / Quality / Latency rank the feasible pool. A `None` here
            // means no candidate satisfied the hard constraints, so we fall
            // through to default / first-available routing below.
            return Ok(decision);
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
    ///
    /// Public so callers that dispatch outside of [`ModelRouter::generate`]
    /// (e.g. streaming handlers that must apply the resolved backend/model to
    /// a request before handing it to a streaming registry call) can still
    /// route through policy + circuit breakers + rate limits rather than
    /// picking a backend directly. `explicit request.backend`/`request.model`
    /// still win inside `select()` — precedence is unchanged.
    pub async fn select_for_dispatch(&self, request: &LLMRequest) -> anyhow::Result<RoutingDecision> {
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
    /// ProfileRouter selects healthy candidates from this status snapshot.
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

    /// EWMA-smoothed whole-request latency (ms) for a backend, as tracked by
    /// the underlying `LLMRegistry`'s `HealthMonitor`. `None` means no
    /// successful request has completed for this backend yet.
    fn backend_latency_ms(&self, backend: &str) -> Option<f64> {
        self.llm_registry.backend_latency_ms_ewma(backend)
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

    /// Price/capability-aware selection: prune the pool to the candidates that
    /// satisfy the request's hard constraints (context fit, required
    /// capabilities, breaker availability), then rank the survivors by
    /// `target`. Returns `None` when no candidate is feasible, letting the
    /// caller fall through to default / first-available routing.
    ///
    /// Cost, Quality, and breaker availability are deterministic rules over a
    /// static table — config columns or current breaker state. `Latency` is
    /// the one exception: it ranks by the live EWMA measured latency from
    /// `HealthMonitor`, so its result can change between calls as backends
    /// serve requests. All targets sort candidates by name first, so ties
    /// (including "no measurement yet" ties under `Latency`) resolve
    /// identically across runs.
    fn select_from_table(
        &self,
        request: &LLMRequest,
        target: RoutingTarget,
    ) -> Option<RoutingDecision> {
        let reqs = RoutingRequirements::from_request(request);

        let mut feasible: Vec<ModelEntry> = self
            .model_registry
            .list()
            .into_iter()
            .filter(|m| self.circuit_breakers.is_available(&m.backend))
            .filter(|m| reqs.satisfied_by(m))
            .collect();

        if feasible.is_empty() {
            return None;
        }

        // Deterministic ordering so equal-keyed candidates break ties by name.
        feasible.sort_by(|a, b| a.name.cmp(&b.name));

        let chosen = match target {
            // Cheapest at the request's operating point.
            RoutingTarget::Cost => feasible.iter().min_by(|a, b| {
                let ca = a.estimate_cost(reqs.est_input, reqs.est_output);
                let cb = b.estimate_cost(reqs.est_input, reqs.est_output);
                ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
            }),
            // Strongest operator-supplied quality prior (first on ties).
            RoutingTarget::Quality => feasible
                .iter()
                .min_by_key(|m| std::cmp::Reverse(m.quality_tier)),
            // Rank by measured EWMA latency (`HealthMonitor::record_success`
            // feeds `BackendLatencyProfile` on every successful call). A
            // backend with no samples yet (cold start, or one that has never
            // completed a request) sorts last rather than crashing or being
            // silently preferred — once it starts serving requests its
            // measured latency takes over on the next selection.
            RoutingTarget::Latency => feasible.iter().min_by(|a, b| {
                let la = self.backend_latency_ms(&a.backend);
                let lb = self.backend_latency_ms(&b.backend);
                match (la, lb) {
                    (Some(la), Some(lb)) => la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            }),
            // Balanced is handled by the caller (tag preference); never reaches
            // here, but map it to the first candidate defensively.
            RoutingTarget::Balanced => feasible.first(),
        }?;

        Some(RoutingDecision {
            backend: chosen.backend.clone(),
            model: Some(chosen.name.clone()),
            was_failover: false,
        })
    }
}

#[cfg(test)]
mod latency_routing_tests {
    use super::*;
    use apxm_backends::llm::backends::MockLLMBackend;
    use std::time::Duration;

    /// Build a router with `backends` registered against a real `LLMRegistry`
    /// (so `HealthMonitor` tracking is live) and a matching `ModelEntry` per
    /// backend in the model table, all otherwise-identical so `Latency` is
    /// the only thing that can break ties.
    fn router_with_backends(backends: &[&str]) -> ModelRouter {
        let llm_registry = Arc::new(LLMRegistry::new());
        let model_registry = Arc::new(ModelRegistry::new());
        for name in backends {
            llm_registry
                .register(*name, MockLLMBackend::static_response("ok"))
                .expect("register mock backend");
            model_registry.register(ModelEntry {
                name: format!("{name}-model"),
                backend: name.to_string(),
                ..Default::default()
            });
        }
        ModelRouter::with_model_registry(
            llm_registry,
            model_registry,
            ModelRouterConfig::default(),
        )
        .expect("router construction")
    }

    /// Record a successful call against `backend` with the given latency by
    /// going through the same path production code uses
    /// (`LLMRegistry::record_streaming_outcome`), which calls
    /// `HealthMonitor::record_success` under the hood.
    fn record_latency(router: &ModelRouter, backend: &str, latency_ms: u64) {
        router.llm_registry().record_streaming_outcome(
            backend,
            "irrelevant-model",
            Duration::from_millis(latency_ms),
            None,
            true,
        );
    }

    #[test]
    fn record_success_feeds_the_ewma_and_is_queryable_from_the_registry() {
        let router = router_with_backends(&["a"]);
        assert_eq!(router.backend_latency_ms("a"), None, "cold start: no signal yet");

        record_latency(&router, "a", 50);
        assert_eq!(router.backend_latency_ms("a"), Some(50.0));

        // EWMA (alpha ~0.2 default) should move toward, but not jump fully
        // to, a very different new sample — proving it's a smoothed signal
        // and not last-value-wins.
        record_latency(&router, "a", 500);
        let after = router.backend_latency_ms("a").unwrap();
        assert!(after > 50.0 && after < 500.0, "expected smoothed value, got {after}");
    }

    #[test]
    fn routing_target_latency_prefers_the_measured_fastest_backend() {
        let router = router_with_backends(&["slow", "fast", "medium"]);
        record_latency(&router, "slow", 500);
        record_latency(&router, "fast", 10);
        record_latency(&router, "medium", 100);

        let request = LLMRequest::new("hello");
        let decision = router
            .select_from_table(&request, RoutingTarget::Latency)
            .expect("a feasible candidate should be found");

        assert_eq!(decision.backend, "fast");
    }

    #[test]
    fn routing_target_latency_falls_back_sensibly_with_no_measurements() {
        // No calls recorded for any backend: every candidate is cold-start
        // (`None` latency). Selection must not panic and must still return a
        // deterministic candidate (ties break by model name, same as the
        // other targets).
        let router = router_with_backends(&["b", "a"]);

        let request = LLMRequest::new("hello");
        let decision = router
            .select_from_table(&request, RoutingTarget::Latency)
            .expect("cold-start candidates are still feasible");

        // "a-model" sorts before "b-model" alphabetically.
        assert_eq!(decision.backend, "a");
    }

    #[test]
    fn routing_target_latency_prefers_measured_backend_over_cold_start() {
        // One backend has a (bad-ish) measurement, the other has none yet.
        // A backend with a real signal should win over an unmeasured one,
        // even though the unmeasured one might turn out faster — we rank
        // what we can observe.
        let router = router_with_backends(&["measured", "cold"]);
        record_latency(&router, "measured", 1000);

        let request = LLMRequest::new("hello");
        let decision = router
            .select_from_table(&request, RoutingTarget::Latency)
            .expect("a feasible candidate should be found");

        assert_eq!(decision.backend, "measured");
    }
}
