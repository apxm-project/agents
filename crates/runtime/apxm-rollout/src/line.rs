//! Wire schema for the per-thread rollout JSONL files.
//!
//! Every line in `rollout-<thread_id>.jsonl` deserializes to a [`RolloutLine`].
//! The Rust definitions here are the authoritative encoding the writer/reader
//! pair agree on. Bump [`crate::SCHEMA_VERSION`] when this changes.

use apxm_core::events::{EventSource, SkillEventProvenance};
use serde::{Deserialize, Serialize};

/// One physical line in a rollout JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutLine {
    pub meta: RolloutMeta,
    pub payload: RolloutPayload,
}

/// Per-line metadata: the apxm event envelope plus rollout-tree extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutMeta {
    pub seq: u64,
    pub timestamp: String,
    pub trace_id: String,
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub source: EventSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<SkillEventProvenance>,

    // Rollout-tree extensions (Claude Code parent_uuid + Codex thread fields).
    pub uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_uuid: Option<String>,
    pub session_id: String,
    pub thread_id: String,
    pub is_sidechain: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

/// Tagged union of line payload types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RolloutPayload {
    SessionMeta(Box<SessionMetaPayload>),
    TurnContext(TurnContextPayload),
    UserMessage(UserMessagePayload),
    AssistantMessage(AssistantMessagePayload),
    ToolUse(ToolUsePayload),
    ToolResult(ToolResultPayload),
    Compacted(CompactedPayload),
    Event(EventMsgPayload),
    Spilled(SpilledPayload),
}

/// First line of every rollout file — pins reproducibility metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetaPayload {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    pub session_id: String,
    pub started_at: String,
    pub cwd: String,
    pub apxm_version: String,
    pub agent_role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_code: Option<String>,
    pub skill_id: String,
    pub skill_version: String,
    pub artifact_hash: String,
    pub source_hash: String,
    pub air_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id_in_parent: Option<String>,
}

/// Per-turn baseline (sandbox/approval policy snapshot — Codex pattern).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnContextPayload {
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

/// Anthropic-compatible content block: text / thinking / tool_use / tool_result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Vec<ContentBlock>,
        #[serde(default)]
        is_error: bool,
    },
}

/// Per-line cache_creation / cache_read split. Matches Anthropic's per-turn shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<CacheCreationBreakdown>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CacheCreationBreakdown {
    #[serde(default)]
    pub ephemeral_1h_input_tokens: u64,
    #[serde(default)]
    pub ephemeral_5m_input_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessagePayload {
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessagePayload {
    pub content: Vec<ContentBlock>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUsePayload {
    pub tool_use_id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultPayload {
    pub tool_use_id: String,
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactedPayload {
    pub message: String,
    pub replacement_history: Vec<Box<RolloutPayload>>,
}

/// Wraps the on-wire shape of an [`apxm_core::events::ApxmEvent`].
///
/// Stored as a serde JSON Value so the rollout reader stays decoupled from
/// the event payload registry — readers that don't know an event kind can
/// still walk the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMsgPayload {
    pub event_kind: String,
    pub event: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpilledPayload {
    pub blob_ref: String,
    pub bytes_estimate: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_kind: Option<String>,
}
