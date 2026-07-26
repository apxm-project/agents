//! Operation Validation
//!
//! Provides shared validation logic for compiler and runtime to ensure
//! operations carry exactly the fields their closed operation spec declares.

use crate::operations::{AISOperationType, OperationSpec, get_operation_spec};
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
    let spec = require_spec(op_type)?;

    for field in spec.required_fields() {
        if !attributes.contains_key(field.name) {
            return Err(ValidationError::MissingField {
                operation: spec.name.to_string(),
                field: field.name,
            });
        }
    }

    Ok(())
}

/// Validates an operation with strict unknown field checking.
///
/// This is a stricter version that also rejects unknown fields.
pub fn validate_operation_strict(
    op_type: AISOperationType,
    attributes: &HashMap<String, Value>,
) -> Result<(), ValidationError> {
    let spec = require_spec(op_type)?;

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

    Ok(())
}

/// Check if an operation has all required fields.
///
/// Returns true if validation would pass, false otherwise.
/// Does not provide error details - use `validate_operation` for that.
pub fn has_required_fields(op_type: AISOperationType, attributes: &HashMap<String, Value>) -> bool {
    let Some(spec) = get_operation_spec(op_type) else {
        return false;
    };
    spec.required_fields()
        .all(|f| attributes.contains_key(f.name))
}

/// Get the list of missing required fields for an operation.
pub fn missing_required_fields(
    op_type: AISOperationType,
    attributes: &HashMap<String, Value>,
) -> Vec<&'static str> {
    let Some(spec) = get_operation_spec(op_type) else {
        return Vec::new();
    };
    spec.required_fields()
        .filter(|f| !attributes.contains_key(f.name))
        .map(|f| f.name)
        .collect()
}

fn require_spec(op_type: AISOperationType) -> Result<&'static OperationSpec, ValidationError> {
    get_operation_spec(op_type).ok_or_else(|| ValidationError::OperationSpecific {
        operation: op_type.to_string(),
        message: "unknown semantic operation".to_string(),
    })
}
