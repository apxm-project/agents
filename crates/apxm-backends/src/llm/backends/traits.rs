//! Core LLMBackend trait defining the unified interface.

use super::{LLMRequest, LLMResponse};
use apxm_core::types::{ModelCapabilities, ModelInfo, TokenUsage};
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

    /// Register a graph for KV-cache scheduling hints (vLLM graph-aware only).
    async fn register_graph(&self, _metadata: Value) -> anyhow::Result<()> {
        Ok(())
    }

    /// Release a graph's pinned KV-cache blocks (vLLM graph-aware only).
    async fn release_graph(&self, _graph_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::{FinishReason, LLMResponse, TokenUsage};

    #[test]
    fn test_stream_chunk_token() {
        let chunk = StreamChunk::Token("hello".into());
        assert!(matches!(chunk, StreamChunk::Token(ref t) if t == "hello"));
    }

    #[test]
    fn test_stream_chunk_thought() {
        let chunk = StreamChunk::Thought("reasoning step".into());
        assert!(matches!(chunk, StreamChunk::Thought(ref t) if t == "reasoning step"));
    }

    #[test]
    fn test_stream_chunk_usage() {
        let usage = TokenUsage::new(50, 100);
        let chunk = StreamChunk::Usage(usage.clone());
        if let StreamChunk::Usage(u) = chunk {
            assert_eq!(u.input_tokens, 50);
            assert_eq!(u.output_tokens, 100);
        } else {
            panic!("Expected Usage variant");
        }
    }

    #[test]
    fn test_stream_chunk_error() {
        let chunk = StreamChunk::Error("connection lost".into());
        assert!(matches!(chunk, StreamChunk::Error(ref e) if e == "connection lost"));
    }

    #[test]
    fn test_stream_chunk_done() {
        let resp = LLMResponse::new(
            "content",
            "model",
            TokenUsage::new(10, 20),
            FinishReason::Stop,
        );
        let chunk = StreamChunk::Done(resp);
        if let StreamChunk::Done(r) = chunk {
            assert_eq!(r.content, "content");
        } else {
            panic!("Expected Done variant");
        }
    }

    #[test]
    fn test_stream_chunk_tool_call_start() {
        let chunk = StreamChunk::ToolCallStart {
            id: "call_1".into(),
            name: "bash".into(),
        };
        if let StreamChunk::ToolCallStart { id, name } = chunk {
            assert_eq!(id, "call_1");
            assert_eq!(name, "bash");
        } else {
            panic!("Expected ToolCallStart variant");
        }
    }

    #[test]
    fn test_stream_chunk_tool_call_delta() {
        let chunk = StreamChunk::ToolCallDelta {
            id: "call_1".into(),
            arguments_delta: r#"{"key":"#.into(),
        };
        if let StreamChunk::ToolCallDelta {
            id,
            arguments_delta,
        } = chunk
        {
            assert_eq!(id, "call_1");
            assert_eq!(arguments_delta, r#"{"key":"#);
        } else {
            panic!("Expected ToolCallDelta variant");
        }
    }

    #[test]
    fn test_stream_chunk_clone() {
        let chunk = StreamChunk::Thought("test".into());
        let cloned = chunk.clone();
        assert!(matches!(cloned, StreamChunk::Thought(ref t) if t == "test"));
    }
}
