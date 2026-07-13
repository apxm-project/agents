//! Core LLMBackend trait defining the unified interface.

use super::{LLMRequest, LLMResponse};
use apxm_core::types::{
    BackendGraphCapabilities, GraphMetadata, GraphStatusSnapshot, ModelCapabilities, ModelInfo,
    TokenUsage,
};
use async_trait::async_trait;
use serde_json::Value;
use std::pin::Pin;
use tokio_stream::Stream;

/// Backend-declared ownership of exact-response caching.
///
/// The runtime consumes this policy instead of inferring cache behavior from a
/// provider name. Backends with an observable prefix/KV cache can preserve
/// their request telemetry by preferring that cache layer; every other backend
/// uses APXM's exact-response memo cache when a node is otherwise eligible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseMemoizationPolicy {
    /// Use APXM's exact-response memo cache for deterministic requests.
    RuntimeExact,
    /// Prefer the backend's own prefix/KV cache over APXM exact-response memoization.
    BackendPrefix,
}

/// Backend support for grouped model execution with per-request outcomes.
///
/// APXM only treats a batch group as executable when the backend can return one
/// correlated result for every submitted request. A generic provider batch API
/// without stable request/result correlation is not an APXM workflow batch
/// capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CorrelatedBatchingCapability {
    /// The backend exposes no correlated request-batch contract.
    #[default]
    Unsupported,
    /// The backend accepts independent requests and returns their outcomes by
    /// the supplied correlation ids, up to the declared batch size.
    CorrelatedOutcomes { max_batch_size: usize },
}

impl CorrelatedBatchingCapability {
    /// Whether this backend can execute a non-empty correlated request batch.
    pub const fn is_supported(self) -> bool {
        matches!(
            self,
            Self::CorrelatedOutcomes { max_batch_size } if max_batch_size > 0
        )
    }
}

/// One independently routable model request in a compiler-approved batch.
///
/// The caller owns the correlation id. Backends must return that exact id with
/// the outcome instead of relying on submission order.
#[derive(Debug, Clone)]
pub struct CorrelatedLLMRequest {
    pub correlation_id: String,
    pub request: LLMRequest,
}

/// A terminal outcome for one request in a correlated model batch.
///
/// A per-request failure remains associated with its original correlation id,
/// so a scheduler can fail exactly the affected node without attributing a
/// neighbouring request's response to it.
#[derive(Debug, Clone)]
pub enum CorrelatedLLMOutcome {
    Response {
        correlation_id: String,
        response: LLMResponse,
    },
    Failure {
        correlation_id: String,
        message: String,
    },
}

impl CorrelatedLLMOutcome {
    /// Return the caller-provided request correlation identity.
    pub fn correlation_id(&self) -> &str {
        match self {
            Self::Response { correlation_id, .. } | Self::Failure { correlation_id, .. } => {
                correlation_id
            }
        }
    }
}

/// A chunk emitted during streaming LLM generation.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// A text token from the model.
    Token(String),
    /// Start of a tool call.
    ToolCallStart { id: String, name: String },
    /// Incremental arguments for a tool call.
    ToolCallDelta { id: String, arguments_delta: String },
    /// Final response (streaming complete).
    Done(LLMResponse),
    /// Extended thinking / reasoning token from the model.
    Thought(String),
    /// Incremental token usage update.
    Usage(TokenUsage),
    /// Stream-level error (non-fatal; the stream may continue or end).
    Error(String),
}

/// Core trait that all LLM backends must implement.
///
/// Provides a unified interface for different providers (OpenAI, Anthropic, etc.)
/// allowing seamless switching between implementations.
#[async_trait]
pub trait LLMBackend: Send + Sync {
    /// Generate a response from the given request.
    async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse>;

    /// Execute a compiler-approved batch with stable per-request correlation.
    ///
    /// The default deliberately fails rather than decomposing the batch into
    /// individual provider calls. A backend may advertise correlated batching
    /// only when it implements this transport contract.
    async fn generate_correlated_batch(
        &self,
        _requests: Vec<CorrelatedLLMRequest>,
    ) -> anyhow::Result<Vec<CorrelatedLLMOutcome>> {
        anyhow::bail!("backend does not implement correlated batch dispatch")
    }

    /// Generate a streaming response. Default wraps generate() into a single Done chunk.
    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send + '_>> {
        Box::pin(futures::stream::once(async move {
            let response = self.generate(request).await?;
            Ok(StreamChunk::Done(response))
        }))
    }

    /// Get the display name of this backend.
    fn name(&self) -> &str;

    /// Get the currently configured model name.
    fn model(&self) -> &str;

    /// Return configured context-window evidence for one resolved model.
    ///
    /// `None` means the backend registration does not declare a context limit.
    /// Callers must not substitute a provider or model default for missing
    /// evidence.
    fn context_window_for_model(&self, _model: &str) -> Option<usize> {
        None
    }

    /// Check if this backend is currently healthy/reachable.
    async fn health_check(&self) -> anyhow::Result<()>;

    /// Get list of available models this provider supports.
    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>>;

    /// Get provider-specific capabilities.
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities::default()
    }

    /// Get graph-aware backend capability evidence.
    ///
    /// Generic providers default to no graph-aware support, while preserving
    /// any structured-output support advertised by the model capability API.
    fn graph_capabilities(&self) -> BackendGraphCapabilities {
        BackendGraphCapabilities {
            supports_structured_outputs: self.capabilities().structured_outputs,
            ..BackendGraphCapabilities::default()
        }
    }

    /// Declare the cache layer that owns deterministic request reuse.
    fn response_memoization_policy(&self) -> ResponseMemoizationPolicy {
        ResponseMemoizationPolicy::RuntimeExact
    }

    /// Declare whether this backend can execute compiler-approved request
    /// batches while preserving one correlated outcome per request.
    fn correlated_batching_capability(&self) -> CorrelatedBatchingCapability {
        CorrelatedBatchingCapability::Unsupported
    }

    /// Get provider-specific metadata as JSON.
    fn metadata(&self) -> Value {
        serde_json::json!({
            "name": self.name(),
            "model": self.model(),
            "capabilities": serde_json::to_value(self.capabilities()).unwrap_or(Value::Null),
            "graph_capabilities": serde_json::to_value(self.graph_capabilities()).unwrap_or(Value::Null),
        })
    }

    /// Register graph metadata for graph-aware backend scheduling.
    async fn register_graph(&self, _metadata: GraphMetadata) -> anyhow::Result<()> {
        Ok(())
    }

    /// Release backend state associated with a registered graph.
    async fn release_graph(&self, _graph_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    /// Whether the backend implements graph-aware extensions
    /// (`register_graph`, `release_graph`). Default `false`.
    fn supports_graph_extensions(&self) -> bool {
        false
    }

    /// Whether the backend accepts `tool_choice="auto"`. Default `true`.
    fn supports_auto_tool_choice(&self) -> bool {
        true
    }

    /// Returns `Some(GraphStatusSnapshot)` for graph-aware backends; `None`
    /// for backends without per-graph status (default).
    async fn get_graph_status(
        &self,
        _graph_id: &str,
    ) -> anyhow::Result<Option<GraphStatusSnapshot>> {
        Ok(None)
    }

    /// Generate a unique execution id for graph registration. Default uses uuid v4.
    /// Backends with custom counters (e.g. vLLM) may override.
    fn next_execution_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::CorrelatedBatchingCapability;

    #[test]
    fn batching_requires_correlated_outcomes_and_a_positive_limit() {
        assert!(!CorrelatedBatchingCapability::Unsupported.is_supported());
        assert!(
            !CorrelatedBatchingCapability::CorrelatedOutcomes { max_batch_size: 0 }.is_supported()
        );
        assert!(
            CorrelatedBatchingCapability::CorrelatedOutcomes { max_batch_size: 2 }.is_supported()
        );
    }
}
