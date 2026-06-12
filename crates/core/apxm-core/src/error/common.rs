//! Common error types and utilities
//!
//! This module provides shared types and utilities for error handling across APXM.

use std::collections::HashMap;
use std::fmt;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::types::{AISOperationType, Value};

/// Operations correspond to node in the execution DAG, so OpID is the same as NodeId.
pub type OpId = u64;

/// Use for distributed tracing and error correlation.
pub type TraceId = String;

/// Represents a location in source code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    /// File path or identifier,
    pub file: String,
    /// Line number (1-indexed).
    pub line: usize,
    /// Column number (1-indexed).
    pub column: usize,
}

impl SourceLocation {
    /// Creates a new source location.
    pub fn new(file: String, line: usize, column: usize) -> Self {
        SourceLocation { file, line, column }
    }

    pub fn unknown() -> Self {
        SourceLocation {
            file: "<unknown>".to_string(),
            line: 0,
            column: 0,
        }
    }
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// Error context for attaching additional information to errors.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorContext {
    /// Operation identifier where the error occurred.
    pub operation_id: Option<OpId>,
    /// Type of operation that failed.
    pub operation_type: Option<AISOperationType>,
    /// Trace identifier for distributed tracing.
    pub trace_id: Option<TraceId>,
    /// Timestamp when the error occurred.
    pub timestamp: SystemTime,
    /// Additional metadata as key-value pairs.
    pub metadata: HashMap<String, Value>,
}

impl ErrorContext {
    /// Creates a new error context with the current timestamp.
    pub fn new() -> Self {
        ErrorContext {
            operation_id: None,
            operation_type: None,
            trace_id: None,
            timestamp: SystemTime::now(),
            metadata: HashMap::new(),
        }
    }

    /// Sets the operation ID.
    pub fn with_operation_id(mut self, op_id: OpId) -> Self {
        self.operation_id = Some(op_id);
        self
    }

    /// Sets the operation type.
    pub fn with_operation_type(mut self, op_type: AISOperationType) -> Self {
        self.operation_type = Some(op_type);
        self
    }

    /// Sets the trace ID.
    pub fn with_trace_id(mut self, trace_id: TraceId) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Adds metadata.
    pub fn with_metadata(mut self, key: String, value: Value) -> Self {
        self.metadata.insert(key, value);
        self
    }
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Chains multiple errors into a formatted string.
pub fn chain_errors(errors: Vec<Box<dyn std::error::Error>>) -> String {
    if let Some(result) = errors
        .iter()
        .map(|e| e.to_string())
        .reduce(|acc, e| format!("{}\n  caused by: {}", acc, e))
    {
        result
    } else {
        String::from("No errors")
    }
}

/// Formats a single error with its context.
pub fn format_error(error: &dyn std::error::Error) -> String {
    std::iter::successors(Some(error), |e| e.source())
        .enumerate()
        .map(|(i, err)| match i {
            0 => err.to_string(),
            1 => format!("\n    caused by:\n    {}", err),
            _ => format!("\n    {}", err),
        })
        .collect()
}

