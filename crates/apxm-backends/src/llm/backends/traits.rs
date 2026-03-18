//! Core LLMBackend trait defining the unified interface.

use super::{LLMRequest, LLMResponse};
use apxm_core::types::{ModelCapabilities, ModelInfo};
use async_trait::async_trait;
use serde_json::Value;
use std::pin::Pin;
use tokio_stream::Stream;

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
}

/// Core trait that all LLM backends must implement.
///
/// Provides a unified interface for different providers (OpenAI, Anthropic, etc.)
/// allowing seamless switching between implementations.
#[async_trait]
pub trait LLMBackend: Send + Sync {
    /// Generate a response from the given request.
    async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse>;

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

    /// Check if this backend is currently healthy/reachable.
    async fn health_check(&self) -> anyhow::Result<()>;

    /// Get list of available models this provider supports.
    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>>;

    /// Get provider-specific capabilities.
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities::default()
    }

    /// Get provider-specific metadata as JSON.
    fn metadata(&self) -> Value {
        serde_json::json!({
            "name": self.name(),
            "model": self.model(),
            "capabilities": serde_json::to_value(self.capabilities()).unwrap_or(Value::Null),
        })
    }
}
