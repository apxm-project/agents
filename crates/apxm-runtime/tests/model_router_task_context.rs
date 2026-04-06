//! Integration tests for model router task context validation.

use apxm_backends::LLMRequest;
use apxm_core::error::RuntimeError;
use apxm_runtime::model_router::validate_task_context;

#[test]
fn validate_task_context_rejects_empty_request() {
    let request = LLMRequest::new("");
    let err = validate_task_context(&request).unwrap_err();

    match err {
        RuntimeError::MissingTaskContext {
            message,
            checked_fields,
            recovery_sources,
            ..
        } => {
            assert!(message.contains("Insufficient task context"));
            assert!(checked_fields.contains(&"prompt".to_string()));
            assert!(recovery_sources.contains(&"apxm_task_spec".to_string()));
        }
        other => panic!("Expected MissingTaskContext, got: {:?}", other),
    }
}

#[test]
fn validate_task_context_accepts_prompt() {
    let request = LLMRequest::new("Implement feature X");
    assert!(validate_task_context(&request).is_ok());
}

#[test]
fn validate_task_context_accepts_metadata_task() {
    let mut request = LLMRequest::new("");
    request.metadata.insert(
        "task".to_string(),
        serde_json::Value::String("Fix bug in parser".to_string()),
    );
    assert!(validate_task_context(&request).is_ok());
}

#[test]
fn validate_task_context_accepts_metadata_issue_ref() {
    let mut request = LLMRequest::new("");
    request.metadata.insert(
        "issue_ref".to_string(),
        serde_json::Value::String("APXM-123".to_string()),
    );
    assert!(validate_task_context(&request).is_ok());
}

#[test]
fn validate_task_context_rejects_metadata_only() {
    let mut request = LLMRequest::new("");
    request.metadata.insert(
        "name".to_string(),
        serde_json::Value::String("coder".to_string()),
    );
    request.metadata.insert(
        "process_id".to_string(),
        serde_json::Value::String("019d609c-35b5-7dc0-aa5f-02286de13e34".to_string()),
    );
    request.metadata.insert(
        "profile".to_string(),
        serde_json::Value::String("claude".to_string()),
    );

    let err = validate_task_context(&request).unwrap_err();

    match err {
        RuntimeError::MissingTaskContext {
            message,
            metadata,
            checked_fields,
            recovery_sources,
            hydration_attempted,
        } => {
            assert!(message.contains("Insufficient task context"));
            assert_eq!(metadata.get("name"), Some(&"coder".to_string()));
            assert_eq!(metadata.get("profile"), Some(&"claude".to_string()));
            assert!(checked_fields.contains(&"prompt".to_string()));
            assert!(recovery_sources.contains(&"apxm_task_spec".to_string()));
            assert!(!hydration_attempted);
        }
        other => panic!("Expected MissingTaskContext, got: {:?}", other),
    }
}

#[test]
fn error_includes_diagnostic_metadata() {
    let mut request = LLMRequest::new("");
    request.metadata.insert(
        "name".to_string(),
        serde_json::Value::String("coder".to_string()),
    );
    request.metadata.insert(
        "process_id".to_string(),
        serde_json::Value::String("019d609c-35b5-7dc0-aa5f-02286de13e34".to_string()),
    );
    request.backend = Some("claude".to_string());
    request.model = Some("claude-sonnet-4".to_string());

    let err = validate_task_context(&request).unwrap_err();

    match err {
        RuntimeError::MissingTaskContext {
            metadata,
            checked_fields,
            recovery_sources,
            hydration_attempted,
            ..
        } => {
            // Diagnostic metadata should be captured
            assert_eq!(metadata.get("name"), Some(&"coder".to_string()));
            assert_eq!(metadata.get("backend"), Some(&"claude".to_string()));
            assert_eq!(metadata.get("model"), Some(&"claude-sonnet-4".to_string()));

            // Recovery contract should be present
            assert!(checked_fields.contains(&"prompt".to_string()));
            assert!(checked_fields.contains(&"metadata.task".to_string()));
            assert!(recovery_sources.contains(&"parent_process_context".to_string()));
            assert!(!hydration_attempted);
        }
        other => panic!("Expected MissingTaskContext, got: {:?}", other),
    }
}

#[test]
fn ignores_whitespace_only_values() {
    let mut request = LLMRequest::new("   \n\t  ");
    request.metadata.insert(
        "task".to_string(),
        serde_json::Value::String("   ".to_string()),
    );
    request.metadata.insert(
        "spec".to_string(),
        serde_json::Value::String("\n\t".to_string()),
    );

    let err = validate_task_context(&request).unwrap_err();
    assert!(matches!(err, RuntimeError::MissingTaskContext { .. }));
}
