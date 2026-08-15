//! Provider enum for unified backend access.
//!
//! Provides both an enum-based dispatch system and a data-driven
//! [`RegisteredProvider`] for extensible provider management.

use crate::llm::backends::{
    AnthropicBackend, GoogleBackend, GraphAwareVllmBackend, LLMBackend, LLMRequest, LLMResponse,
    LlamaCppBackend, MockLLMBackend, OllamaBackend, OpenAIBackend, StreamChunk,
};
use crate::llm::{ProviderProtocol, ProviderSpec};
use apxm_core::types::{
    ApxmGraphDescriptor, ApxmGraphHints, GraphHintCapabilities, GraphHintPlan, GraphHintProjector,
    GraphPreparationRef, GraphPrepareOutcome, GraphReleaseOutcome, ModelCapabilities, ModelInfo,
};
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
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
    LlamaCpp,
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
            ProviderId::LlamaCpp => "llamacpp",
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
            ProviderId::LlamaCpp,
            ProviderId::Mock,
        ]
    }

    /// Returns the concrete protocol for this built-in provider.
    pub fn protocol(&self) -> ProviderProtocol {
        match self {
            ProviderId::OpenAI => ProviderProtocol::OpenAI,
            ProviderId::Anthropic => ProviderProtocol::Anthropic,
            ProviderId::Google => ProviderProtocol::Google,
            ProviderId::Ollama => ProviderProtocol::Ollama,
            ProviderId::Vllm => ProviderProtocol::Vllm,
            ProviderId::LlamaCpp => ProviderProtocol::LlamaCpp,
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
            "vllm" => Ok(ProviderId::Vllm),
            "llamacpp" | "llama.cpp" | "llama-cpp" => Ok(ProviderId::LlamaCpp),
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
    LlamaCpp(LlamaCppBackend),
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
            ProviderProtocol::LlamaCpp => Ok(Provider::LlamaCpp(
                LlamaCppBackend::new(api_key, config).await?,
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
            Provider::LlamaCpp(_) => ProviderId::LlamaCpp,
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
            Provider::LlamaCpp(b) => b,
            Provider::Mock(b) => b,
        }
    }

    fn projector_ref(&self) -> &dyn GraphHintProjector {
        match self {
            Provider::OpenAI(b) => b,
            Provider::Anthropic(b) => b,
            Provider::Google(b) => b,
            Provider::Ollama(b) => b,
            Provider::Vllm(b) => b,
            Provider::LlamaCpp(b) => b,
            Provider::Mock(b) => b,
        }
    }
}

impl GraphHintProjector for Provider {
    fn graph_hint_capabilities(&self) -> GraphHintCapabilities {
        self.projector_ref().graph_hint_capabilities()
    }

    fn plan_graph_hints(&self, hints: Option<&ApxmGraphHints>) -> Result<GraphHintPlan, String> {
        self.projector_ref().plan_graph_hints(hints)
    }

    fn render_graph_hint_fields(
        &self,
        hints: &ApxmGraphHints,
        plan: &GraphHintPlan,
    ) -> Result<serde_json::Map<String, serde_json::Value>, String> {
        self.projector_ref().render_graph_hint_fields(hints, plan)
    }
}

#[async_trait]
impl LLMBackend for Provider {
    async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse> {
        self.backend_ref().generate(request).await
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send + '_>> {
        self.backend_ref().generate_stream(request)
    }

    fn name(&self) -> &str {
        self.backend_ref().name()
    }

    fn model(&self) -> &str {
        self.backend_ref().model()
    }

    fn context_window_for_model(&self, model: &str) -> Option<usize> {
        self.backend_ref().context_window_for_model(model)
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

    fn response_memoization_policy(
        &self,
    ) -> crate::llm::backends::traits::ResponseMemoizationPolicy {
        self.backend_ref().response_memoization_policy()
    }

    async fn register_graph(
        &self,
        metadata: apxm_core::types::GraphMetadata,
    ) -> anyhow::Result<()> {
        self.backend_ref().register_graph(metadata).await
    }

    async fn release_graph(&self, graph_id: &str) -> anyhow::Result<()> {
        self.backend_ref().release_graph(graph_id).await
    }

    async fn prepare_graph(
        &self,
        descriptor: ApxmGraphDescriptor,
    ) -> anyhow::Result<GraphPrepareOutcome> {
        self.backend_ref().prepare_graph(descriptor).await
    }

    async fn release_graph_preparation(
        &self,
        preparation: GraphPreparationRef,
    ) -> anyhow::Result<GraphReleaseOutcome> {
        self.backend_ref()
            .release_graph_preparation(preparation)
            .await
    }

    fn supports_auto_tool_choice(&self) -> bool {
        self.backend_ref().supports_auto_tool_choice()
    }

    async fn get_graph_status(
        &self,
        graph_id: &str,
    ) -> anyhow::Result<Option<apxm_core::types::GraphStatusSnapshot>> {
        self.backend_ref().get_graph_status(graph_id).await
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
    use std::str::FromStr;

    #[test]
    fn a_provider_id_parses_from_exactly_its_own_spelling() {
        for id in ProviderId::all_variants() {
            assert_eq!(
                ProviderId::from_str(id.as_str()).expect("a provider id parses from its own name"),
                *id,
            );
        }

        for other_name in ["vllm-graph-aware", "gemini"] {
            assert!(
                ProviderId::from_str(other_name).is_err(),
                "'{other_name}' is not a provider id, so it names no provider"
            );
        }
    }

    #[test]
    fn a_provider_protocol_parses_from_exactly_its_own_spelling() {
        for protocol in ProviderProtocol::all_variants() {
            assert_eq!(
                ProviderProtocol::from_str(protocol.as_str())
                    .expect("a protocol parses from its own name"),
                *protocol,
            );
        }

        for other_name in ["vllm-graph-aware", "gemini"] {
            assert!(
                ProviderProtocol::from_str(other_name).is_err(),
                "'{other_name}' is not a protocol name, so it names no protocol"
            );
        }
    }
}
