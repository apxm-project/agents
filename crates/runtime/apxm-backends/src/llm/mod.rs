//! APxM Models - Unified LLM Provider Integration Library
//!
//! Provides a declarative, minimalistic interface for integrating multiple LLM providers
//! and protocols (OpenAI, Anthropic, Google, Ollama, vLLM, mock) with intelligent
//! routing, cost tracking, retry logic, and schema validation.
//!
//! # Design Principles
//!
//! - **Minimal scope**: Only manages model connections and requests, not storage or sessions
//! - **Declarative**: Configuration-driven behavior
//! - **Modular**: Each provider is independently extensible
//! - **DRY**: No duplication across providers
//! - **Type-safe**: Leverages Rust's type system

pub mod assembler;
pub mod backends;
pub mod catalog;
pub mod config;
#[cfg(feature = "metrics")]
pub mod observability;
pub mod protocol;
pub mod rate_limit;
pub mod wire;
#[cfg(not(feature = "metrics"))]
pub mod observability {
    use apxm_core::types::TokenUsage;
    use serde::{Deserialize, Serialize};
    use std::time::Duration;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RequestMetrics {
        pub backend: String,
        pub model: String,
        pub latency: Duration,
        pub usage: TokenUsage,
        pub success: bool,
        pub retry_count: usize,
        pub timestamp: std::time::SystemTime,
    }

    impl Default for RequestMetrics {
        fn default() -> Self {
            Self {
                backend: String::new(),
                model: String::new(),
                latency: Duration::default(),
                usage: TokenUsage::new(0, 0),
                success: false,
                retry_count: 0,
                timestamp: std::time::SystemTime::UNIX_EPOCH,
            }
        }
    }

    impl RequestMetrics {
        pub fn new(
            backend: String,
            model: String,
            latency: Duration,
            usage: TokenUsage,
            success: bool,
            retry_count: usize,
        ) -> Self {
            Self {
                backend,
                model,
                latency,
                usage,
                success,
                retry_count,
                timestamp: std::time::SystemTime::now(),
            }
        }
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct AggregatedMetrics {
        pub total_requests: usize,
        pub successful_requests: usize,
        pub failed_requests: usize,
        pub total_input_tokens: usize,
        pub total_output_tokens: usize,
        pub average_latency: Duration,
        pub p50_latency: Duration,
        pub p99_latency: Duration,
        pub total_retries: usize,
    }

    #[derive(Clone, Default)]
    pub struct MetricsTracker;

    impl MetricsTracker {
        pub fn new() -> Self {
            Self
        }

        pub fn record(&self, _metrics: RequestMetrics) {}

        pub fn aggregate(&self) -> AggregatedMetrics {
            AggregatedMetrics::default()
        }

        pub fn aggregate_backend(&self, _backend: &str) -> Option<AggregatedMetrics> {
            None
        }

        pub fn aggregate_per_backend(
            &self,
        ) -> std::collections::HashMap<String, AggregatedMetrics> {
            std::collections::HashMap::new()
        }

        pub fn aggregate_model(&self, _model: &str) -> Option<AggregatedMetrics> {
            None
        }

        pub fn success_rate(&self) -> f64 {
            0.0
        }

        pub fn reset(&self) {}
    }

    pub struct RequestTracer;

    impl RequestTracer {
        pub fn start(_backend: String, _model: String) -> Self {
            Self
        }

        pub fn record_retry(&mut self) {}

        pub fn finish(self, usage: TokenUsage, success: bool) -> RequestMetrics {
            RequestMetrics::new(
                String::new(),
                String::new(),
                Duration::default(),
                usage,
                success,
                0,
            )
        }
    }

    pub struct BackendMetricsSource {
        pub aggregate: AggregatedMetrics,
        pub per_backend: std::collections::HashMap<String, AggregatedMetrics>,
        pub graph_status_snapshots: Vec<apxm_core::types::GraphStatusSnapshot>,
        pub graph_capabilities:
            std::collections::HashMap<String, apxm_core::types::BackendGraphCapabilities>,
    }

    impl apxm_core::metrics::MetricsSource for BackendMetricsSource {
        fn section_name(&self) -> &'static str {
            apxm_core::constants::session::metrics_keys::SECTION_BACKENDS
        }

        fn collect(&self) -> serde_json::Value {
            use apxm_core::constants::session::metrics_keys;
            use apxm_core::types::GraphStatusSnapshot;

            if self.aggregate.total_requests == 0
                && self.per_backend.is_empty()
                && self.graph_status_snapshots.is_empty()
                && self.graph_capabilities.is_empty()
            {
                return serde_json::Value::Null;
            }

            let mut map = serde_json::Map::new();
            map.insert(
                metrics_keys::BACKENDS_AGGREGATE.to_owned(),
                serde_json::to_value(&self.aggregate).unwrap_or_default(),
            );
            map.insert(
                metrics_keys::BACKENDS_PER_BACKEND.to_owned(),
                serde_json::to_value(&self.per_backend).unwrap_or_default(),
            );
            let graph_statuses: Vec<_> = self
                .graph_status_snapshots
                .iter()
                .map(GraphStatusSnapshot::to_metrics_json)
                .collect();
            if !graph_statuses.is_empty() {
                map.insert(
                    metrics_keys::BACKENDS_GRAPHS.to_owned(),
                    serde_json::Value::Array(graph_statuses),
                );
            }
            if !self.graph_capabilities.is_empty() {
                map.insert(
                    metrics_keys::BACKENDS_GRAPH_CAPABILITIES.to_owned(),
                    serde_json::to_value(&self.graph_capabilities).unwrap_or_default(),
                );
            }
            serde_json::Value::Object(map)
        }
    }
}
pub mod provider;
pub mod registration;
pub mod registry;
pub mod retry;
pub mod schema;

pub use catalog::{
    BUILTIN_MODELS, BUILTIN_PROVIDERS, BuiltinModelSpec, BuiltinProviderSpec,
    DEFAULT_VLLM_BASE_URL, default_model_for_protocol, default_model_for_provider,
    models_for_protocol, models_for_provider, resolve_builtin_model, resolve_builtin_provider,
    resolve_provider_spec,
};
pub use config::{BackendConfig, BackendType, DockerConfig, ModelConfig};
pub use protocol::{
    ProviderProtocol, ProviderSpec, normalize_anthropic_gateway_endpoint,
    normalize_endpoint_for_protocol,
};

pub use assembler::{AssembledEvent, AssembledToolCall, StreamAssembler};
pub use backends::{
    BackendFactory, ContentPart, FunctionCall, GenerationConfig, LLMBackend, LLMRequest,
    LLMResponse, Message, Role, StreamChunk, TokenUsage, ToolChoice, ToolDefinition,
};
pub use observability::{
    AggregatedMetrics, BackendMetricsSource, MetricsTracker, RequestMetrics, RequestTracer,
};
pub use provider::{Provider, ProviderId, RegisteredProvider};
pub use rate_limit::{RateLimitConfig, RateLimitConfigError, RateLimitError};
pub use registration::{
    BackendFallback, BackendRegistration, ModelAliasRegistration, ModelRegistration,
    OperationRoute, RegistryPolicy,
};
pub use registry::{
    HealthMonitor, HealthStatus, LLMRegistry, StreamingBackendError, StreamingFailureKind,
};
pub use retry::{ErrorClass, RetryConfig, RetryStrategy};
pub use schema::{JsonSchema, OutputParser};

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
