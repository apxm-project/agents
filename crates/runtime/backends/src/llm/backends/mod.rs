//! LLM backend implementations and traits.
//!
//! This module provides:
//! - `LLMBackend` trait: Unified interface for all providers
//! - Request/Response types: Normalized API across providers
//! - Provider implementations: OpenAI, Anthropic, Google, Ollama
//! - Factory: Create backends from configuration

#![allow(clippy::unused_async)]

pub mod request;
pub mod response;
pub mod traits;

pub mod anthropic;
pub mod configuration;
pub mod google;
pub(crate) mod http;
pub mod llama_cpp;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod vllm;

pub use anthropic::AnthropicBackend;
pub use google::GoogleBackend;
pub use llama_cpp::LlamaCppBackend;
pub use mock::{MockLLMBackend, MockResponse, RecordedCall};
pub use ollama::OllamaBackend;
pub use openai::OpenAIBackend;
pub use request::{
    ContentPart, FunctionCall, GenerationConfig, LLMRequest, Message, Role, ToolChoice,
    ToolDefinition,
};
pub use response::{LLMResponse, TokenUsage};
pub use traits::{
    CorrelatedBatchingCapability, CorrelatedLLMOutcome, CorrelatedLLMRequest, LLMBackend,
    StreamChunk,
};
pub use vllm::{ApxmGraphHints, GraphAwareVllmBackend, GraphMetadata, NodeSpec};

use crate::llm::ProviderProtocol;
pub use configuration::BackendConfigurationError;
pub(crate) use configuration::{
    ConfiguredModelCapabilities, configured_model_capabilities, configured_model_info,
    required_config_string, resolve_configured_value,
};
use std::sync::Arc;

/// Factory for creating LLM backends from provider configuration.
pub struct BackendFactory;

impl BackendFactory {
    /// Create a backend from provider name and a materialized API key value.
    ///
    /// Durable credential custody belongs to auth or the process environment;
    /// this factory receives only the short-lived value needed by the adapter.
    pub async fn create(
        provider: &str,
        api_key: &str,
        config: Option<serde_json::Value>,
    ) -> anyhow::Result<Arc<dyn LLMBackend>> {
        let protocol = provider.parse::<ProviderProtocol>().map_err(|_| {
            BackendConfigurationError::UnknownProvider {
                provider: provider.to_string(),
            }
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
            ProviderProtocol::LlamaCpp => {
                Arc::new(llama_cpp::LlamaCppBackend::new(api_key, config).await?)
            }
            ProviderProtocol::Mock => {
                Arc::new(mock::MockLLMBackend::from_config(api_key, config).await?)
            }
        };
        Ok(backend)
    }

    /// List available backend providers.
    pub fn list_providers() -> Vec<&'static str> {
        ProviderProtocol::all_variants()
            .iter()
            .map(ProviderProtocol::as_str)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn factory_rejects_unknown_provider_without_catalog_fallback() {
        let error = match BackendFactory::create("openrouter", "", None).await {
            Err(error) => error,
            Ok(_) => panic!("unregistered provider must fail closed"),
        };

        assert!(matches!(
            error.downcast_ref::<BackendConfigurationError>(),
            Some(BackendConfigurationError::UnknownProvider { provider }) if provider == "openrouter"
        ));
    }

    #[tokio::test]
    async fn factory_requires_explicit_model_and_endpoint_configuration() {
        for protocol in [
            ProviderProtocol::OpenAI,
            ProviderProtocol::Anthropic,
            ProviderProtocol::Google,
            ProviderProtocol::Ollama,
            ProviderProtocol::Vllm,
        ] {
            let error = match BackendFactory::create_from_protocol(protocol, "", None).await {
                Err(error) => error,
                Ok(_) => panic!("missing adapter configuration must fail closed"),
            };

            assert!(matches!(
                error.downcast_ref::<BackendConfigurationError>(),
                Some(BackendConfigurationError::MissingConfiguration { protocol: actual })
                if *actual == protocol
            ));
        }

        let error = match BackendFactory::create_from_protocol(
            ProviderProtocol::OpenAI,
            "",
            Some(serde_json::json!({ "model": "fixture-model" })),
        )
        .await
        {
            Err(error) => error,
            Ok(_) => panic!("missing endpoint must fail closed"),
        };
        assert!(matches!(
            error.downcast_ref::<BackendConfigurationError>(),
            Some(BackendConfigurationError::MissingRequiredField {
                protocol: ProviderProtocol::OpenAI,
                field: "base_url",
            })
        ));
    }
}
