//! Mock LLM backend for deterministic testing.
//!
//! `MockLLMBackend` lets you configure pre-programmed responses for testing
//! without making real network calls. It supports:
//!
//! - Static responses (same content every call)
//! - Pattern-matched responses (different answers per prompt keyword)
//! - Call counting and inspection for assertion
//! - Tool call simulation
//! - Configurable failure injection
//!
//! # Usage
//!
//! ```rust,ignore
//! use apxm_backends::llm::backends::mock::{MockLLMBackend, MockResponse};
//!
//! // Static response mock
//! let mock = MockLLMBackend::static_response("The answer is 42.");
//!
//! // Pattern-based mock
//! let mock = MockLLMBackend::new()
//!     .when_prompt_contains("summarize", "Summary: short text")
//!     .when_prompt_contains("translate", "Translation: bonjour")
//!     .default_response("I don't know");
//!
//! // Register in runtime
//! runtime.llm_registry().register("mock", mock).unwrap();
//! runtime.llm_registry().set_default("mock").unwrap();
//! ```

use super::traits::{LLMBackend, StreamChunk};
use super::{LLMRequest, LLMResponse};
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage};
use async_trait::async_trait;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_stream::Stream;

/// A recorded LLM call for inspection in tests.
#[derive(Debug, Clone)]
pub struct RecordedCall {
    /// The prompt that was sent
    pub prompt: String,
    /// The system prompt (if any)
    pub system: Option<String>,
    /// Model name requested
    pub model: String,
    /// Temperature used
    pub temperature: f32,
    /// Input token count
    pub input_tokens: usize,
    /// Output token count
    pub output_tokens: usize,
    /// Simulated latency in milliseconds
    pub latency_ms: u64,
}

/// Configurable response for pattern-based mocking.
#[derive(Debug, Clone)]
pub struct MockResponse {
    /// Content to return
    pub content: String,
    /// Simulated input token count (default: 10)
    pub input_tokens: usize,
    /// Simulated output token count (default: 20)
    pub output_tokens: usize,
    /// Finish reason (default: Stop)
    pub finish_reason: FinishReason,
}

impl MockResponse {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            input_tokens: 10,
            output_tokens: 20,
            finish_reason: FinishReason::Stop,
        }
    }

    pub fn with_tokens(mut self, input: usize, output: usize) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self
    }
}

/// A match rule: if the prompt contains `keyword`, return `response`.
#[derive(Clone)]
struct PatternRule {
    keyword: String,
    response: MockResponse,
}

/// Mock backend metrics for benchmarking.
#[derive(Debug, Clone, Default)]
pub struct MockMetrics {
    /// Total number of LLM calls
    pub total_calls: usize,
    /// Total input tokens across all calls
    pub total_input_tokens: usize,
    /// Total output tokens across all calls
    pub total_output_tokens: usize,
    /// Number of unique prompts (for CSE dedup measurement)
    pub unique_prompts: usize,
    /// Total simulated latency in milliseconds
    pub total_latency_ms: u64,
}

/// LLM backend that returns configurable pre-programmed responses.
///
/// Thread-safe and suitable for concurrent test scenarios.
#[derive(Clone)]
pub struct MockLLMBackend {
    name: String,
    model: String,
    default_response: MockResponse,
    patterns: Vec<PatternRule>,
    /// Shared call log for test assertions
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    /// If Some, return this error on every call (failure injection)
    fail_with: Option<String>,
    /// Simulated latency in milliseconds (0 = instant)
    latency_ms: u64,
    /// Tokens per second for realistic streaming (0 = instant)
    tokens_per_second: u64,
}

impl MockLLMBackend {
    /// Create a new mock with a configurable default response.
    pub fn new() -> Self {
        Self {
            name: "mock".to_string(),
            model: "mock-model".to_string(),
            default_response: MockResponse::new("Mock response."),
            patterns: Vec::new(),
            calls: Arc::new(Mutex::new(Vec::new())),
            fail_with: None,
            latency_ms: 0,
            tokens_per_second: 0,
        }
    }

    /// Create a mock that always returns `content`.
    pub fn static_response(content: impl Into<String>) -> Self {
        Self::new().default(MockResponse::new(content))
    }

    /// Set simulated latency in milliseconds.
    pub fn with_latency_ms(mut self, latency_ms: u64) -> Self {
        self.latency_ms = latency_ms;
        self
    }

    /// Set simulated tokens per second for realistic streaming.
    pub fn with_tokens_per_second(mut self, tokens_per_second: u64) -> Self {
        self.tokens_per_second = tokens_per_second;
        self
    }

    /// Create from config (for backend factory integration).
    ///
    /// Config options:
    /// - `latency_ms`: Simulated latency in milliseconds (default: 500)
    /// - `tokens_per_second`: Simulated token generation rate (default: 50)
    /// - `default_response`: Default response text (default: "Mock response")
    pub async fn from_config(
        _api_key: &str,
        config: Option<serde_json::Value>,
    ) -> anyhow::Result<Self> {
        let mut backend = Self::new();

        if let Some(cfg) = config {
            if let Some(latency) = cfg.get("latency_ms").and_then(|v| v.as_u64()) {
                backend = backend.with_latency_ms(latency);
            }
            if let Some(tps) = cfg.get("tokens_per_second").and_then(|v| v.as_u64()) {
                backend = backend.with_tokens_per_second(tps);
            }
            if let Some(resp) = cfg.get("default_response").and_then(|v| v.as_str()) {
                backend = backend.default(MockResponse::new(resp));
            }
        } else {
            // Default benchmarking config: 500ms latency, 50 tokens/sec
            backend = backend.with_latency_ms(500).with_tokens_per_second(50);
        }

        Ok(backend)
    }

    /// Set the mock's display name.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the model name returned in responses.
    pub fn model_name(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Set the default response (used when no pattern matches).
    pub fn default(mut self, response: MockResponse) -> Self {
        self.default_response = response;
        self
    }

    /// Add a pattern rule: if the prompt contains `keyword`, return `content`.
    pub fn when_prompt_contains(
        mut self,
        keyword: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.patterns.push(PatternRule {
            keyword: keyword.into(),
            response: MockResponse::new(content),
        });
        self
    }

    /// Add a pattern rule with a fully configured response.
    pub fn when_prompt_contains_response(
        mut self,
        keyword: impl Into<String>,
        response: MockResponse,
    ) -> Self {
        self.patterns.push(PatternRule {
            keyword: keyword.into(),
            response,
        });
        self
    }

    /// Configure this mock to always fail with the given error message.
    pub fn always_fail(mut self, error: impl Into<String>) -> Self {
        self.fail_with = Some(error.into());
        self
    }

    /// Return all recorded calls for test assertions.
    pub fn recorded_calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Return the number of times `generate` was called.
    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Clear the recorded call history.
    pub fn reset(&self) {
        self.calls.lock().unwrap().clear();
    }

    /// Get aggregated metrics for all recorded calls.
    pub fn metrics(&self) -> MockMetrics {
        let calls = self.calls.lock().unwrap();
        let unique_prompts = calls
            .iter()
            .map(|c| c.prompt.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();

        MockMetrics {
            total_calls: calls.len(),
            total_input_tokens: calls.iter().map(|c| c.input_tokens).sum(),
            total_output_tokens: calls.iter().map(|c| c.output_tokens).sum(),
            unique_prompts,
            total_latency_ms: calls.iter().map(|c| c.latency_ms).sum(),
        }
    }

    /// Select the response for a given prompt.
    fn select_response(&self, prompt: &str) -> &MockResponse {
        for rule in &self.patterns {
            if prompt.contains(&rule.keyword) {
                return &rule.response;
            }
        }
        &self.default_response
    }

    /// Extract the effective prompt from request.
    fn extract_prompt(&self, request: &LLMRequest) -> String {
        if request.has_messages() {
            request
                .resolved_messages()
                .iter()
                .map(|m| m.text_content())
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            request.prompt.clone()
        }
    }

    /// Record a call to the mock backend.
    fn record_call(&self, prompt: String, system: Option<String>, resp: &MockResponse) {
        self.calls.lock().unwrap().push(RecordedCall {
            prompt,
            system,
            model: self.model.clone(),
            temperature: 1.0, // default
            input_tokens: resp.input_tokens,
            output_tokens: resp.output_tokens,
            latency_ms: self.latency_ms,
        });
    }

    /// Extract the effective prompt and record the call.
    async fn record_and_extract(&self, request: &LLMRequest) -> (String, MockResponse) {
        let effective_prompt = self.extract_prompt(request);
        let resp = self.select_response(&effective_prompt).clone();

        // Simulate latency if configured
        if self.latency_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.latency_ms)).await;
        }

        self.record_call(
            effective_prompt.clone(),
            request.system_prompt.clone(),
            &resp,
        );

        (effective_prompt, resp)
    }
}

impl Default for MockLLMBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LLMBackend for MockLLMBackend {
    async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse> {
        request.validate()?;

        if let Some(ref err) = self.fail_with {
            return Err(anyhow::anyhow!("{}", err));
        }

        let (_prompt, resp) = self.record_and_extract(&request).await;

        Ok(LLMResponse::new(
            resp.content.clone(),
            self.model.clone(),
            TokenUsage::new(resp.input_tokens, resp.output_tokens),
            resp.finish_reason.clone(),
        ))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        if let Some(ref err) = self.fail_with {
            return Err(anyhow::anyhow!("Mock backend configured to fail: {}", err));
        }
        Ok(())
    }

    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: self.model.clone(),
            name: format!("Mock: {}", self.model),
            context_window: 128_000,
            supports_functions: true,
            supports_vision: false,
        }])
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send + '_>> {
        if let Some(ref err) = self.fail_with {
            let err_msg = err.clone();
            return Box::pin(tokio_stream::iter(vec![Err(anyhow::anyhow!(
                "{}", err_msg
            ))]));
        }

        let effective_prompt = self.extract_prompt(&request);
        let resp = self.select_response(&effective_prompt).clone();

        // Record the call (without latency for streaming)
        self.record_call(effective_prompt, request.system_prompt.clone(), &resp);

        let response = LLMResponse::new(
            resp.content.clone(),
            self.model.clone(),
            TokenUsage::new(resp.input_tokens, resp.output_tokens),
            resp.finish_reason.clone(),
        );

        // Split content into word-level token chunks
        let words: Vec<String> = response
            .content
            .split_whitespace()
            .map(|w| format!("{} ", w))
            .collect();
        let mut chunks: Vec<anyhow::Result<StreamChunk>> = words
            .into_iter()
            .map(|w| Ok(StreamChunk::Token(w)))
            .collect();
        chunks.push(Ok(StreamChunk::Done(response)));

        Box::pin(tokio_stream::iter(chunks))
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            streaming: true,
            vision: false,
            functions: true,
            batch: false,
            fine_tuning: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_static_response() {
        let mock = MockLLMBackend::static_response("Hello, world!");
        let req = LLMRequest::new("test");
        let resp = mock.generate(req).await.unwrap();
        assert_eq!(resp.content, "Hello, world!");
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn test_pattern_matching() {
        let mock = MockLLMBackend::new()
            .when_prompt_contains("summarize", "Summary here.")
            .when_prompt_contains("translate", "Translation here.");

        let req_sum = LLMRequest::new("Please summarize this text.");
        let resp_sum = mock.generate(req_sum).await.unwrap();
        assert_eq!(resp_sum.content, "Summary here.");

        let req_trans = LLMRequest::new("Please translate to French.");
        let resp_trans = mock.generate(req_trans).await.unwrap();
        assert_eq!(resp_trans.content, "Translation here.");

        let req_other = LLMRequest::new("What is the weather?");
        let resp_other = mock.generate(req_other).await.unwrap();
        assert_eq!(resp_other.content, "Mock response.");
    }

    #[tokio::test]
    async fn test_failure_injection() {
        let mock = MockLLMBackend::new().always_fail("Backend unavailable");
        let req = LLMRequest::new("test");
        let result = mock.generate(req).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Backend unavailable")
        );
    }

    #[tokio::test]
    async fn test_call_recording() {
        let mock = MockLLMBackend::static_response("ok");
        let req1 = LLMRequest::new("first call");
        let req2 = LLMRequest::new("second call");
        mock.generate(req1).await.unwrap();
        mock.generate(req2).await.unwrap();

        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].prompt, "first call");
        assert_eq!(calls[1].prompt, "second call");
    }

    #[tokio::test]
    async fn test_health_check_ok() {
        let mock = MockLLMBackend::new();
        assert!(mock.health_check().await.is_ok());
    }

    #[tokio::test]
    async fn test_health_check_fails_when_configured() {
        let mock = MockLLMBackend::new().always_fail("down");
        assert!(mock.health_check().await.is_err());
    }

    #[tokio::test]
    async fn test_mock_streaming() {
        use tokio_stream::StreamExt;

        let mock = MockLLMBackend::static_response("Hello world from streaming");
        let request = LLMRequest::new("test prompt");
        let mut stream = mock.generate_stream(request);

        let mut tokens = Vec::new();
        let mut done = false;
        while let Some(chunk) = stream.next().await {
            match chunk.unwrap() {
                StreamChunk::Token(t) => tokens.push(t),
                StreamChunk::Done(resp) => {
                    assert_eq!(resp.content, "Hello world from streaming");
                    done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(done);
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0], "Hello ");
        assert_eq!(tokens[1], "world ");
        assert_eq!(tokens[2], "from ");
        assert_eq!(tokens[3], "streaming ");
        // Streaming call should also be recorded
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn test_concurrent_calls_are_safe() {
        let mock = Arc::new(MockLLMBackend::static_response("concurrent"));
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let m = mock.clone();
                tokio::spawn(async move {
                    let req = LLMRequest::new(format!("call {}", i));
                    m.generate(req).await.unwrap();
                })
            })
            .collect();
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(mock.call_count(), 10);
    }
}
