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

pub use anthropic::{AnthropicBackend, AnthropicModel};
pub use google::{GoogleBackend, GoogleModel};
pub use mock::{MockLLMBackend, MockResponse, RecordedCall};
pub use ollama::{OllamaBackend, OllamaModel};
pub use openai::{OpenAIBackend, OpenAIModel};
pub use request::{
    ContentPart, FunctionCall, GenerationConfig, LLMRequest, Message, RequestBuilder, Role,
    ToolChoice, ToolDefinition,
};
pub use response::{LLMResponse, TokenUsage};
pub use traits::{LLMBackend, StreamChunk};

use apxm_core::types::ProviderProtocol;
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
        let protocol = match provider.to_lowercase().as_str() {
            "openai" => ProviderProtocol::OpenAI,
            "anthropic" => ProviderProtocol::Anthropic,
            "google" => ProviderProtocol::Google,
            "ollama" => ProviderProtocol::Ollama,
            _ => {
                return Err(anyhow::anyhow!(
                    "Unknown provider: {}. Supported: openai, anthropic, google, ollama",
                    provider
                ))
            }
        };
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
            ProviderProtocol::OpenAI => Arc::new(openai::OpenAIBackend::new(api_key, config).await?),
            ProviderProtocol::Anthropic => Arc::new(anthropic::AnthropicBackend::new(api_key, config).await?),
            ProviderProtocol::Google => Arc::new(google::GoogleBackend::new(api_key, config).await?),
            ProviderProtocol::Ollama => Arc::new(ollama::OllamaBackend::new(api_key, config).await?),
        };
        Ok(backend)
    }

    /// List available backend providers.
    pub fn list_providers() -> Vec<&'static str> {
        vec!["openai", "anthropic", "google", "ollama"]
    }
}
