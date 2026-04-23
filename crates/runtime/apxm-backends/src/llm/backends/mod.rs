//! LLM backend implementations and traits.
//!
//! This module provides:
//! - `LLMBackend` trait: Unified interface for all providers
//! - Request/Response types: Normalized API across providers
//! - Provider implementations: OpenAI, Anthropic, Google, Ollama
//! - Factory: Create backends from configuration

pub mod request;
pub mod response;
pub mod traits;

pub mod anthropic;
pub mod google;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod vllm;

pub use anthropic::AnthropicBackend;
pub use google::GoogleBackend;
pub use mock::{MockLLMBackend, MockResponse, RecordedCall};
pub use ollama::OllamaBackend;
pub use openai::OpenAIBackend;
pub use request::{
    ContentPart, FunctionCall, GenerationConfig, LLMRequest, Message, Role, ToolChoice,
    ToolDefinition,
};
pub use response::{LLMResponse, TokenUsage};
pub use traits::{LLMBackend, StreamChunk};
pub use vllm::{
    ApxmGraphHints, CompilerHints, GraphAwareVllmBackend, GraphMetadata, NodeSpec, PinMode,
    PinPolicy, PriorityClass,
};

use apxm_core::types::{ProviderProtocol, resolve_builtin_provider};
use std::sync::Arc;

/// Factory for creating LLM backends from provider configuration.
pub struct BackendFactory;

impl BackendFactory {
    /// Create a backend from provider name (string) and API key.
    pub async fn create(
        provider: &str,
        api_key: &str,
        config: Option<serde_json::Value>,
    ) -> anyhow::Result<Arc<dyn LLMBackend>> {
        let protocol = resolve_builtin_provider(provider)
            .map(|spec| spec.protocol)
            .or_else(|| provider.parse::<ProviderProtocol>().ok())
            .ok_or_else(|| {
                let supported = ProviderProtocol::all_variants()
                    .iter()
                    .map(ProviderProtocol::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow::anyhow!("Unknown provider: {}. Supported: {}", provider, supported)
            })?;
        Self::create_from_protocol(protocol, api_key, config).await
    }

    /// Create a backend from a `ProviderProtocol`.
    ///
    /// This is the data-driven alternative — routes by protocol enum
    /// instead of string matching. Useful for custom providers that use
    /// an existing protocol (e.g., OpenRouter via `ProviderProtocol::OpenAI`).
    pub async fn create_from_protocol(
        protocol: ProviderProtocol,
        api_key: &str,
        config: Option<serde_json::Value>,
    ) -> anyhow::Result<Arc<dyn LLMBackend>> {
        let backend: Arc<dyn LLMBackend> = match protocol {
            ProviderProtocol::OpenAI => {
                Arc::new(openai::OpenAIBackend::new(api_key, config).await?)
            }
            ProviderProtocol::Anthropic => {
                Arc::new(anthropic::AnthropicBackend::new(api_key, config).await?)
            }
            ProviderProtocol::Google => {
                Arc::new(google::GoogleBackend::new(api_key, config).await?)
            }
            ProviderProtocol::Ollama => {
                Arc::new(ollama::OllamaBackend::new(api_key, config).await?)
            }
            ProviderProtocol::Vllm => {
                Arc::new(vllm::GraphAwareVllmBackend::new(api_key, config).await?)
            }
            ProviderProtocol::Mock => {
                Arc::new(mock::MockLLMBackend::from_config(api_key, config).await?)
            }
        };
        Ok(backend)
    }

    /// List available backend providers.
    pub fn list_providers() -> Vec<&'static str> {
        vec!["openai", "anthropic", "google", "ollama", "vllm", "mock"]
    }
}
