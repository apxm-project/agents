//! Task context validation for model router requests.
//!
//! Validates that LLM requests contain sufficient task context beyond
//! process/profile metadata, preventing under-specified coding requests.

use apxm_backends::LLMRequest;
use apxm_core::error::RuntimeError;
use std::collections::BTreeMap;

/// Check if the request has sufficient task context.
///
/// Task context can come from:
/// - `prompt` field (non-empty text)
/// - `messages` field (non-empty conversation)
/// - `metadata` fields with task-related keys
///
/// Returns `true` if at least one task-bearing field is non-empty.
pub fn has_task_context(request: &LLMRequest) -> bool {
    // Check main prompt field
    if !request.prompt.trim().is_empty() {
        return true;
    }

    // Check structured messages
    if !request.messages.is_empty() {
        return true;
    }

    // Check metadata for task-related fields
    const TASK_KEYS: &[&str] = &[
        "task",
        "spec",
        "objective",
        "issue",
        "issue_ref",
        "repository",
        "feature",
        "description",
        "context",
    ];

    for key in TASK_KEYS {
        if let Some(value) = request.metadata.get(*key) {
            if let Some(text) = value.as_str() {
                if !text.trim().is_empty() {
                    return true;
                }
            }
        }
    }

    false
}

/// Validate that the request contains sufficient task context.
///
/// Returns `Ok(())` if the request has task context, or an error describing
/// what was missing and how to fix it.
pub fn validate_task_context(request: &LLMRequest) -> Result<(), RuntimeError> {
    if has_task_context(request) {
        return Ok(());
    }

    // Build diagnostic metadata from available fields
    let mut metadata = BTreeMap::new();

    // Extract metadata fields that might identify the agent/process
    for (key, value) in &request.metadata {
        if ["name", "process_id", "profile", "spawned_by", "agent_id"].contains(&key.as_str()) {
            if let Some(text) = value.as_str() {
                if !text.trim().is_empty() {
                    metadata.insert(key.clone(), text.to_string());
                }
            }
        }
    }

    if let Some(ref backend) = request.backend {
        metadata.insert("backend".to_string(), backend.clone());
    }

    if let Some(ref model) = request.model {
        metadata.insert("model".to_string(), model.clone());
    }

    Err(RuntimeError::MissingTaskContext {
        message: "Insufficient task context. The request contains process/profile metadata but does not specify the coding objective.".to_string(),
        metadata,
        checked_fields: vec![
            "prompt".to_string(),
            "messages".to_string(),
            "metadata.task".to_string(),
            "metadata.spec".to_string(),
            "metadata.issue_ref".to_string(),
            "metadata.context".to_string(),
        ],
        recovery_sources: vec![
            "apxm_task_spec".to_string(),
            "parent_process_context".to_string(),
            "repository_or_issue_reference".to_string(),
            "prior_context_messages".to_string(),
        ],
        hydration_attempted: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_backends::{LLMRequest, Message, Role};

    #[test]
    fn accepts_non_empty_prompt() {
        let request = LLMRequest::new("Implement validation for missing task context");
        assert!(has_task_context(&request));
        assert!(validate_task_context(&request).is_ok());
    }

    #[test]
    fn accepts_structured_messages() {
        let request =
            LLMRequest::from_messages(vec![Message::text(Role::User, "Fix the bug in parser.rs")]);
        assert!(has_task_context(&request));
        assert!(validate_task_context(&request).is_ok());
    }

    #[test]
    fn accepts_task_in_metadata() {
        let mut request = LLMRequest::new("");
        request.metadata.insert(
            "task".to_string(),
            serde_json::Value::String("Add model router validation".to_string()),
        );
        assert!(has_task_context(&request));
        assert!(validate_task_context(&request).is_ok());
    }

    #[test]
    fn accepts_issue_ref_in_metadata() {
        let mut request = LLMRequest::new("");
        request.metadata.insert(
            "issue_ref".to_string(),
            serde_json::Value::String("APXM-123".to_string()),
        );
        assert!(has_task_context(&request));
        assert!(validate_task_context(&request).is_ok());
    }

    #[test]
    fn rejects_metadata_only_request() {
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

        assert!(!has_task_context(&request));
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
            _ => panic!("Expected MissingTaskContext error"),
        }
    }

    #[test]
    fn ignores_whitespace_only_prompt() {
        let request = LLMRequest::new("   \n\t  ");
        assert!(!has_task_context(&request));
        assert!(validate_task_context(&request).is_err());
    }

    #[test]
    fn ignores_whitespace_only_metadata_values() {
        let mut request = LLMRequest::new("");
        request.metadata.insert(
            "task".to_string(),
            serde_json::Value::String("   ".to_string()),
        );
        request.metadata.insert(
            "spec".to_string(),
            serde_json::Value::String("\n\t".to_string()),
        );
        assert!(!has_task_context(&request));
    }
}
