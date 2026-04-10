//! Convenient error builders for common error patterns.
//!
//! This module provides centralized error construction helpers to eliminate
//! duplication across the codebase. All builders create `Error` instances
//! with appropriate error codes and default spans.

use crate::error::{Error, ErrorCode};

/// Builder for errors without source location information.
pub struct ErrorBuilder;

impl ErrorBuilder {
    /// Create a generic error with unknown span.
    pub fn generic(code: ErrorCode, message: impl Into<String>) -> Error {
        Error::new_generic(code, message)
    }

    /// Create an internal error with unknown span.
    pub fn internal(message: impl Into<String>) -> Error {
        Self::generic(ErrorCode::InternalError, message)
    }

    /// Create a parse error with unknown span.
    pub fn parse(message: impl Into<String>) -> Error {
        Self::generic(ErrorCode::SyntaxError, message)
    }

    /// Create a verification error with unknown span.
    pub fn verification(message: impl Into<String>) -> Error {
        Error::new_generic(ErrorCode::MLIRVerificationFailed, message)
    }

    /// Create a pass execution error with unknown span.
    pub fn pass_execution(message: impl Into<String>) -> Error {
        Error::new_generic(ErrorCode::PassExecutionFailed, message)
    }

    /// Create a pass manager error with unknown span.
    pub fn pass_manager(code: ErrorCode, message: impl Into<String>) -> Error {
        Error::new_generic(code, message)
    }

    /// Create a serialization error with unknown span.
    pub fn serialization(message: impl Into<String>) -> Error {
        Error::new_generic(ErrorCode::InternalError, message)
    }

    /// Create a context creation error with unknown span.
    pub fn context_creation(message: impl Into<String>) -> Error {
        Error::new_generic(ErrorCode::InternalError, message)
    }

    /// Create a context operation error with unknown span.
    pub fn context_operation(message: impl Into<String>) -> Error {
        Error::new_generic(ErrorCode::InternalError, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generic_builder() {
        let err = ErrorBuilder::generic(ErrorCode::TypeMismatch, "Type error");
        assert_eq!(err.code, ErrorCode::TypeMismatch);
        assert_eq!(err.message, "Type error");
        assert_eq!(err.primary_span.file, "<unknown>");
    }

    #[test]
    fn test_context_operation_builder() {
        let err = ErrorBuilder::context_operation("Context operation failed");
        assert_eq!(err.code, ErrorCode::InternalError);
        assert_eq!(err.message, "Context operation failed");
    }
}
