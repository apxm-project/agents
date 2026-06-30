//! Operation Validation
//!
//! Provides shared validation logic for compiler and runtime to ensure
//! operations have all required fields and correct types.

use crate::attrs;
use crate::operations::{AISOperationType, get_operation_spec};
use crate::types::Value;
use std::collections::HashMap;
use thiserror::Error;

/// Validation error types.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// A required field is missing.
    #[error("Missing required field '{field}' for operation {operation}")]
    MissingField {
        operation: String,
        field: &'static str,
    },

    /// A field has an invalid type.
    #[error(
        "Invalid type for field '{field}' in operation {operation}: expected {expected}, got {actual}"
    )]
    InvalidFieldType {
        operation: String,
        field: String,
        expected: String,
        actual: String,
    },

    /// An unknown field was provided.
    #[error("Unknown field '{field}' for operation {operation}")]
    UnknownField { operation: String, field: String },

    /// Operation-specific validation failed.
    #[error("Validation failed for operation {operation}: {message}")]
    OperationSpecific { operation: String, message: String },
}

/// Validates an operation against its specification.
///
/// This function checks that all required fields are present. Unlike the
/// previous implementation, this function **fails loudly** if validation
/// cannot be performed (e.g., if the operation spec is missing).
///
/// # Arguments
///
/// * `op_type` - The operation type to validate
/// * `attributes` - The attributes/fields provided for the operation
///
/// # Returns
///
/// * `Ok(())` if validation passes
/// * `Err(ValidationError)` if validation fails
pub fn validate_operation(
    op_type: AISOperationType,
    attributes: &HashMap<String, Value>,
) -> Result<(), ValidationError> {
    let spec = get_operation_spec(op_type);

    // Check all required fields are present
    for field in spec.required_fields() {
        if !attributes.contains_key(field.name) {
            return Err(ValidationError::MissingField {
                operation: spec.name.to_string(),
                field: field.name,
            });
        }
    }

    // Operation-specific format validation
    validate_field_formats(op_type, spec.name, attributes)?;

    Ok(())
}

/// Validates an operation with strict unknown field checking.
///
/// This is a stricter version that also rejects unknown fields.
pub fn validate_operation_strict(
    op_type: AISOperationType,
    attributes: &HashMap<String, Value>,
) -> Result<(), ValidationError> {
    let spec = get_operation_spec(op_type);

    // Check all required fields are present
    for field in spec.required_fields() {
        if !attributes.contains_key(field.name) {
            return Err(ValidationError::MissingField {
                operation: spec.name.to_string(),
                field: field.name,
            });
        }
    }

    // Check for unknown fields
    for key in attributes.keys() {
        if spec.get_field(key).is_none() {
            return Err(ValidationError::UnknownField {
                operation: spec.name.to_string(),
                field: key.clone(),
            });
        }
    }

    // Operation-specific format validation
    validate_field_formats(op_type, spec.name, attributes)?;

    Ok(())
}

/// Check if an operation has all required fields.
///
/// Returns true if validation would pass, false otherwise.
/// Does not provide error details - use `validate_operation` for that.
pub fn has_required_fields(op_type: AISOperationType, attributes: &HashMap<String, Value>) -> bool {
    let spec = get_operation_spec(op_type);
    spec.required_fields()
        .all(|f| attributes.contains_key(f.name))
}

/// Get the list of missing required fields for an operation.
pub fn missing_required_fields(
    op_type: AISOperationType,
    attributes: &HashMap<String, Value>,
) -> Vec<&'static str> {
    let spec = get_operation_spec(op_type);
    spec.required_fields()
        .filter(|f| !attributes.contains_key(f.name))
        .map(|f| f.name)
        .collect()
}

/// Validates format constraints on specific fields (e.g. `python_handler_id`).
fn validate_field_formats(
    _op_type: AISOperationType,
    op_name: &str,
    attributes: &HashMap<String, Value>,
) -> Result<(), ValidationError> {
    if let Some(val) = attributes.get(attrs::PYTHON_HANDLER_ID) {
        match val.as_str() {
            Some(s) => {
                if !is_valid_python_handler_id(s) {
                    return Err(ValidationError::OperationSpecific {
                        operation: op_name.to_string(),
                        message: format!(
                            "python_handler_id must match sha256:<64 hex chars>, got: {s}"
                        ),
                    });
                }
            }
            None => {
                return Err(ValidationError::InvalidFieldType {
                    operation: op_name.to_string(),
                    field: attrs::PYTHON_HANDLER_ID.to_string(),
                    expected: "string".to_string(),
                    actual: format!("{val:?}"),
                });
            }
        }
    }
    Ok(())
}

/// Returns `true` if `s` matches `^sha256:[0-9a-f]{64}$`.
fn is_valid_python_handler_id(s: &str) -> bool {
    let Some(hex) = s.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
