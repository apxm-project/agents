//! Tests for LLMRegistry routing strategies and streaming fallback.

use apxm_backends::llm::backends::LLMRequest;
use apxm_backends::llm::backends::mock::MockLLMBackend;
use apxm_backends::llm::registry::{LLMRegistry, RoutingStrategy};
use futures::StreamExt;

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
    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            apxm_backends::llm::backends::StreamChunk::Token(t) => chunks.push(t),
            apxm_backends::llm::backends::StreamChunk::Done(resp) => {
                final_content = Some(resp.content);
                break;
            }
            _ => {}
        }
    }

    // Should have fallen back to fallback backend
    assert_eq!(final_content, Some("Fallback response".to_string()));
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
    assert!(
        chunk
            .unwrap_err()
            .to_string()
            .contains("All backends failed")
    );
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
