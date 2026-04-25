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
#[cfg(feature = "metrics")]
pub mod observability;
pub mod rate_limit;
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

        pub fn aggregate_per_backend(&self) -> std::collections::HashMap<String, AggregatedMetrics> {
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
        pub vllm_graphs: Vec<serde_json::Value>,
    }

    impl apxm_core::metrics::MetricsSource for BackendMetricsSource {
        fn section_name(&self) -> &'static str {
            apxm_core::constants::session::metrics_keys::SECTION_BACKENDS
        }

        fn collect(&self) -> serde_json::Value {
            serde_json::Value::Null
        }
    }
}
pub mod provider;
pub mod registration;
pub mod registry;
pub mod retry;
pub mod schema;

pub use apxm_core::types::{ProviderProtocol, ProviderSpec};

// Re-export key public API types
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
pub use registry::{HealthMonitor, HealthStatus, LLMRegistry};
pub use retry::{ErrorClass, RetryConfig, RetryStrategy};
pub use schema::{JsonSchema, OutputParser};

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
