//! Contract tests for typed error envelopes.

mod contract;

use apxm_server::types::errors::{ApiFaultCode, FaultClass, TypedError};
use serde_json::Value;

fn to_json(error: &TypedError) -> Value {
    serde_json::to_value(error).expect("typed error serializes")
}

#[test]
fn program_fault_fixture_has_distinct_class_and_code() {
    let error = TypedError::program_fault(
        ApiFaultCode::InvalidTask,
        "task payload rejected",
        Some("Fix the task fields and resubmit.".to_string()),
    );
    let json = to_json(&error);

    contract::assert_typed_error_envelope(&json);
    contract::assert_fault_class(&json, "program_fault");
    contract::assert_fault_code(&json, "invalid_task");
    assert_eq!(error.http_status(), axum::http::StatusCode::BAD_REQUEST);
}

#[test]
fn server_fault_fixture_has_distinct_class_and_code() {
    let error = TypedError::server_fault(ApiFaultCode::InternalError, "unexpected failure", None);
    let json = to_json(&error);

    contract::assert_typed_error_envelope(&json);
    contract::assert_fault_class(&json, "server_fault");
    contract::assert_fault_code(&json, "internal_error");
    assert_eq!(
        error.http_status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[test]
fn labeled_fixtures_produce_distinct_fault_classes() {
    let program = TypedError::program_fault(ApiFaultCode::TurnCapExceeded, "turn 6 denied", None);
    let server = TypedError::server_fault(ApiFaultCode::LlmError, "provider down", None);

    assert_eq!(program.class, FaultClass::ProgramFault);
    assert_eq!(server.class, FaultClass::ServerFault);
    assert_ne!(to_json(&program)["class"], to_json(&server)["class"]);
}

#[test]
fn runtime_invalid_task_maps_to_program_fault() {
    use apxm_core::error::RuntimeError;

    let runtime = RuntimeError::InvalidTask {
        reason: "missing session_id".to_string(),
    };
    let typed = TypedError::from_runtime(&runtime);
    let json = to_json(&typed);

    contract::assert_fault_class(&json, "program_fault");
    contract::assert_fault_code(&json, "invalid_task");
}

#[test]
fn runtime_scheduler_error_maps_to_server_fault() {
    use apxm_core::error::RuntimeError;

    let runtime = RuntimeError::Scheduler {
        message: "worker pool exhausted".to_string(),
    };
    let typed = TypedError::from_runtime(&runtime);
    let json = to_json(&typed);

    contract::assert_fault_class(&json, "server_fault");
    contract::assert_fault_code(&json, "scheduler_error");
}
