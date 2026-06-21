//! Machine-readable API fault envelopes.
//!
//! Distinguishes **program faults** (caller/program recoverable, 4xx) from
//! **server faults** (infrastructure/runtime, 5xx). Wire shape matches
//! `specs/0002-apxm-chat-thin-clients/contracts/openapi-session-v1.yaml`
//! `TypedError` schema.

use apxm_core::error::RuntimeError;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Fault class on the wire — program vs server responsibility boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FaultClass {
    /// Caller or program can adjust inputs and retry (4xx).
    ProgramFault,
    /// Server-side failure; not model-recoverable (5xx).
    ServerFault,
}

/// Stable machine-readable fault codes for HTTP and SSE error frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFaultCode {
    // ── Program faults (4xx) ────────────────────────────────────────
    BadRequest,
    NotFound,
    UnprocessableEntity,
    TooManyRequests,
    Conflict,
    TurnCapExceeded,
    ToolBudgetExceeded,
    CapabilityDenied,
    InvalidTask,
    AdmissionDenied,

    // ── Server faults (5xx) ─────────────────────────────────────────
    InternalError,
    RuntimeError,
    SchedulerError,
    LlmError,
    Timeout,
    StreamLag,
}

impl ApiFaultCode {
    /// Wire `code` string (stable for contract tests and generated client).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::NotFound => "not_found",
            Self::UnprocessableEntity => "unprocessable_entity",
            Self::TooManyRequests => "too_many_requests",
            Self::Conflict => "conflict",
            Self::TurnCapExceeded => "turn_cap_exceeded",
            Self::ToolBudgetExceeded => "tool_budget_exceeded",
            Self::CapabilityDenied => "capability_denied",
            Self::InvalidTask => "invalid_task",
            Self::AdmissionDenied => "admission_denied",
            Self::InternalError => "internal_error",
            Self::RuntimeError => "runtime_error",
            Self::SchedulerError => "scheduler_error",
            Self::LlmError => "llm_error",
            Self::Timeout => "timeout",
            Self::StreamLag => "stream_lag",
        }
    }

    pub fn fault_class(self) -> FaultClass {
        match self {
            Self::BadRequest
            | Self::NotFound
            | Self::UnprocessableEntity
            | Self::TooManyRequests
            | Self::Conflict
            | Self::TurnCapExceeded
            | Self::ToolBudgetExceeded
            | Self::CapabilityDenied
            | Self::InvalidTask
            | Self::AdmissionDenied => FaultClass::ProgramFault,
            Self::InternalError
            | Self::RuntimeError
            | Self::SchedulerError
            | Self::LlmError
            | Self::Timeout
            | Self::StreamLag => FaultClass::ServerFault,
        }
    }

    pub fn http_status(self) -> StatusCode {
        match self {
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::UnprocessableEntity => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::Conflict => StatusCode::CONFLICT,
            Self::TurnCapExceeded | Self::ToolBudgetExceeded | Self::CapabilityDenied => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            Self::InvalidTask | Self::AdmissionDenied => StatusCode::BAD_REQUEST,
            Self::InternalError
            | Self::RuntimeError
            | Self::SchedulerError
            | Self::LlmError
            | Self::Timeout
            | Self::StreamLag => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Typed error envelope returned on HTTP responses and SSE error frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TypedError {
    pub class: FaultClass,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_hint: Option<String>,
}

impl TypedError {
    pub fn new(
        code: ApiFaultCode,
        message: impl Into<String>,
        recovery_hint: Option<String>,
    ) -> Self {
        Self {
            class: code.fault_class(),
            code: code.as_str().to_string(),
            message: message.into(),
            recovery_hint,
        }
    }

    pub fn program_fault(
        code: ApiFaultCode,
        message: impl Into<String>,
        recovery_hint: Option<String>,
    ) -> Self {
        debug_assert_eq!(code.fault_class(), FaultClass::ProgramFault);
        Self::new(code, message, recovery_hint)
    }

    pub fn server_fault(
        code: ApiFaultCode,
        message: impl Into<String>,
        recovery_hint: Option<String>,
    ) -> Self {
        debug_assert_eq!(code.fault_class(), FaultClass::ServerFault);
        Self::new(code, message, recovery_hint)
    }

    pub fn http_status(&self) -> StatusCode {
        ApiFaultCode::from_wire(&self.code)
            .map(ApiFaultCode::http_status)
            .unwrap_or_else(|| {
                if self.class == FaultClass::ProgramFault {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            })
    }

    /// Map a runtime execution error to a typed fault used by `error.rs`.
    pub fn from_runtime(error: &RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidTask { reason } => Self::program_fault(
                ApiFaultCode::InvalidTask,
                reason.clone(),
                Some("Fix the task payload and resubmit.".to_string()),
            ),
            RuntimeError::SchedulerCancelled => {
                Self::program_fault(ApiFaultCode::Conflict, "execution cancelled", None)
            }
            RuntimeError::Scheduler { .. }
            | RuntimeError::SchedulerMissingToken { .. }
            | RuntimeError::SchedulerDuplicateProducer { .. }
            | RuntimeError::SchedulerDeadlock { .. }
            | RuntimeError::SchedulerRetryExhausted { .. } => {
                Self::server_fault(ApiFaultCode::SchedulerError, error.to_string(), None)
            }
            RuntimeError::Capability { .. } => Self::program_fault(
                ApiFaultCode::CapabilityDenied,
                error.to_string(),
                Some("Adjust capability grants or tool arguments.".to_string()),
            ),
            RuntimeError::LLM { .. } => {
                Self::server_fault(ApiFaultCode::LlmError, error.to_string(), None)
            }
            RuntimeError::Timeout { .. } => {
                Self::server_fault(ApiFaultCode::Timeout, error.to_string(), None)
            }
            RuntimeError::Security(_) => {
                Self::program_fault(ApiFaultCode::AdmissionDenied, error.to_string(), None)
            }
            _ => Self::server_fault(ApiFaultCode::RuntimeError, error.to_string(), None),
        }
    }
}

impl ApiFaultCode {
    pub(crate) fn from_wire(code: &str) -> Option<Self> {
        Some(match code {
            "bad_request" => Self::BadRequest,
            "not_found" => Self::NotFound,
            "unprocessable_entity" => Self::UnprocessableEntity,
            "too_many_requests" => Self::TooManyRequests,
            "conflict" => Self::Conflict,
            "turn_cap_exceeded" => Self::TurnCapExceeded,
            "tool_budget_exceeded" => Self::ToolBudgetExceeded,
            "capability_denied" => Self::CapabilityDenied,
            "invalid_task" => Self::InvalidTask,
            "admission_denied" => Self::AdmissionDenied,
            "internal_error" => Self::InternalError,
            "runtime_error" => Self::RuntimeError,
            "scheduler_error" => Self::SchedulerError,
            "llm_error" => Self::LlmError,
            "timeout" => Self::Timeout,
            "stream_lag" => Self::StreamLag,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_and_server_fault_classes_differ() {
        let program = TypedError::program_fault(ApiFaultCode::BadRequest, "bad input", None);
        let server = TypedError::server_fault(ApiFaultCode::InternalError, "boom", None);
        assert_eq!(program.class, FaultClass::ProgramFault);
        assert_eq!(server.class, FaultClass::ServerFault);
        assert_ne!(program.class, server.class);
    }
}
