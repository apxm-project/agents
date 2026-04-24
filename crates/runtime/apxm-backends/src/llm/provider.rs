//! Provider enum for unified backend access.
//!
//! Provides both an enum-based dispatch system and a data-driven
//! [`RegisteredProvider`] for extensible provider management.

use crate::llm::backends::{
    AnthropicBackend, GoogleBackend, GraphAwareVllmBackend, LLMBackend, LLMRequest, LLMResponse,
    MockLLMBackend, OllamaBackend, OpenAIBackend,
};
use apxm_core::types::{ModelCapabilities, ModelInfo, ProviderProtocol, ProviderSpec};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Built-in provider identifiers supported by the enum-based backend surface.
///
/// This stays aligned with the built-in [`ProviderProtocol`] variants that
/// `Provider` can construct directly. Custom providers that share a protocol
/// should use [`RegisteredProvider`] with a [`ProviderSpec`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    OpenAI,
    Anthropic,
    Google,
    Ollama,
    Vllm,
    Mock,
}

impl ProviderId {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderId::OpenAI => "openai",
            ProviderId::Anthropic => "anthropic",
            ProviderId::Google => "google",
            ProviderId::Ollama => "ollama",
            ProviderId::Vllm => "vllm",
            ProviderId::Mock => "mock",
        }
    }

    pub fn all_variants() -> &'static [ProviderId] {
        &[
            ProviderId::OpenAI,
            ProviderId::Anthropic,
            ProviderId::Google,
            ProviderId::Ollama,
            ProviderId::Vllm,
            ProviderId::Mock,
        ]
    }

    /// Returns the concrete protocol used by this built-in provider.
    pub fn protocol(&self) -> ProviderProtocol {
        match self {
            ProviderId::OpenAI => ProviderProtocol::OpenAI,
            ProviderId::Anthropic => ProviderProtocol::Anthropic,
            ProviderId::Google => ProviderProtocol::Google,
            ProviderId::Ollama => ProviderProtocol::Ollama,
            ProviderId::Vllm => ProviderProtocol::Vllm,
            ProviderId::Mock => ProviderProtocol::Mock,
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ProviderId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(ProviderId::OpenAI),
            "anthropic" => Ok(ProviderId::Anthropic),
            "google" => Ok(ProviderId::Google),
            "ollama" => Ok(ProviderId::Ollama),
            "vllm" | "vllm-graph-aware" => Ok(ProviderId::Vllm),
            "mock" => Ok(ProviderId::Mock),
            _ => Err(anyhow::anyhow!("Unknown provider: {}", s)),
        }
    }
}

/// Enum wrapping all backend implementations.
pub enum Provider {
    OpenAI(OpenAIBackend),
    Anthropic(AnthropicBackend),
    Google(GoogleBackend),
    Ollama(OllamaBackend),
    Vllm(GraphAwareVllmBackend),
    Mock(MockLLMBackend),
}

impl Provider {
    /// Create a provider from ID and configuration.
    pub async fn new(
        provider_id: ProviderId,
        api_key: &str,
        config: Option<serde_json::Value>,
    ) -> anyhow::Result<Self> {
        Self::from_protocol(provider_id.protocol(), api_key, config).await
    }

    /// Create a provider from a `ProviderProtocol`.
    ///
    /// This is the data-driven alternative to `new()` — routes by protocol instead
    /// of enum variant, supporting custom/OpenAI-compatible providers.
    pub async fn from_protocol(
        protocol: ProviderProtocol,
        api_key: &str,
        config: Option<serde_json::Value>,
    ) -> anyhow::Result<Self> {
        match protocol {
            ProviderProtocol::OpenAI => {
                Ok(Provider::OpenAI(OpenAIBackend::new(api_key, config).await?))
            }
            ProviderProtocol::Anthropic => Ok(Provider::Anthropic(
                AnthropicBackend::new(api_key, config).await?,
            )),
            ProviderProtocol::Google => {
                Ok(Provider::Google(GoogleBackend::new(api_key, config).await?))
            }
            ProviderProtocol::Ollama => {
                Ok(Provider::Ollama(OllamaBackend::new(api_key, config).await?))
            }
            ProviderProtocol::Vllm => Ok(Provider::Vllm(
                GraphAwareVllmBackend::new(api_key, config).await?,
            )),
            ProviderProtocol::Mock => Ok(Provider::Mock(
                MockLLMBackend::from_config(api_key, config).await?,
            )),
        }
    }

    /// Get the built-in provider identifier for this enum variant.
    pub fn provider_id(&self) -> ProviderId {
        match self {
            Provider::OpenAI(_) => ProviderId::OpenAI,
            Provider::Anthropic(_) => ProviderId::Anthropic,
            Provider::Google(_) => ProviderId::Google,
            Provider::Ollama(_) => ProviderId::Ollama,
            Provider::Vllm(_) => ProviderId::Vllm,
            Provider::Mock(_) => ProviderId::Mock,
        }
    }

    /// Get the wire protocol spoken by this provider.
    pub fn protocol(&self) -> ProviderProtocol {
        self.provider_id().protocol()
    }

    fn backend_ref(&self) -> &dyn LLMBackend {
        match self {
            Provider::OpenAI(b) => b,
            Provider::Anthropic(b) => b,
            Provider::Google(b) => b,
            Provider::Ollama(b) => b,
            Provider::Vllm(b) => b,
            Provider::Mock(b) => b,
        }
    }
}

#[async_trait]
impl LLMBackend for Provider {
    async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse> {
        self.backend_ref().generate(request).await
    }

    fn name(&self) -> &str {
        self.backend_ref().name()
    }

    fn model(&self) -> &str {
        self.backend_ref().model()
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        self.backend_ref().health_check().await
    }

    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        self.backend_ref().list_models().await
    }

    fn capabilities(&self) -> ModelCapabilities {
        self.backend_ref().capabilities()
    }

    async fn register_graph(&self, metadata: serde_json::Value) -> anyhow::Result<()> {
        self.backend_ref().register_graph(metadata).await
    }

    async fn release_graph(&self, graph_id: &str) -> anyhow::Result<()> {
        self.backend_ref().release_graph(graph_id).await
    }
}

/// A provider instance paired with its metadata.
///
/// This is the data-driven alternative to the `Provider` enum, allowing
/// registration of custom/third-party providers without adding enum variants.
pub struct RegisteredProvider {
    /// Provider metadata (protocol, API key env var, etc.).
    pub spec: ProviderSpec,
    /// The actual backend implementation.
    pub backend: Arc<dyn LLMBackend>,
}

impl RegisteredProvider {
    /// Create a registered provider from a spec and backend.
    pub fn new(spec: ProviderSpec, backend: Arc<dyn LLMBackend>) -> Self {
        Self { spec, backend }
    }

    /// Create a registered provider by resolving a `ProviderSpec` and building the backend.
    pub async fn from_spec(
        spec: ProviderSpec,
        api_key: &str,
        config: Option<serde_json::Value>,
    ) -> anyhow::Result<Self> {
        let provider = Provider::from_protocol(spec.protocol, api_key, config).await?;
        let backend: Arc<dyn LLMBackend> = Arc::new(provider);
        Ok(Self { spec, backend })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_id_parsing() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!("openai".parse::<ProviderId>()?, ProviderId::OpenAI);
        assert_eq!("anthropic".parse::<ProviderId>()?, ProviderId::Anthropic);
        assert_eq!("google".parse::<ProviderId>()?, ProviderId::Google);
        assert_eq!("ollama".parse::<ProviderId>()?, ProviderId::Ollama);
        assert_eq!("vllm".parse::<ProviderId>()?, ProviderId::Vllm);
        assert_eq!("vllm-graph-aware".parse::<ProviderId>()?, ProviderId::Vllm);
        assert_eq!("mock".parse::<ProviderId>()?, ProviderId::Mock);

        assert!("unknown".parse::<ProviderId>().is_err());
        Ok(())
    }

    #[test]
    fn test_provider_id_to_string() {
        assert_eq!(ProviderId::OpenAI.as_str(), "openai");
        assert_eq!(ProviderId::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderId::Google.as_str(), "google");
        assert_eq!(ProviderId::Ollama.as_str(), "ollama");
        assert_eq!(ProviderId::Vllm.as_str(), "vllm");
        assert_eq!(ProviderId::Mock.as_str(), "mock");
    }

    #[test]
    fn test_provider_id_to_protocol() {
        assert_eq!(ProviderId::OpenAI.protocol(), ProviderProtocol::OpenAI);
        assert_eq!(
            ProviderId::Anthropic.protocol(),
            ProviderProtocol::Anthropic
        );
        assert_eq!(ProviderId::Google.protocol(), ProviderProtocol::Google);
        assert_eq!(ProviderId::Ollama.protocol(), ProviderProtocol::Ollama);
        assert_eq!(ProviderId::Vllm.protocol(), ProviderProtocol::Vllm);
        assert_eq!(ProviderId::Mock.protocol(), ProviderProtocol::Mock);
    }

    #[test]
    fn test_provider_variant_identity_is_consistent() {
        let provider = Provider::Mock(MockLLMBackend::new());
        assert_eq!(provider.provider_id(), ProviderId::Mock);
        assert_eq!(provider.protocol(), ProviderProtocol::Mock);
    }
}
