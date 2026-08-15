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
        backend = Self::backend_suffix(.backend.as_ref())
    )]
    LLM {
        /// Error message describing the LLM failure.
        message: String,
        /// Optional backend identifier.
        backend: Option<String>,
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
    fn backend_suffix(backend: Option<&String>) -> String {
        match backend {
            Some(b) => format!(" (backend: {})", b),
            None => String::new(),
        }
    }

    /// The serialized `kind` discriminant this error records itself under.
    ///
    /// This is the single place a variant's wire name is written, and the
    /// match is exhaustive with no wildcard arm: a variant added to
    /// `RuntimeError` does not compile until it names the kind it records.
    pub fn kind(&self) -> &'static str {
        match self {
            RuntimeError::Scheduler { .. } => "scheduler",
            RuntimeError::SchedulerMissingToken { .. } => "scheduler_missing_token",
            RuntimeError::SchedulerDuplicateProducer { .. } => "scheduler_duplicate_producer",
            RuntimeError::SchedulerDeadlock { .. } => "scheduler_deadlock",
            RuntimeError::SchedulerCancelled => "scheduler_cancelled",
            RuntimeError::SchedulerRetryExhausted { .. } => "scheduler_retry_exhausted",
            RuntimeError::Operation { .. } => "operation",
            RuntimeError::OperationParked { .. } => "operation_parked",
            RuntimeError::Capability { .. } => "capability",
            RuntimeError::LLM { .. } => "llm",
            RuntimeError::Security(_) => "security",
            RuntimeError::Timeout { .. } => "timeout",
            RuntimeError::Serialization(_) => "serialization",
            RuntimeError::Executor(_) => "executor",
            RuntimeError::State(_) => "state",
            RuntimeError::InvalidTask { .. } => "invalid_task",
        }
    }

    /// Serialize this error into a JSON `Value` for the catch-branch input slot.
    pub fn to_value(&self) -> serde_json::Value {
        let (message, details) = match self {
            RuntimeError::Scheduler { message } => (message.clone(), serde_json::Value::Null),
            RuntimeError::SchedulerMissingToken { node_id, token_id } => (
                format!("Missing token {} for node {}", token_id, node_id),
                serde_json::json!({ "node_id": node_id, "token_id": token_id }),
            ),
            RuntimeError::SchedulerDuplicateProducer { token_id } => (
                format!("Duplicate producer for token {}", token_id),
                serde_json::json!({ "token_id": token_id }),
            ),
            RuntimeError::SchedulerDeadlock {
                timeout_ms,
                remaining,
            } => (
                format!(
                    "Deadlock detected after {}ms with {} nodes remaining",
                    timeout_ms, remaining
                ),
                serde_json::json!({ "timeout_ms": timeout_ms, "remaining": remaining }),
            ),
            RuntimeError::SchedulerCancelled => {
                ("Execution cancelled".to_string(), serde_json::Value::Null)
            }
            RuntimeError::SchedulerRetryExhausted { node_id, reason } => (
                format!("Node {} failed after retries: {}", node_id, reason),
                serde_json::json!({ "node_id": node_id, "reason": reason }),
            ),
            RuntimeError::Operation { op_type, message } => (
                message.clone(),
                serde_json::json!({ "op_type": format!("{}", op_type) }),
            ),
            RuntimeError::OperationParked { wait_key } => (
                format!("parked awaiting event: {}", wait_key),
                serde_json::json!({ "wait_key": wait_key }),
            ),
            RuntimeError::Capability {
                capability,
                message,
            } => (
                message.clone(),
                serde_json::json!({ "capability": capability }),
            ),
            RuntimeError::LLM { message, backend } => {
                (message.clone(), serde_json::json!({ "backend": backend }))
            }
            RuntimeError::Security(sec) => (format!("{}", sec), serde_json::Value::Null),
            RuntimeError::Timeout { op_id, timeout } => (
                format!("Operation {:?} exceeded timeout {:?}", op_id, timeout),
                serde_json::json!({ "timeout_ms": timeout.as_millis() as u64 }),
            ),
            RuntimeError::Serialization(msg)
            | RuntimeError::Executor(msg)
            | RuntimeError::State(msg) => (msg.clone(), serde_json::Value::Null),
            RuntimeError::InvalidTask { reason } => (reason.clone(), serde_json::Value::Null),
        };
        serde_json::json!({
            "kind": self.kind(),
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
                op_type: AISOperationType::ModelCall,
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
            "serialization" => RuntimeError::Serialization(message),
            "executor" => RuntimeError::Executor(message),
            "state" => RuntimeError::State(message),
            "invalid_task" => RuntimeError::InvalidTask { reason: message },
            _ => RuntimeError::Executor(format!("Unknown error kind '{}': {}", kind, message)),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::RuntimeError;
    use crate::error::security::SecurityError;
    use crate::types::AISOperationType;

    /// One representative value of every `RuntimeError` variant.
    ///
    /// `variant_name` below is the proof of completeness: its match has no
    /// wildcard arm, so a variant added to `RuntimeError` does not compile until
    /// it is named there, and this list is only accepted once every named arm is
    /// reached by a representative. A hand-written count could see neither.
    fn every_runtime_error_variant() -> Vec<RuntimeError> {
        let variants = representatives();
        let reached: std::collections::BTreeSet<&'static str> =
            variants.iter().map(variant_name).collect();
        assert_eq!(
            reached.len(),
            variants.len(),
            "two representatives are the same variant, so the scan does not reach them all"
        );
        assert_eq!(
            reached,
            EVERY_VARIANT_NAME.iter().copied().collect(),
            "the representative list does not cover every RuntimeError variant"
        );
        variants
    }

    /// Every `RuntimeError` variant name, in declaration order.
    ///
    /// Paired with the wildcard-free match in `variant_name`: a new variant
    /// breaks that match, and adding its arm without adding it here fails the
    /// coverage assertion above.
    const EVERY_VARIANT_NAME: &[&str] = &[
        "Scheduler",
        "SchedulerMissingToken",
        "SchedulerDuplicateProducer",
        "SchedulerDeadlock",
        "SchedulerCancelled",
        "SchedulerRetryExhausted",
        "Operation",
        "OperationParked",
        "Capability",
        "LLM",
        "Security",
        "Timeout",
        "Serialization",
        "Executor",
        "State",
        "InvalidTask",
    ];

    fn variant_name(error: &RuntimeError) -> &'static str {
        match error {
            RuntimeError::Scheduler { .. } => "Scheduler",
            RuntimeError::SchedulerMissingToken { .. } => "SchedulerMissingToken",
            RuntimeError::SchedulerDuplicateProducer { .. } => "SchedulerDuplicateProducer",
            RuntimeError::SchedulerDeadlock { .. } => "SchedulerDeadlock",
            RuntimeError::SchedulerCancelled => "SchedulerCancelled",
            RuntimeError::SchedulerRetryExhausted { .. } => "SchedulerRetryExhausted",
            RuntimeError::Operation { .. } => "Operation",
            RuntimeError::OperationParked { .. } => "OperationParked",
            RuntimeError::Capability { .. } => "Capability",
            RuntimeError::LLM { .. } => "LLM",
            RuntimeError::Security(_) => "Security",
            RuntimeError::Timeout { .. } => "Timeout",
            RuntimeError::Serialization(_) => "Serialization",
            RuntimeError::Executor(_) => "Executor",
            RuntimeError::State(_) => "State",
            RuntimeError::InvalidTask { .. } => "InvalidTask",
        }
    }

    fn representatives() -> Vec<RuntimeError> {
        vec![
            RuntimeError::Scheduler {
                message: "m".to_string(),
            },
            RuntimeError::SchedulerMissingToken {
                node_id: 1,
                token_id: 2,
            },
            RuntimeError::SchedulerDuplicateProducer { token_id: 2 },
            RuntimeError::SchedulerDeadlock {
                timeout_ms: 1,
                remaining: 1,
            },
            RuntimeError::SchedulerCancelled,
            RuntimeError::SchedulerRetryExhausted {
                node_id: 1,
                reason: "m".to_string(),
            },
            RuntimeError::Operation {
                op_type: AISOperationType::ModelCall,
                message: "m".to_string(),
            },
            RuntimeError::OperationParked {
                wait_key: "k".to_string(),
            },
            RuntimeError::Capability {
                capability: "c".to_string(),
                message: "m".to_string(),
            },
            RuntimeError::LLM {
                message: "m".to_string(),
                backend: None,
            },
            RuntimeError::Security(SecurityError::SandboxError {
                message: "m".to_string(),
            }),
            RuntimeError::Timeout {
                op_id: 1,
                timeout: Duration::from_secs(1),
            },
            RuntimeError::Serialization("m".to_string()),
            RuntimeError::Executor("m".to_string()),
            RuntimeError::State("m".to_string()),
            RuntimeError::InvalidTask {
                reason: "m".to_string(),
            },
        ]
    }

    /// No runtime error reports a failed selection among candidates.
    ///
    /// A `model.call` target resolves to exactly one bound Model Deployment or
    /// fails closed, and a Capability dispatches through exactly one graph
    /// operation. There is no candidate set, so no failure mode is "the
    /// selection step found nothing eligible" — that error can only be raised
    /// by a router, and a typed error naming one asserts on the wire that the
    /// execution plane ranks candidates.
    ///
    /// The exhaustive `RuntimeError::kind` match protects the wire vocabulary;
    /// this explicit count protects the representative list on stable Rust.
    #[test]
    fn no_runtime_error_kind_reports_a_failed_selection() {
        let variants = every_runtime_error_variant();
        let kinds: Vec<&'static str> = variants.iter().map(RuntimeError::kind).collect();
        let distinct: std::collections::BTreeSet<&'static str> = kinds.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            kinds.len(),
            "two variants share a wire kind, so the scan does not reach them both: {kinds:?}"
        );

        let offenders: Vec<&&str> = kinds
            .iter()
            .filter(|kind| kind.contains("route") || kind.contains("candidate"))
            .collect();
        assert!(
            offenders.is_empty(),
            "runtime error kinds report a failed selection: {offenders:?}"
        );
    }

    /// The serialized envelope records the same kind the variant declares, so
    /// scanning `RuntimeError::kind` scans what is actually written to
    /// evidence.
    #[test]
    fn the_serialized_kind_is_the_kind_the_variant_declares() {
        for error in every_runtime_error_variant() {
            assert_eq!(
                error.to_value()["kind"].as_str(),
                Some(error.kind()),
                "serialized kind drifted from the declared kind for {error}"
            );
        }
    }

    /// A retired error kind decodes into the unknown-kind arm rather than a
    /// typed routing failure.
    ///
    /// This is the one-way reduction, not a legacy reader: the retired name is
    /// carried through in the message instead of being translated onto a
    /// canonical routing variant, and the record is not dropped, so an
    /// operator reading persisted evidence sees exactly what was written.
    #[test]
    fn a_retired_routing_error_decodes_as_unknown_rather_than_a_route_failure() {
        let persisted = serde_json::json!({
            "kind": "no_route_found",
            "message": "No route found for target 'topic:receivables': none eligible",
            "details": { "target": "topic:receivables" },
        });
        let decoded =
            RuntimeError::from_value(&persisted).expect("persisted history still decodes");
        let message = decoded.to_string();
        assert!(
            message.contains("Unknown error kind 'no_route_found'"),
            "the retired kind must decode as unknown, not as a typed route failure: {message}"
        );
        assert!(
            message.contains("topic:receivables"),
            "the persisted message must survive verbatim: {message}"
        );
    }
}
