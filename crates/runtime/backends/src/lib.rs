//! APXM Backends - LLM providers and prompt templates.
//!
//! This crate consolidates two backend systems:
//!
//! - **`llm`**: Unified LLM provider integration (OpenAI, Anthropic, Google, Ollama, vLLM, mock)
//! - **`prompts`**: Compile-time embedded prompt templates with MiniJinja
//!
//! # Architecture
//!
//! ```text
//!                  ┌─────────────────┐
//!                  │  apxm-backends  │
//!                  └────────┬────────┘
//!                           │
//!            ┌──────────────┴──────────────┐
//!            ▼                             ▼
//!       ┌─────────┐                   ┌──────────┐
//!       │   llm   │                   │ prompts  │
//!       │ OpenAI  │                   │ MiniJinja│
//!       │Anthropic│                   │ Templates│
//!       │ Google  │                   │          │
//!       │ Ollama  │                   │          │
//!       └─────────┘                   └──────────┘
//! ```

// Module declarations
pub mod llm;
pub mod prompts;

// ═══════════════════════════════════════════════════════════════════════════
// LLM Re-exports
// ═══════════════════════════════════════════════════════════════════════════

pub use llm::{
    // Observability
    AggregatedMetrics,
    // Provider management
    BUILTIN_MODELS,
    BUILTIN_PROVIDERS,
    // Factory
    BackendConfig,
    BackendConfigurationError,
    BackendFactory,
    BackendMetricsSource,
    BackendRegistration,
    BackendType,
    BuiltinModelSpec,
    BuiltinProviderSpec,
    // Structured message types
    ContentPart,
    CorrelatedBatchRoute,
    CorrelatedBatchingCapability,
    CorrelatedLLMOutcome,
    CorrelatedLLMRequest,
    // Retry logic
    ErrorClass,
    FunctionCall,
    GenerationConfig,
    // Registry and health
    HealthMonitor,
    HealthStatus,
    // Schema validation
    JsonSchema,
    LLMBackend,
    LLMRegistry,
    LLMRequest,
    LLMResponse,
    Message,
    MetricsTracker,
    ModelConfig,
    ModelReferenceError,
    ModelRegistration,
    OutputParser,
    Provider,
    ProviderId,
    ProviderProtocol,
    ProviderSpec,
    // Rate limiting
    RateLimitConfig,
    RateLimitConfigError,
    RateLimitError,
    RegisteredProvider,
    RequestMetrics,
    RequestTracer,
    RetryConfig,
    RetryStrategy,
    Role,
    StreamChunk,
    // Streaming
    StreamingBackendError,
    StreamingFailureKind,
    TokenUsage,
    // Tool types
    ToolChoice,
    ToolDefinition,
    models_for_protocol,
    models_for_provider,
    normalize_endpoint_for_protocol,
    resolve_builtin_model,
    resolve_builtin_provider,
    resolve_provider_spec,
};

pub use prompts::{list_prompts, render_inline, render_prompt};

pub use apxm_core::{error::RuntimeError, types::values::Value};
