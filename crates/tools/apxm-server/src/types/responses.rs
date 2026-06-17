use apxm_core::error::RuntimeError;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::types::errors::{ApiFaultCode, FaultClass, TypedError};

#[derive(Debug, Serialize)]
pub(crate) struct OkAck {
    pub(crate) ok: bool,
}

impl OkAck {
    pub(crate) fn new() -> Self {
        Self { ok: true }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct OkAckId {
    pub(crate) ok: bool,
    pub(crate) id: String,
}

impl OkAckId {
    pub(crate) fn new(id: impl Into<String>) -> Self {
        Self {
            ok: true,
            id: id.into(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct OkAckName {
    pub(crate) ok: bool,
    pub(crate) name: String,
}

impl OkAckName {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            ok: true,
            name: name.into(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct HealthResponse {
    pub(crate) status: &'static str,
    pub(crate) version: &'static str,
    pub(crate) uptime_secs: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ModelEntry {
    pub(crate) id: String,
    pub(crate) object: &'static str,
    pub(crate) created: u64,
    pub(crate) owned_by: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct ModelList {
    pub(crate) object: &'static str,
    pub(crate) data: Vec<ModelEntry>,
}

/// A single model id served by a backend (sanitized; no provider metadata).
#[derive(Debug, Serialize)]
pub(crate) struct BackendModelEntry {
    pub(crate) id: String,
}

/// One registered backend, projected to the non-secret fields a compile-time
/// registry needs. Deliberately omits `api_key`, `headers`, and `endpoint` so
/// the discovery endpoint never puts credentials on the wire.
#[derive(Debug, Serialize)]
pub(crate) struct BackendEntry {
    pub(crate) name: String,
    pub(crate) protocol: String,
    pub(crate) models: Vec<BackendModelEntry>,
}

/// Response body for `GET /v1/backends` — the live backend registry with the
/// model ids each backend serves, so a client (e.g. apxm-studio) can compile
/// graphs against the same backends the runtime will dispatch to.
#[derive(Debug, Serialize)]
pub(crate) struct BackendList {
    pub(crate) object: &'static str,
    pub(crate) data: Vec<BackendEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolEntry {
    pub(crate) name: String,
    pub(crate) description: String,
    #[serde(rename = "inputSchema")]
    pub(crate) input_schema: JsonValue,
}

#[derive(Debug, Serialize)]
pub(crate) struct CapabilityEntry {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters_schema: JsonValue,
    /// Read-only capabilities run unattended; write capabilities require an
    /// explicit per-execution grant (admit-list). Surfaced so clients can show
    /// a consent prompt for the writes a workflow performs.
    pub(crate) read_only: bool,
    /// Whether the capability needs a connected credential to call.
    pub(crate) requires_auth: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ServerInfo {
    pub(crate) name: &'static str,
    pub(crate) version: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolsCapability {
    #[serde(rename = "listChanged")]
    pub(crate) list_changed: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResourcesCapability {
    #[serde(rename = "listChanged")]
    pub(crate) list_changed: bool,
    pub(crate) subscribe: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpInitializeCapabilities {
    pub(crate) tools: ToolsCapability,
    pub(crate) resources: ResourcesCapability,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpInitializeResult {
    #[serde(rename = "protocolVersion")]
    pub(crate) protocol_version: &'static str,
    #[serde(rename = "serverInfo")]
    pub(crate) server_info: ServerInfo,
    pub(crate) capabilities: McpInitializeCapabilities,
}

/// Lifecycle state of an A2A task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum A2aTaskStatus {
    Submitted,
    Working,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Serialize)]
pub(crate) struct A2aTaskState {
    pub(crate) state: A2aTaskStatus,
}

impl A2aTaskState {
    pub(crate) fn new(state: A2aTaskStatus) -> Self {
        Self { state }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct A2aErrorBody {
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct A2aTaskFailure {
    pub(crate) id: String,
    pub(crate) status: A2aTaskState,
    pub(crate) error: A2aErrorBody,
}

impl A2aTaskFailure {
    pub(crate) fn new(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: A2aTaskState::new(A2aTaskStatus::Failed),
            error: A2aErrorBody {
                message: message.into(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum A2aTextPartKind {
    Text,
}

#[derive(Debug, Serialize)]
pub(crate) struct A2aTextPart {
    #[serde(rename = "type")]
    pub(crate) kind: A2aTextPartKind,
    pub(crate) text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum A2aMessageRole {
    Agent,
}

#[derive(Debug, Serialize)]
pub(crate) struct A2aAgentMessage {
    pub(crate) role: A2aMessageRole,
    pub(crate) parts: Vec<A2aTextPart>,
}

#[derive(Debug, Serialize)]
pub(crate) struct A2aResultMessage {
    pub(crate) message: A2aAgentMessage,
}

#[derive(Debug, Serialize)]
pub(crate) struct A2aTaskSuccess {
    pub(crate) id: String,
    pub(crate) status: A2aTaskState,
    pub(crate) result: A2aResultMessage,
}

impl A2aTaskSuccess {
    pub(crate) fn completed(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: A2aTaskState::new(A2aTaskStatus::Completed),
            result: A2aResultMessage {
                message: A2aAgentMessage {
                    role: A2aMessageRole::Agent,
                    parts: vec![A2aTextPart {
                        kind: A2aTextPartKind::Text,
                        text: text.into(),
                    }],
                },
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct CheckpointCreatedResponse {
    pub(crate) ok: bool,
    pub(crate) checkpoint_id: String,
    pub(crate) status: apxm_core::types::SessionStatus,
    pub(crate) resume_url: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct CheckpointResumedResponse {
    pub(crate) ok: bool,
    pub(crate) checkpoint_id: String,
    pub(crate) status: apxm_core::types::SessionStatus,
    pub(crate) human_input: Option<JsonValue>,
    pub(crate) resumed_at_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CheckpointWebhookPayload {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) checkpoint_id: String,
    pub(crate) message: String,
    pub(crate) review_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExecutionStats {
    pub(crate) executed_nodes: usize,
    pub(crate) failed_nodes: usize,
    pub(crate) duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LlmUsageSummary {
    pub(crate) input_tokens: usize,
    pub(crate) output_tokens: usize,
    pub(crate) total_requests: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct TaskCreatedResponse {
    pub(crate) ok: bool,
    pub(crate) id: String,
    pub(crate) queue: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct TaskListResponse {
    pub(crate) queue: String,
    pub(crate) count: usize,
    pub(crate) tasks: Vec<crate::tasks::QueuedTask>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TaskClaimResponse {
    pub(crate) task_id: String,
    pub(crate) queue: String,
    pub(crate) data: JsonValue,
    pub(crate) claim_token: Option<String>,
    pub(crate) expires_at_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct StreamUsage {
    pub(crate) input_tokens: usize,
    pub(crate) output_tokens: usize,
    pub(crate) total_tokens: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct SseEventMeta {
    pub(crate) seq: u64,
    pub(crate) timestamp_ms: u64,
    pub(crate) trace_id: String,
    pub(crate) source: &'static str,
}

pub(crate) mod stream_sse {
    pub(crate) const EVENT_APXM: &str = "apxm";
    pub(crate) const EVENT_ERROR: &str = "error";
    pub(crate) const SOURCE_BACKEND: &str = "backend";
    pub(crate) const EMPTY_JSON_OBJECT: &str = "{}";
    pub(crate) const TIMEOUT_MESSAGE_PREFIX: &str = "Stream timeout after ";
    pub(crate) const TIMEOUT_MESSAGE_SUFFIX: &str = "s";
}

pub(crate) mod stream_payload_kind {
    pub(crate) const TOKEN: &str = "token";
    pub(crate) const THOUGHT: &str = "thought";
    pub(crate) const TOOL_CALL: &str = "tool_call";
    pub(crate) const LLM_DONE: &str = "llm_done";
    pub(crate) const USAGE: &str = "usage";
}

pub(crate) mod stream_tool_phase {
    pub(crate) const START: &str = "start";
    pub(crate) const DELTA: &str = "delta";
}

#[derive(Debug, Serialize)]
pub(crate) struct StreamTokenPayload {
    pub(crate) kind: &'static str,
    pub(crate) text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct StreamToolCallPayload {
    pub(crate) kind: &'static str,
    pub(crate) tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) arguments_delta: Option<String>,
    pub(crate) phase: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct StreamLlmDonePayload {
    pub(crate) kind: &'static str,
    pub(crate) content: String,
    pub(crate) model: String,
    pub(crate) finish_reason: String,
    pub(crate) usage: StreamUsage,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) reasoning: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct StreamUsagePayload {
    pub(crate) kind: &'static str,
    #[serde(flatten)]
    pub(crate) usage: StreamUsage,
}

/// Typed fault envelope on SSE `error` event streams (spec 0002 US3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StreamErrorBody {
    pub(crate) class: FaultClass,
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_hint: Option<String>,
}

impl From<TypedError> for StreamErrorBody {
    fn from(error: TypedError) -> Self {
        Self {
            class: error.class,
            code: error.code,
            message: error.message,
            recovery_hint: error.recovery_hint,
        }
    }
}

impl StreamErrorBody {
    pub(crate) fn from_typed(error: TypedError) -> Self {
        error.into()
    }

    pub(crate) fn from_runtime(error: &RuntimeError) -> Self {
        TypedError::from_runtime(error).into()
    }

    pub(crate) fn server_message(message: impl Into<String>) -> Self {
        TypedError::server_fault(ApiFaultCode::RuntimeError, message, None).into()
    }

    pub(crate) fn timeout(message: impl Into<String>) -> Self {
        TypedError::server_fault(ApiFaultCode::Timeout, message, None).into()
    }
}

#[cfg(test)]
mod stream_error_tests {
    use super::*;

    #[test]
    fn server_message_is_server_fault_runtime_error() {
        let body = StreamErrorBody::server_message("backend failed");
        assert_eq!(body.class, FaultClass::ServerFault);
        assert_eq!(body.code, "runtime_error");
        assert_eq!(body.message, "backend failed");
    }

    #[test]
    fn runtime_invalid_task_maps_to_program_fault_on_stream() {
        let body = StreamErrorBody::from_runtime(&RuntimeError::InvalidTask {
            reason: "missing field".to_string(),
        });
        assert_eq!(body.class, FaultClass::ProgramFault);
        assert_eq!(body.code, "invalid_task");
    }

    #[test]
    fn typed_error_round_trips_through_stream_body() {
        let typed = TypedError::server_fault(ApiFaultCode::LlmError, "provider down", None);
        let body = StreamErrorBody::from_typed(typed.clone());
        assert_eq!(body.class, typed.class);
        assert_eq!(body.code, typed.code);
        assert_eq!(body.message, typed.message);
    }
}
