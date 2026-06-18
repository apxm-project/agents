//! Runtime errors.
//!
//! This module defines errors that occur during execution of the program,
//! including scheduler errors, operation executiong failures, and system errors.

use std::time::Duration;

use thiserror::Error;

use crate::error::common::OpId;
use crate::error::security::SecurityError;
use crate::types::AISOperationType;

/// Errors that occur during runtime execution.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Scheduler error: general.
    #[error("Scheduler error: {message}")]
    Scheduler {
        /// Error message describing the scheduler failure.
        message: String,
    },

    /// Scheduler error: missing token.
    #[error("Missing token {token_id} for node {node_id}")]
    SchedulerMissingToken {
        /// Node ID that needs the token.
        node_id: u64,
        /// Token ID that is missing.
        token_id: u64,
    },

    /// Scheduler error: duplicate producer.
    #[error("Duplicate producer for token {token_id}")]
    SchedulerDuplicateProducer {
        /// Token ID with duplicate producer.
        token_id: u64,
    },

    /// Scheduler error: deadlock detected.
    #[error("Deadlock detected after {timeout_ms}ms with {remaining} nodes remaining")]
    SchedulerDeadlock {
        /// Timeout in milliseconds.
        timeout_ms: u64,
        /// Number of remaining nodes.
        remaining: usize,
    },

    /// Scheduler error: execution cancelled.
    #[error("Execution cancelled")]
    SchedulerCancelled,

    /// Scheduler error: retry exhausted.
    #[error("Node {node_id} failed after retries: {reason}")]
    SchedulerRetryExhausted {
        /// Node ID that failed.
        node_id: u64,
        /// Reason for failure.
        reason: String,
    },

    /// Operation execution error.
    #[error("Operation execution failed: {op_type} - {message}")]
    Operation {
        /// Type of operation that failed.
        op_type: AISOperationType,
        /// Error message describing the operation failure.
        message: String,
    },

    /// Not an error: a handler signalling it has PARKED on an external event
    /// (e.g. PAUSE awaiting human input). The scheduler intercepts this, yields
    /// the worker lane + concurrency permit, and re-injects the node when the
    /// event arrives via the park registry (`wait_key`). Never surfaced to users.
    #[error("Operation parked awaiting event: {wait_key}")]
    OperationParked {
        /// Correlation key the external waker uses to resume this node.
        wait_key: String,
    },

    /// Capability invocation error.
    #[error("Capability error: {capability} - {message}")]
    Capability {
        /// Name of the capability that failed.
        capability: String,
        /// Error message describing the capability failure.
        message: String,
    },

    /// LLM backend error
    #[error(
        "LLM error{backend}: {message}",
        backend = Self::backend_suffix(.backend)
    )]
    LLM {
        /// Error message describing the LLM failure.
        message: String,
        /// Optional backend identifier.
        backend: Option<String>,
    },

    /// Memory system error.
    #[error(
        "Memory error{space}: {message}",
        space = Self::memory_space_suffix(.space)
    )]
    Memory {
        /// Error message describing the memory failure.
        message: String,
        /// Optional memory space identifier.
        space: Option<String>,
    },

    /// Security error (wraps SecurityError).
    #[error("Security error: {0}")]
    Security(#[from] SecurityError),

    /// Timeout error.
    #[error("Timeout: operation {op_id} exceeded timeout {timeout:?}")]
    Timeout {
        /// Operation ID that timed out.
        op_id: OpId,
        /// Time duration that was exceeded.
        timeout: Duration,
    },

    /// Serialization/Deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Executor error.
    #[error("Executor error: {0}")]
    Executor(String),

    /// State error.
    #[error("State error: {0}")]
    State(String),

    /// Invalid task error (task payload rejected at validation boundary).
    #[error("Invalid task: {reason}")]
    InvalidTask {
        /// Reason why the task is invalid.
        reason: String,
    },
}

impl RuntimeError {
    fn backend_suffix(backend: &Option<String>) -> String {
        match backend {
            Some(b) => format!(" (backend: {})", b),
            None => String::new(),
        }
    }

    fn memory_space_suffix(space: &Option<String>) -> String {
        match space {
            Some(s) => format!(" (space: {})", s),
            None => String::new(),
        }
    }

    /// Serialize this error into a JSON `Value` for the catch-branch input slot.
    pub fn to_value(&self) -> serde_json::Value {
        let (kind, message, details) = match self {
            RuntimeError::Scheduler { message } => {
                ("scheduler", message.clone(), serde_json::Value::Null)
            }
            RuntimeError::SchedulerMissingToken { node_id, token_id } => (
                "scheduler_missing_token",
                format!("Missing token {} for node {}", token_id, node_id),
                serde_json::json!({ "node_id": node_id, "token_id": token_id }),
            ),
            RuntimeError::SchedulerDuplicateProducer { token_id } => (
                "scheduler_duplicate_producer",
                format!("Duplicate producer for token {}", token_id),
                serde_json::json!({ "token_id": token_id }),
            ),
            RuntimeError::SchedulerDeadlock {
                timeout_ms,
                remaining,
            } => (
                "scheduler_deadlock",
                format!(
                    "Deadlock detected after {}ms with {} nodes remaining",
                    timeout_ms, remaining
                ),
                serde_json::json!({ "timeout_ms": timeout_ms, "remaining": remaining }),
            ),
            RuntimeError::SchedulerCancelled => (
                "scheduler_cancelled",
                "Execution cancelled".to_string(),
                serde_json::Value::Null,
            ),
            RuntimeError::SchedulerRetryExhausted { node_id, reason } => (
                "scheduler_retry_exhausted",
                format!("Node {} failed after retries: {}", node_id, reason),
                serde_json::json!({ "node_id": node_id, "reason": reason }),
            ),
            RuntimeError::Operation { op_type, message } => (
                "operation",
                message.clone(),
                serde_json::json!({ "op_type": format!("{}", op_type) }),
            ),
            RuntimeError::OperationParked { wait_key } => (
                "operation_parked",
                format!("parked awaiting event: {}", wait_key),
                serde_json::json!({ "wait_key": wait_key }),
            ),
            RuntimeError::Capability {
                capability,
                message,
            } => (
                "capability",
                message.clone(),
                serde_json::json!({ "capability": capability }),
            ),
            RuntimeError::LLM { message, backend } => (
                "llm",
                message.clone(),
                serde_json::json!({ "backend": backend }),
            ),
            RuntimeError::Memory { message, space } => (
                "memory",
                message.clone(),
                serde_json::json!({ "space": space }),
            ),
            RuntimeError::Security(sec) => {
                ("security", format!("{}", sec), serde_json::Value::Null)
            }
            RuntimeError::Timeout { op_id, timeout } => (
                "timeout",
                format!("Operation {:?} exceeded timeout {:?}", op_id, timeout),
                serde_json::json!({ "timeout_ms": timeout.as_millis() as u64 }),
            ),
            RuntimeError::Serialization(msg) => {
                ("serialization", msg.clone(), serde_json::Value::Null)
            }
            RuntimeError::Executor(msg) => ("executor", msg.clone(), serde_json::Value::Null),
            RuntimeError::State(msg) => ("state", msg.clone(), serde_json::Value::Null),
            RuntimeError::InvalidTask { reason } => {
                ("invalid_task", reason.clone(), serde_json::Value::Null)
            }
        };
        serde_json::json!({
            "kind": kind,
            "message": message,
            "details": details,
        })
    }

    /// Reconstruct a `RuntimeError` from a JSON `Value` produced by [`to_value`].
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        let kind = value.get("kind")?.as_str()?;
        let message = value.get("message")?.as_str()?.to_string();
        Some(match kind {
            "scheduler" => RuntimeError::Scheduler { message },
            "scheduler_cancelled" => RuntimeError::SchedulerCancelled,
            "operation" => RuntimeError::Operation {
                op_type: AISOperationType::Nop,
                message,
            },
            "capability" => RuntimeError::Capability {
                capability: value
                    .get("details")
                    .and_then(|d| d.get("capability"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string(),
                message,
            },
            "llm" => RuntimeError::LLM {
                message,
                backend: value
                    .get("details")
                    .and_then(|d| d.get("backend"))
                    .and_then(|b| b.as_str())
                    .map(|s| s.to_string()),
            },
            "memory" => RuntimeError::Memory {
                message,
                space: value
                    .get("details")
                    .and_then(|d| d.get("space"))
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string()),
            },
            "serialization" => RuntimeError::Serialization(message),
            "executor" => RuntimeError::Executor(message),
            "state" => RuntimeError::State(message),
            "invalid_task" => RuntimeError::InvalidTask { reason: message },
            _ => RuntimeError::Executor(format!("Unknown error kind '{}': {}", kind, message)),
        })
    }
}
