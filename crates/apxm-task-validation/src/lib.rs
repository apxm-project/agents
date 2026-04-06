use thiserror::Error;
use serde_json;

/// A validated task description that is safe to pass deeper into planning
/// or implementation pipelines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidTask {
    raw: String,
}

impl ValidTask {
    /// Returns the original normalized task text.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes the task and returns the owned string.
    pub fn into_string(self) -> String {
        self.raw
    }
}

/// Validation errors for inbound task descriptions.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TaskValidationError {
    #[error("task is empty")]
    Empty,

    #[error("task contains only whitespace")]
    WhitespaceOnly,

    #[error("task is an explicit invalid placeholder: {0}")]
    ExplicitInvalidPlaceholder(String),

    #[error("task contains only metadata without actionable requirements")]
    MetadataOnly,
}

/// Validates a task description.
///
/// Rules:
/// - Reject empty string
/// - Reject whitespace-only input
/// - Reject the literal placeholder "INVALID TASK" (case-insensitive after trim)
/// - Reject metadata-only JSON (contains only metadata fields without actionable requirements)
///
/// The validated task is normalized with leading/trailing whitespace removed.
pub fn validate_task(input: &str) -> Result<ValidTask, TaskValidationError> {
    if input.is_empty() {
        return Err(TaskValidationError::Empty);
    }

    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err(TaskValidationError::WhitespaceOnly);
    }

    if trimmed.eq_ignore_ascii_case("INVALID TASK") {
        return Err(TaskValidationError::ExplicitInvalidPlaceholder(
            trimmed.to_string(),
        ));
    }

    // Check if input is metadata-only JSON
    if is_metadata_only_json(trimmed) {
        return Err(TaskValidationError::MetadataOnly);
    }

    Ok(ValidTask {
        raw: trimmed.to_string(),
    })
}

/// Checks if the input is a JSON object containing only metadata fields
/// without actionable task requirements.
fn is_metadata_only_json(input: &str) -> bool {
    // Quick check: if it doesn't look like JSON, skip expensive parsing
    if !input.starts_with('{') {
        return false;
    }

    // Attempt to parse as JSON
    let Ok(value) = serde_json::from_str::<serde_json::Value>(input) else {
        return false;
    };

    // Must be an object
    let Some(obj) = value.as_object() else {
        return false;
    };

    // If empty object, consider it metadata-only
    if obj.is_empty() {
        return true;
    }

    // Known metadata-only fields that don't represent actionable tasks
    const METADATA_FIELDS: &[&str] = &[
        "profile",
        "process_id",
        "name",
        "spawned_by",
        "id",
        "session_id",
        "agent_id",
        "timestamp",
        "metadata",
    ];

    // If ALL fields are metadata fields, reject
    obj.keys().all(|k| METADATA_FIELDS.contains(&k.as_str()))
}

/// Indicates whether the task should be allowed to proceed into downstream
/// architecture/implementation logic.
pub fn is_actionable_task(input: &str) -> bool {
    validate_task(input).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_task() {
        let task = validate_task("Add timeout support to model router").unwrap();
        assert_eq!(task.as_str(), "Add timeout support to model router");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let task = validate_task("   Fix scheduler race condition   ").unwrap();
        assert_eq!(task.as_str(), "Fix scheduler race condition");
    }

    #[test]
    fn rejects_empty_input() {
        let err = validate_task("").unwrap_err();
        assert_eq!(err, TaskValidationError::Empty);
    }

    #[test]
    fn rejects_whitespace_only() {
        let err = validate_task("   \n\t  ").unwrap_err();
        assert_eq!(err, TaskValidationError::WhitespaceOnly);
    }

    #[test]
    fn rejects_invalid_task_literal() {
        let err = validate_task("INVALID TASK").unwrap_err();
        assert_eq!(
            err,
            TaskValidationError::ExplicitInvalidPlaceholder("INVALID TASK".to_string())
        );
    }

    #[test]
    fn rejects_invalid_task_literal_case_insensitive() {
        let err = validate_task("invalid task").unwrap_err();
        assert_eq!(
            err,
            TaskValidationError::ExplicitInvalidPlaceholder("invalid task".to_string())
        );
    }

    #[test]
    fn actionable_task_matches_validation() {
        assert!(is_actionable_task("Implement router fallback metrics"));
        assert!(!is_actionable_task(""));
        assert!(!is_actionable_task("   "));
        assert!(!is_actionable_task("INVALID TASK"));
    }

    #[test]
    fn rejects_metadata_only_json() {
        let input = r#"{"profile":"default","process_id":"123","name":"test"}"#;
        let err = validate_task(input).unwrap_err();
        assert_eq!(err, TaskValidationError::MetadataOnly);
    }

    #[test]
    fn rejects_empty_json_object() {
        let err = validate_task("{}").unwrap_err();
        assert_eq!(err, TaskValidationError::MetadataOnly);
    }

    #[test]
    fn accepts_json_with_task_field() {
        let input = r#"{"profile":"default","task":"Implement feature X"}"#;
        let task = validate_task(input).unwrap();
        assert!(task.as_str().contains("task"));
    }

    #[test]
    fn accepts_plain_text_task() {
        let task = validate_task("Add error handling to model router").unwrap();
        assert_eq!(task.as_str(), "Add error handling to model router");
    }

    #[test]
    fn rejects_metadata_with_spawned_by() {
        let input = r#"{"spawned_by":"agent1","session_id":"abc","agent_id":"xyz"}"#;
        let err = validate_task(input).unwrap_err();
        assert_eq!(err, TaskValidationError::MetadataOnly);
    }

    #[test]
    fn accepts_malformed_json_as_task() {
        // If it's not valid JSON, treat as task description
        let task = validate_task(r#"{"incomplete"#).unwrap();
        assert_eq!(task.as_str(), r#"{"incomplete"#);
    }
}
