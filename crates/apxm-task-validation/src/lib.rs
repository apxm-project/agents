use thiserror::Error;

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
}

/// Validates a task description.
///
/// Rules:
/// - Reject empty string
/// - Reject whitespace-only input
/// - Reject the literal placeholder "INVALID TASK" (case-insensitive after trim)
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

    Ok(ValidTask {
        raw: trimmed.to_string(),
    })
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
}
