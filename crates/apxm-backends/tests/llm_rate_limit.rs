//! Integration tests for LLM backend rate limiting.

use apxm_backends::llm::backends::openai::OpenAIBackend;
use apxm_backends::llm::provider::Provider;
use apxm_backends::{LLMRegistry, LLMRequest, RateLimitConfig};
use std::collections::HashMap;

#[tokio::test]
async fn test_rate_limit_allows_within_capacity() -> Result<(), Box<dyn std::error::Error>> {
    let mut rate_limits = HashMap::new();
    rate_limits.insert(
        "test-backend".to_string(),
        RateLimitConfig {
            capacity: 3,
            tokens_per_second: 1.0,
        },
    );

    let registry = LLMRegistry::with_rate_limits(rate_limits)?;

    let backend = Provider::OpenAI(OpenAIBackend::new("test-key", None).await?);
    registry.register("test-backend", backend)?;

    // All three requests should be allowed
    for i in 0..3 {
        let request = LLMRequest::new(format!("Request {}", i)).with_backend("test-backend");
        let result = registry.generate(request).await;

        // We expect the backend to fail (invalid key), but NOT due to rate limiting
        // Rate limiting happens before backend dispatch
        assert!(
            result.is_err(),
            "Expected backend error, not rate limit error"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            !err_msg.contains("rate limited"),
            "Request {} should not be rate limited",
            i
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_rate_limit_rejects_after_capacity_exhausted(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rate_limits = HashMap::new();
    rate_limits.insert(
        "test-backend".to_string(),
        RateLimitConfig {
            capacity: 2,
            tokens_per_second: 1.0,
        },
    );

    let registry = LLMRegistry::with_rate_limits(rate_limits)?;

    let backend = Provider::OpenAI(OpenAIBackend::new("test-key", None).await?);
    registry.register("test-backend", backend)?;

    // First two requests consume the capacity
    for i in 0..2 {
        let request = LLMRequest::new(format!("Request {}", i)).with_backend("test-backend");
        let result = registry.generate(request).await;
        let err_msg = result.unwrap_err().to_string();
        assert!(
            !err_msg.contains("rate limited"),
            "Request {} should not be rate limited",
            i
        );
    }

    // Third request should be rate limited
    let request = LLMRequest::new("Request 3").with_backend("test-backend");
    let result = registry.generate(request).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_chain = format!("{:?}", err);
    assert!(
        err_chain.contains("rate limited"),
        "Request 3 should be rate limited, got: {}",
        err_chain
    );

    Ok(())
}

#[tokio::test]
async fn test_unconfigured_backend_is_unlimited() -> Result<(), Box<dyn std::error::Error>> {
    let rate_limits = HashMap::new(); // No rate limits configured

    let registry = LLMRegistry::with_rate_limits(rate_limits)?;

    let backend = Provider::OpenAI(OpenAIBackend::new("test-key", None).await?);
    registry.register("unlimited-backend", backend)?;

    // Many requests should all be allowed (not rate limited)
    for i in 0..10 {
        let request = LLMRequest::new(format!("Request {}", i)).with_backend("unlimited-backend");
        let result = registry.generate(request).await;

        // Should fail due to invalid backend, not rate limiting
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            !err_msg.contains("rate limited"),
            "Request {} should not be rate limited",
            i
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_independent_backend_rate_limits() -> Result<(), Box<dyn std::error::Error>> {
    let mut rate_limits = HashMap::new();
    rate_limits.insert(
        "backend-a".to_string(),
        RateLimitConfig {
            capacity: 1,
            tokens_per_second: 1.0,
        },
    );
    rate_limits.insert(
        "backend-b".to_string(),
        RateLimitConfig {
            capacity: 2,
            tokens_per_second: 1.0,
        },
    );

    let registry = LLMRegistry::with_rate_limits(rate_limits)?;

    let backend_a = Provider::OpenAI(OpenAIBackend::new("test-key-a", None).await?);
    let backend_b = Provider::OpenAI(OpenAIBackend::new("test-key-b", None).await?);
    registry.register("backend-a", backend_a)?;
    registry.register("backend-b", backend_b)?;

    // Backend A: first request allowed, second rate limited
    let req_a1 = LLMRequest::new("A1").with_backend("backend-a");
    let result = registry.generate(req_a1).await;
    let err_chain = format!("{:?}", result.unwrap_err());
    assert!(!err_chain.contains("rate limited"));

    let req_a2 = LLMRequest::new("A2").with_backend("backend-a");
    let result = registry.generate(req_a2).await;
    let err_chain = format!("{:?}", result.unwrap_err());
    assert!(err_chain.contains("rate limited"));

    // Backend B: first two requests allowed, third rate limited
    let req_b1 = LLMRequest::new("B1").with_backend("backend-b");
    let result = registry.generate(req_b1).await;
    let err_chain = format!("{:?}", result.unwrap_err());
    assert!(!err_chain.contains("rate limited"));

    let req_b2 = LLMRequest::new("B2").with_backend("backend-b");
    let result = registry.generate(req_b2).await;
    let err_chain = format!("{:?}", result.unwrap_err());
    assert!(!err_chain.contains("rate limited"));

    let req_b3 = LLMRequest::new("B3").with_backend("backend-b");
    let result = registry.generate(req_b3).await;
    let err_chain = format!("{:?}", result.unwrap_err());
    assert!(err_chain.contains("rate limited"));

    Ok(())
}

#[test]
fn test_invalid_rate_limit_config_zero_capacity() {
    let mut rate_limits = HashMap::new();
    rate_limits.insert(
        "bad-backend".to_string(),
        RateLimitConfig {
            capacity: 0,
            tokens_per_second: 1.0,
        },
    );

    let result = LLMRegistry::with_rate_limits(rate_limits);
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("rate limit capacity must be > 0"));
    }
}

#[test]
fn test_invalid_rate_limit_config_invalid_tokens_per_second() {
    let mut rate_limits = HashMap::new();
    rate_limits.insert(
        "bad-backend".to_string(),
        RateLimitConfig {
            capacity: 10,
            tokens_per_second: 0.0,
        },
    );

    let result = LLMRegistry::with_rate_limits(rate_limits);
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("tokens_per_second must be finite and > 0"));
    }
}

#[tokio::test]
async fn test_rate_limit_config_serialization() -> Result<(), Box<dyn std::error::Error>> {
    let config = RateLimitConfig {
        capacity: 5,
        tokens_per_second: 2.5,
    };

    // Serialize to JSON
    let json = serde_json::to_string(&config)?;
    assert!(json.contains("\"capacity\":5"));
    assert!(json.contains("\"tokens_per_second\":2.5"));

    // Deserialize back
    let deserialized: RateLimitConfig = serde_json::from_str(&json)?;
    assert_eq!(deserialized.capacity, 5);
    assert_eq!(deserialized.tokens_per_second, 2.5);

    Ok(())
}
