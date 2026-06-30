//! Compilation errors.
//!
//! This module defines errors that occur during the compilation phase,
//! including parsing, type checking, verification, and optimization errors.
//!
//! All errors wrap [`Error`] to provide detailed, contextual error reporting
//! with source spans, suggestions, and help text.

use thiserror::Error as ThisError;

use crate::error::Error;

/// Errors that occur during compilation.
#[derive(Debug, ThisError)]
pub enum CompileError {
    /// Parse error with rich context
    #[error("Parse error: {0}")]
    Parse(Box<Error>),

    /// Type error with rich context
    #[error("Type error: {0}")]
    Type(Box<Error>),

    /// Verification error with rich context
    #[error("Verification error: {0}")]
    Verification(Box<Error>),

    /// Optimization error with rich context
    #[error("Optimization error: {0}")]
    Optimization(Box<Error>),

    /// Pass execution failed with rich context
    #[error("Pass execution failed: {0}")]
    PassFailed(Box<Error>),

    /// DAG construction error with rich context
    #[error("DAG construction failed: {0}")]
    DagConstruction(Box<Error>),

    /// Module not found (simple error, no rich context needed)
    #[error("Module not found: {name}")]
    ModuleNotFound {
        /// Name of the module that was not found.
        name: String,
    },
}

impl CompileError {
    /// Get the underlying Error if available
    pub fn as_error(&self) -> Option<&Error> {
        match self {
            CompileError::ModuleNotFound { .. } => None,
            CompileError::Parse(e)
            | CompileError::Type(e)
            | CompileError::Verification(e)
            | CompileError::Optimization(e)
            | CompileError::PassFailed(e)
            | CompileError::DagConstruction(e) => Some(e),
        }
    }

    /// Pretty-print with source code
    pub fn pretty_print(&self, source: Option<&str>) -> String {
        if let Some(e) = self.as_error() {
            e.pretty_print(source)
        } else {
            format!("{}", self)
        }
    }
}
