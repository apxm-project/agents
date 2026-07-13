//! APXM Backends - LLM providers, storage backends, and prompt templates.
//!
//! This crate consolidates three backend systems:
//!
//! - **`llm`**: Unified LLM provider integration (OpenAI, Anthropic, Google, Ollama, vLLM, mock)
//! - **`storage`**: Pluggable storage backends (in-memory, SQLite, embedded KV)
//! - **`prompts`**: Compile-time embedded prompt templates with MiniJinja
//!
//! # Architecture
//!
//! ```text
//!                      ┌─────────────────┐
//!                      │  apxm-backends  │
//!                      └────────┬────────┘
//!                               │
//!        ┌──────────────────────┼──────────────────────┐
//!        ▼                      ▼                      ▼
//!   ┌─────────┐           ┌──────────┐           ┌──────────┐
//!   │   llm   │           │  storage │           │ prompts  │
//!   │ OpenAI  │           │  SQLite  │           │ MiniJinja│
//!   │Anthropic│           │  Memory  │           │ Templates│
//!   │ Google  │           │  Redb    │           │          │
//!   │ Ollama  │           │          │           │          │
//!   └─────────┘           └──────────┘           └──────────┘
//! ```

// Module declarations
pub mod llm;
pub mod prompts;
pub mod storage;

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
    BackendFallback,
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
    ModelAliasRegistration,
    ModelConfig,
    ModelRegistration,
    OperationRoute,
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
    RegistryPolicy,
    RequestMetrics,
    RequestSelectionError,
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
    default_model_for_protocol,
    default_model_for_provider,
    models_for_protocol,
    models_for_provider,
    normalize_endpoint_for_protocol,
    resolve_builtin_model,
    resolve_builtin_provider,
    resolve_provider_spec,
};

// ═══════════════════════════════════════════════════════════════════════════
// Storage Re-exports
// ═══════════════════════════════════════════════════════════════════════════

pub use storage::{
    // Backend trait and types
    BackendStats,
    // Embeddings
    Embedder,
    // Implementations
    InMemoryBackend,
    RedbBackend,
    SearchResult,
    SqliteBackend,
    StorageBackend,
    StorageResult,
    cosine_similarity,
};

#[cfg(feature = "embeddings")]
pub use storage::LocalEmbedder;

pub use prompts::{list_prompts, render_inline, render_prompt};

pub use apxm_core::{error::RuntimeError, types::values::Value};
