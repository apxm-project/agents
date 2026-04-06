//! Integration test for invalid task rejection at LLM backend boundary.
//!
//! These tests verify that invalid task payloads (e.g., "INVALID TASK")
//! are rejected at the validation boundary before reaching backend execution.

use apxm_backends::llm::backends::{LLMBackend, MockLLMBackend};
use apxm_backends::{LLMRegistry, LLMRequest, Message, Role};

#[tokio::test]
async fn test_backend_directly_rejects_invalid_task() {
    // Test validation at the backend level (bypasses registry wrapper)
    let backend = MockLLMBackend::static_response("SHOULD NEVER EXECUTE");

    // Test 1: Direct "INVALID TASK" prompt should fail validation
    let request = LLMRequest::new("INVALID TASK");
    let result = backend.generate(request).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("INVALID TASK"),
        "Expected error about invalid task, got: {}",
        err_msg
    );

    // Test 2: Case-insensitive rejection
    let request = LLMRequest::new("invalid task");
    let result = backend.generate(request).await;
    assert!(result.is_err());

    // Test 3: With whitespace
    let request = LLMRequest::new("  INVALID TASK  ");
    let result = backend.generate(request).await;
    assert!(result.is_err());

    // Test 4: Valid tasks should pass through
    let request = LLMRequest::new("This is a valid task");
    let result = backend.generate(request).await;
    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.content, "SHOULD NEVER EXECUTE");
}

#[tokio::test]
async fn test_invalid_task_in_messages_rejected() {
    let backend = MockLLMBackend::static_response("should not reach");

    // Invalid task in messages should be rejected
    let messages = vec![Message::text(Role::User, "INVALID TASK")];
    let request = LLMRequest::from_messages(messages);
    let result = backend.generate(request).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("INVALID TASK"),
        "Expected error about invalid task, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_backend_never_executes_invalid_task() {
    // This test verifies that invalid tasks are rejected at validation
    // before the mock backend can return its response.

    let backend = MockLLMBackend::static_response("SHOULD NEVER EXECUTE");

    // Invalid request should fail at validation
    let request = LLMRequest::new("INVALID TASK");
    let result = backend.generate(request).await;

    // Should error with validation message, not execute the backend
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("INVALID TASK"));
    // The backend response should never be returned
    assert!(!err_msg.contains("SHOULD NEVER EXECUTE"));
}

#[tokio::test]
async fn test_registry_propagates_validation_error() {
    // Test that LLMRegistry propagates validation errors (even though wrapped)
    let llm_registry = LLMRegistry::new();
    llm_registry
        .register("mock", MockLLMBackend::static_response("should not execute"))
        .unwrap();
    llm_registry.set_default("mock").unwrap();

    let request = LLMRequest::new("INVALID TASK");
    let result = llm_registry.generate(request).await;

    // Should error (registry wraps it with context, but error is propagated)
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    // The error chain should include the validation failure
    assert!(
        err_msg.contains("INVALID TASK") || err_msg.contains("failed"),
        "Expected validation error, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_model_router_propagates_validation_error() {
    use apxm_runtime::{ModelRouter, ModelRouterConfig};
    use std::sync::Arc;

    let llm_registry = Arc::new(LLMRegistry::new());
    llm_registry
        .register("mock", MockLLMBackend::static_response("should not execute"))
        .unwrap();
    llm_registry.set_default("mock").unwrap();

    let router = ModelRouter::new(llm_registry.clone(), ModelRouterConfig::default()).unwrap();

    // Invalid task should be rejected
    let request = LLMRequest::new("INVALID TASK");
    let result = router.generate(request).await;

    assert!(result.is_err());
    // ModelRouter may wrap the error, but validation should prevent execution
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("INVALID TASK") || err_msg.contains("failed"),
        "Expected validation error, got: {}",
        err_msg
    );
}
