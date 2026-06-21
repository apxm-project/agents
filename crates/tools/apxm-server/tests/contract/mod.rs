//! Shared helpers for contract and integration tests.

use serde_json::Value;

/// Assert a JSON value matches the OpenAPI `TypedError` envelope
/// (`specs/0002-apxm-chat-thin-clients/contracts/openapi-session-v1.yaml`).
pub fn assert_typed_error_envelope(value: &Value) {
    let class = value
        .get("class")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("typed error missing class: {value}"));
    assert!(
        matches!(class, "program_fault" | "server_fault"),
        "unexpected fault class: {class}"
    );
    assert!(
        value.get("code").and_then(Value::as_str).is_some(),
        "typed error missing code: {value}"
    );
    assert!(
        value.get("message").and_then(Value::as_str).is_some(),
        "typed error missing message: {value}"
    );
}

/// Assert the wire fault class matches the expected snake_case label.
pub fn assert_fault_class(value: &Value, expected: &str) {
    assert_eq!(
        value.get("class").and_then(Value::as_str),
        Some(expected),
        "unexpected class in {value}"
    );
}

/// Assert the stable machine-readable code field.
pub fn assert_fault_code(value: &Value, expected: &str) {
    assert_eq!(
        value.get("code").and_then(Value::as_str),
        Some(expected),
        "unexpected code in {value}"
    );
}
