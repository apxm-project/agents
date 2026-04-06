use apxm_task_validation::{is_actionable_task, validate_task, TaskValidationError};

#[test]
fn integration_accepts_realistic_task() {
    let task =
        validate_task("Read crates/apxm-runtime/src/model_router and add a failing test first")
            .expect("expected valid task");
    assert_eq!(
        task.as_str(),
        "Read crates/apxm-runtime/src/model_router and add a failing test first"
    );
}

#[test]
fn integration_rejects_invalid_placeholder_with_whitespace() {
    let err = validate_task("   INVALID TASK   ").unwrap_err();
    assert_eq!(
        err,
        TaskValidationError::ExplicitInvalidPlaceholder("INVALID TASK".to_string())
    );
}

#[test]
fn integration_reports_actionability() {
    assert!(is_actionable_task("Add unit tests for Scheduler"));
    assert!(!is_actionable_task("INVALID TASK"));
}
