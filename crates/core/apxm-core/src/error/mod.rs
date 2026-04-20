//! Error types, codes, and helpers for APXM.

/// Error type with full context (like Rust compiler errors).
pub mod api;
/// Errors related to tooling interfaces.
pub mod cli;
/// Error codes for all APXM components.
pub mod codes;
/// Shared error helpers such as [`SourceLocation`] and [`ErrorContext`].
pub mod common;
/// Errors produced during compilation phases such as parsing or verification.
pub mod compile;
/// Errors related to the MLIR compiler infrastructure.
pub mod compiler;
/// Errors that can happen while executing operations at runtime.
pub mod runtime;
/// Policy, authorization, and rate-limiting errors.
pub mod security;
/// Source code spans for error reporting.
pub mod span;
/// Suggestions for fixing errors.
pub mod suggestion;

pub use api::Error;
pub use cli::{CliError, CliResult};
pub use codes::ErrorCode;
pub use common::{ErrorContext, OpId, SourceLocation, TraceId, chain_errors, format_error};
pub use compile::CompileError;
pub use compiler::CompilerError;
pub use runtime::RuntimeError;
pub use security::SecurityError;
pub use span::Span;
pub use suggestion::Suggestion;
