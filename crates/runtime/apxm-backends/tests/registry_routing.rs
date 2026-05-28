//! Tests for LLMRegistry routing strategies and streaming fallback.

use apxm_backends::llm::backends::mock::MockLLMBackend;
use apxm_backends::llm::backends::{LLMBackend, LLMRequest, LLMResponse, StreamChunk, TokenUsage};
use apxm_backends::llm::registry::{LLMRegistry, RoutingStrategy};
use apxm_backends::llm::wire::response_metadata;
use apxm_backends::{StreamingBackendError, StreamingFailureKind};
use apxm_core::types::{FinishReason, ModelInfo};
use futures::StreamExt;
use std::pin::Pin;

struct CommitThenFailBackend {
    name: String,
    model: String,
}

impl CommitThenFailBackend {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            model: "mock-model".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl LLMBackend for CommitThenFailBackend {
    async fn generate(&self, _request: LLMRequest) -> anyhow::Result<LLMResponse> {
        Ok(LLMResponse::new(
            "",
            self.model.as_str(),
            TokenUsage::new(0, 0),
            FinishReason::Stop,
        ))
    }

    fn generate_stream(
        &self,
        _request: LLMRequest,
    ) -> Pin<Box<dyn tokio_stream::Stream<Item = anyhow::Result<StreamChunk>> + Send + '_>> {
        Box::pin(tokio_stream::iter(vec![
            Ok(StreamChunk::Token("partial ".to_string())),
            Err(anyhow::anyhow!("connection closed")),
        ]))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: self.model.clone(),
            name: self.model.clone(),
            context_window: 128_000,
            supports_functions: true,
            supports_vision: false,
        }])
    }
}

#[tokio::test]
async fn test_round_robin_rotates_across_backends() {
    // Create registry with RoundRobin strategy
    let registry = LLMRegistry::with_strategy(RoutingStrategy::RoundRobin);

    // Register three mock backends
    let mock1 = MockLLMBackend::static_response("response1").named("backend1");
    let mock2 = MockLLMBackend::static_response("response2").named("backend2");
    let mock3 = MockLLMBackend::static_response("response3").named("backend3");

    registry.register("backend1", mock1).unwrap();
    registry.register("backend2", mock2).unwrap();
    registry.register("backend3", mock3).unwrap();

    // Round-robin treats Unknown as unroutable, so the pool must be probed
    // before requests can be served. In production this is the explicit
    // post-registration step the operator (or registration code) runs.
    registry.check_all_backends().await;

    // Make multiple requests and track which backends are selected
    let mut selected_backends = Vec::new();
    for _ in 0..9 {
        let request = LLMRequest::new("test");
        let backend_name = registry.resolve_backend_name(&request).unwrap();
        selected_backends.push(backend_name);
    }

    // Verify round-robin pattern: should cycle through all three backends
    // The order depends on insertion order in DashMap, but it should repeat every 3 calls
    assert_eq!(selected_backends.len(), 9);

    // Verify the pattern repeats
    assert_eq!(selected_backends[0], selected_backends[3]);
    assert_eq!(selected_backends[1], selected_backends[4]);
    assert_eq!(selected_backends[2], selected_backends[5]);
    assert_eq!(selected_backends[0], selected_backends[6]);
    assert_eq!(selected_backends[1], selected_backends[7]);
    assert_eq!(selected_backends[2], selected_backends[8]);
}

#[tokio::test]
async fn test_round_robin_skips_unhealthy_backends() {
    let registry = LLMRegistry::with_strategy(RoutingStrategy::RoundRobin);

    // Register three backends, one configured to fail
    let mock1 = MockLLMBackend::static_response("response1").named("backend1");
    let mock2 = MockLLMBackend::static_response("response2")
        .named("backend2")
        .always_fail("unhealthy");
    let mock3 = MockLLMBackend::static_response("response3").named("backend3");

    registry.register("backend1", mock1).unwrap();
    registry.register("backend2", mock2).unwrap();
    registry.register("backend3", mock3).unwrap();

    // Trigger health check to mark backend2 as unhealthy
    registry.check_all_backends().await;

    // Make multiple requests - should only rotate between backend1 and backend3
    let mut selected_backends = Vec::new();
    for _ in 0..6 {
        let request = LLMRequest::new("test");
        let backend_name = registry.resolve_backend_name(&request).unwrap();
        selected_backends.push(backend_name);
    }

    // Verify backend2 is never selected
    for backend in &selected_backends {
        assert_ne!(backend, "backend2");
    }

    // Verify we're rotating between the two healthy backends
    assert_eq!(selected_backends.len(), 6);
    assert_eq!(selected_backends[0], selected_backends[2]);
    assert_eq!(selected_backends[1], selected_backends[3]);
    assert_eq!(selected_backends[0], selected_backends[4]);
    assert_eq!(selected_backends[1], selected_backends[5]);
}

#[tokio::test]
async fn test_round_robin_single_backend() {
    let registry = LLMRegistry::with_strategy(RoutingStrategy::RoundRobin);

    let mock = MockLLMBackend::static_response("response").named("only");
    registry.register("only", mock).unwrap();
    registry.check_all_backends().await;

    // Multiple requests should all use the same backend
    for _ in 0..5 {
        let request = LLMRequest::new("test");
        let backend_name = registry.resolve_backend_name(&request).unwrap();
        assert_eq!(backend_name, "only");
    }
}

#[tokio::test]
async fn test_round_robin_counter_overflow_handling() {
    // This test verifies that counter overflow doesn't cause panics
    let registry = LLMRegistry::with_strategy(RoutingStrategy::RoundRobin);

    let mock1 = MockLLMBackend::static_response("response1").named("backend1");
    let mock2 = MockLLMBackend::static_response("response2").named("backend2");

    registry.register("backend1", mock1).unwrap();
    registry.register("backend2", mock2).unwrap();
    registry.check_all_backends().await;

    // Simulate many requests (won't actually overflow, but tests the modulo logic)
    for _ in 0..1000 {
        let request = LLMRequest::new("test");
        let backend_name = registry.resolve_backend_name(&request);
        assert!(backend_name.is_ok());
    }
}

#[tokio::test]
async fn test_streaming_fallback_primary_succeeds() {
    let registry = LLMRegistry::new();

    let primary = MockLLMBackend::static_response("Primary response").named("primary");
    registry.register("primary", primary).unwrap();
    registry.set_default("primary").unwrap();

    let request = LLMRequest::new("test");
    let mut stream = registry.generate_stream_with_fallback(&request);

    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            apxm_backends::llm::backends::StreamChunk::Token(t) => chunks.push(t),
            apxm_backends::llm::backends::StreamChunk::Done(resp) => {
                assert_eq!(resp.content, "Primary response");
                assert_eq!(
                    resp.metadata
                        .get(response_metadata::APXM_BACKEND_NAME)
                        .and_then(serde_json::Value::as_str),
                    Some("primary")
                );
                assert_eq!(
                    resp.metadata
                        .get(response_metadata::APXM_PRIMARY_BACKEND)
                        .and_then(serde_json::Value::as_str),
                    Some("primary")
                );
                assert_eq!(
                    resp.metadata
                        .get(response_metadata::APXM_FALLBACK_USED)
                        .and_then(serde_json::Value::as_bool),
                    Some(false)
                );
                break;
            }
            _ => {}
        }
    }

    // Should have received token chunks
    assert!(!chunks.is_empty());
}

#[tokio::test]
async fn test_streaming_fallback_primary_fails_first_chunk() {
    let registry = LLMRegistry::new();

    // Primary fails, fallback succeeds
    let primary = MockLLMBackend::static_response("won't see this")
        .named("primary")
        .always_fail("Primary failed");
    let fallback = MockLLMBackend::static_response("Fallback response").named("fallback");

    registry.register("primary", primary).unwrap();
    registry.register("fallback", fallback).unwrap();
    registry.set_default("primary").unwrap();
    registry
        .set_fallback("primary", vec!["fallback".to_string()])
        .unwrap();

    let request = LLMRequest::new("test");
    let mut stream = registry.generate_stream_with_fallback(&request);

    let mut chunks = Vec::new();
    let mut final_content = None;
    let mut final_backend = None;
    let mut fallback_used = None;
    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            apxm_backends::llm::backends::StreamChunk::Token(t) => chunks.push(t),
            apxm_backends::llm::backends::StreamChunk::Done(resp) => {
                final_backend = resp
                    .metadata
                    .get(response_metadata::APXM_BACKEND_NAME)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                fallback_used = resp
                    .metadata
                    .get(response_metadata::APXM_FALLBACK_USED)
                    .and_then(serde_json::Value::as_bool);
                final_content = Some(resp.content);
                break;
            }
            _ => {}
        }
    }

    // Should have fallen back to fallback backend
    assert_eq!(final_content, Some("Fallback response".to_string()));
    assert_eq!(final_backend, Some("fallback".to_string()));
    assert_eq!(fallback_used, Some(true));
    assert!(!chunks.is_empty());
}

#[tokio::test]
async fn test_streaming_fallback_all_backends_fail() {
    let registry = LLMRegistry::new();

    // Both primary and fallback fail
    let primary = MockLLMBackend::static_response("won't see this")
        .named("primary")
        .always_fail("Primary failed");
    let fallback = MockLLMBackend::static_response("won't see this either")
        .named("fallback")
        .always_fail("Fallback also failed");

    registry.register("primary", primary).unwrap();
    registry.register("fallback", fallback).unwrap();
    registry.set_default("primary").unwrap();
    registry
        .set_fallback("primary", vec!["fallback".to_string()])
        .unwrap();

    let request = LLMRequest::new("test");
    let mut stream = registry.generate_stream_with_fallback(&request);

    // Should get an error
    let result = stream.next().await;
    assert!(result.is_some());
    let chunk = result.unwrap();
    assert!(chunk.is_err());
    let error = chunk.unwrap_err().to_string();
    assert!(error.contains("All streaming backends failed"), "{error}");
}

#[tokio::test]
async fn test_streaming_fallback_empty_primary_stream() {
    let registry = LLMRegistry::new();

    // Create a mock that returns an empty stream by failing
    // (The mock backend's generate_stream doesn't have a way to return empty stream,
    // so we'll use the failing backend which returns error on first chunk)
    let primary = MockLLMBackend::static_response("empty")
        .named("primary")
        .always_fail("Empty stream");
    let fallback = MockLLMBackend::static_response("Fallback works").named("fallback");

    registry.register("primary", primary).unwrap();
    registry.register("fallback", fallback).unwrap();
    registry.set_default("primary").unwrap();
    registry
        .set_fallback("primary", vec!["fallback".to_string()])
        .unwrap();

    let request = LLMRequest::new("test");
    let mut stream = registry.generate_stream_with_fallback(&request);

    let mut final_content = None;
    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            apxm_backends::llm::backends::StreamChunk::Done(resp) => {
                final_content = Some(resp.content);
                break;
            }
            _ => {}
        }
    }

    // Should have fallen back successfully
    assert_eq!(final_content, Some("Fallback works".to_string()));
}

#[tokio::test]
async fn test_streaming_fallback_multiple_fallbacks() {
    let registry = LLMRegistry::new();

    // Primary fails, first fallback fails, second fallback succeeds
    let primary = MockLLMBackend::static_response("primary")
        .named("primary")
        .always_fail("Primary failed");
    let fallback1 = MockLLMBackend::static_response("fallback1")
        .named("fallback1")
        .always_fail("Fallback1 failed");
    let fallback2 = MockLLMBackend::static_response("Fallback2 success").named("fallback2");

    registry.register("primary", primary).unwrap();
    registry.register("fallback1", fallback1).unwrap();
    registry.register("fallback2", fallback2).unwrap();
    registry.set_default("primary").unwrap();
    registry
        .set_fallback(
            "primary",
            vec!["fallback1".to_string(), "fallback2".to_string()],
        )
        .unwrap();

    let request = LLMRequest::new("test");
    let mut stream = registry.generate_stream_with_fallback(&request);

    let mut final_content = None;
    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            apxm_backends::llm::backends::StreamChunk::Done(resp) => {
                final_content = Some(resp.content);
                break;
            }
            _ => {}
        }
    }

    // Should have fallen back to fallback2
    assert_eq!(final_content, Some("Fallback2 success".to_string()));
}

#[tokio::test]
async fn test_streaming_no_mid_stream_switching() {
    let registry = LLMRegistry::new();

    // Primary succeeds on first chunk - should commit to it even if it hypothetically
    // failed later (though our mock doesn't simulate that scenario)
    let primary = MockLLMBackend::static_response("Primary complete response").named("primary");
    let fallback = MockLLMBackend::static_response("Fallback response").named("fallback");

    registry.register("primary", primary).unwrap();
    registry.register("fallback", fallback).unwrap();
    registry.set_default("primary").unwrap();
    registry
        .set_fallback("primary", vec!["fallback".to_string()])
        .unwrap();

    let request = LLMRequest::new("test");
    let mut stream = registry.generate_stream_with_fallback(&request);

    let mut final_content = None;
    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            apxm_backends::llm::backends::StreamChunk::Done(resp) => {
                final_content = Some(resp.content);
                break;
            }
            _ => {}
        }
    }

    // Should use primary backend (not fallback)
    assert_eq!(final_content, Some("Primary complete response".to_string()));
}

#[tokio::test]
async fn test_committed_stream_error_exposes_backend_name() {
    let registry = LLMRegistry::new();

    let primary = CommitThenFailBackend::new("primary");
    let fallback = MockLLMBackend::static_response("Fallback response").named("fallback");

    registry.register("primary", primary).unwrap();
    registry.register("fallback", fallback).unwrap();
    registry.set_default("primary").unwrap();
    registry
        .set_fallback("primary", vec!["fallback".to_string()])
        .unwrap();

    let request = LLMRequest::new("test");
    let mut stream = registry.generate_stream_with_fallback(&request);

    match stream.next().await.unwrap().unwrap() {
        StreamChunk::Token(token) => assert_eq!(token, "partial "),
        chunk => panic!("expected committed token before failure, got {chunk:?}"),
    }

    let error = stream.next().await.unwrap().unwrap_err();
    let stream_error = error
        .downcast_ref::<StreamingBackendError>()
        .expect("committed stream errors should carry backend metadata");

    assert_eq!(stream_error.backend_name(), "primary");
    assert_eq!(stream_error.kind(), StreamingFailureKind::BackendError);
}
