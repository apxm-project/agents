use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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

#[derive(Debug, Serialize)]
pub(crate) struct StreamWarningPayload {
    pub(crate) kind: &'static str,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct StreamErrorBody {
    pub(crate) message: String,
}
