//! All core APXM event payload structs plus the shared payload trait.

use std::any::Any;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::kind::{self, EventKind};
use super::registry::{
    EventPayloadRegistry, decode_registered_payload, event_kind_for_payload, payload_without_kind,
};
use crate::types::consent::ApprovalResolution;
use crate::types::execution::NodeMetrics;
use crate::types::operations::AISOperationType;

/// Redaction policy used for prompt and node-output observability payloads.
pub const REDACTION_POLICY_SUMMARY_HASH: &str = "summary_hash_v1";
/// Hash prefix used in redacted observability payloads.
pub const REDACTION_HASH_PREFIX_BLAKE3: &str = "blake3:";
/// Media type used when redacting plain prompt text.
pub const REDACTED_CONTENT_TYPE_TEXT: &str = "text/plain";
/// Media type used when redacting JSON node outputs.
pub const REDACTED_CONTENT_TYPE_JSON: &str = "application/json";
const JSON_SUMMARY_NULL: &str = "null";
const JSON_SUMMARY_BOOL: &str = "boolean";
const JSON_SUMMARY_NUMBER: &str = "number";
const JSON_SUMMARY_STRING: &str = "string";
const JSON_SUMMARY_ARRAY: &str = "array";
const JSON_SUMMARY_OBJECT: &str = "object";

/// Parent trait for all event payloads.
pub trait EventPayload: Send + Sync + 'static {
    /// Typed kind metadata for this payload.
    fn event_kind(&self) -> EventKind;

    /// Serialize only the payload fields. The envelope adds the `"kind"` tag.
    fn to_json(&self) -> serde_json::Value;

    /// Downcasting hook for consumers that need concrete payload types.
    fn as_any(&self) -> &dyn Any;
}

impl dyn EventPayload {
    /// Attempt to downcast the payload to a concrete type.
    pub fn downcast_ref<T: EventPayload>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }
}

impl std::fmt::Debug for dyn EventPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventPayload")
            .field("kind", &self.event_kind())
            .field("json", &self.to_json())
            .finish()
    }
}

/// Implement [`EventPayload`] for a serializable struct and a typed kind.
#[macro_export]
macro_rules! impl_event_payload {
    ($ty:ty, $kind:expr) => {
        impl $crate::events::payload::EventPayload for $ty {
            fn event_kind(&self) -> $crate::events::EventKind {
                $kind
            }

            fn to_json(&self) -> serde_json::Value {
                serde_json::to_value(self).unwrap_or_default()
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
    };
}

/// Deserialize a serialized core payload back into a boxed trait object.
pub fn boxed_payload_from_json(
    kind_name: &str,
    payload_json: serde_json::Value,
) -> serde_json::Result<Box<dyn EventPayload>> {
    boxed_payload_from_json_with_registry(kind_name, payload_json, None)
}

/// Deserialize a serialized payload using core decoders, an optional extension
/// registry, the global extension registry, then opaque fallback.
pub fn boxed_payload_from_json_with_registry(
    kind_name: &str,
    payload_json: serde_json::Value,
    registry: Option<&EventPayloadRegistry>,
) -> serde_json::Result<Box<dyn EventPayload>> {
    let payload_json = payload_without_kind(payload_json);

    if let Some(payload) = boxed_core_payload_from_json(kind_name, payload_json.clone())? {
        return Ok(payload);
    }

    if let Some(payload) =
        registry.and_then(|registry| registry.decode(kind_name, payload_json.clone()))
    {
        return payload;
    }

    if let Some(payload) = decode_registered_payload(kind_name, payload_json.clone()) {
        return payload;
    }

    Ok(Box::new(UnknownEventPayload::from_json(
        kind_name,
        payload_json,
    )))
}

fn boxed_core_payload_from_json(
    kind_name: &str,
    payload_json: serde_json::Value,
) -> serde_json::Result<Option<Box<dyn EventPayload>>> {
    if kind::core_event_kind(kind_name).is_none() {
        return Ok(None);
    }

    macro_rules! boxed {
        ($ty:ty) => {
            Ok(Some(
                Box::new(serde_json::from_value::<$ty>(payload_json)?) as Box<dyn EventPayload>
            ))
        };
    }

    if kind_name == kind::TOKEN.name() {
        boxed!(TokenPayload)
    } else if kind_name == kind::THOUGHT.name() {
        boxed!(ThoughtPayload)
    } else if kind_name == kind::TOOL_CALL.name() {
        boxed!(ToolCallPayload)
    } else if kind_name == kind::LLM_DONE.name() {
        boxed!(LlmDonePayload)
    } else if kind_name == kind::LLM_STEP_COMPLETED.name() {
        boxed!(LlmStepCompletedPayload)
    } else if kind_name == kind::LLM_PROMPT.name() {
        boxed!(LlmPromptPayload)
    } else if kind_name == kind::USAGE.name() {
        boxed!(UsagePayload)
    } else if kind_name == kind::RETRY.name() {
        boxed!(RetryPayload)
    } else if kind_name == kind::WARNING.name() {
        boxed!(WarningPayload)
    } else if kind_name == kind::CITATION.name() {
        boxed!(CitationPayload)
    } else if kind_name == kind::PROVIDER_EVENT.name() {
        boxed!(ProviderEventPayload)
    } else if kind_name == kind::OPERATION_START.name() {
        boxed!(OperationStartPayload)
    } else if kind_name == kind::OPERATION_END.name() {
        boxed!(OperationEndPayload)
    } else if kind_name == kind::NODE_OUTPUT.name() {
        boxed!(NodeOutputPayload)
    } else if kind_name == kind::NODE_METRICS.name() {
        boxed!(NodeMetricsPayload)
    } else if kind_name == kind::TOOL_START.name() {
        boxed!(ToolStartPayload)
    } else if kind_name == kind::TOOL_END.name() {
        boxed!(ToolEndPayload)
    } else if kind_name == kind::PLAN_CREATED.name() {
        boxed!(PlanCreatedPayload)
    } else if kind_name == kind::PLAN_STEP_STARTED.name() {
        boxed!(PlanStepStartedPayload)
    } else if kind_name == kind::PLAN_STEP_COMPLETED.name() {
        boxed!(PlanStepCompletedPayload)
    } else if kind_name == kind::PLAN_WORKFLOW_EMITTED.name() {
        boxed!(PlanWorkflowEmittedPayload)
    } else if kind_name == kind::WORKFLOW_STARTED.name() {
        boxed!(WorkflowStartedPayload)
    } else if kind_name == kind::WORKFLOW_STEP_STARTED.name() {
        boxed!(WorkflowStepStartedPayload)
    } else if kind_name == kind::WORKFLOW_STEP_COMPLETED.name() {
        boxed!(WorkflowStepCompletedPayload)
    } else if kind_name == kind::WORKFLOW_FINISHED.name() {
        boxed!(WorkflowFinishedPayload)
    } else if kind_name == kind::EXECUTION_STARTED.name() {
        boxed!(ExecutionStartedPayload)
    } else if kind_name == kind::EXECUTE_COMPLETE.name() {
        boxed!(ExecuteCompletePayload)
    } else if kind_name == kind::MEMORY_READ.name() {
        boxed!(MemoryReadPayload)
    } else if kind_name == kind::MEMORY_WRITE.name() {
        boxed!(MemoryWritePayload)
    } else if kind_name == kind::CHECKPOINT_SAVED.name() {
        boxed!(CheckpointSavedPayload)
    } else if kind_name == kind::CHECKPOINT_RESTORED.name() {
        boxed!(CheckpointRestoredPayload)
    } else if kind_name == kind::SCHEDULER_DECISION.name() {
        boxed!(SchedulerDecisionPayload)
    } else if kind_name == kind::MODEL_ROUTE_DECISION.name() {
        boxed!(ModelRouteDecisionPayload)
    } else if kind_name == kind::AGENT_ROUTE_DECISION.name() {
        boxed!(AgentRouteDecisionPayload)
    } else if kind_name == kind::HEAD_OF_LINE_BLOCK.name() {
        boxed!(HeadOfLineBlockPayload)
    } else if kind_name == kind::GPU_UTILIZATION.name() {
        boxed!(GpuUtilizationPayload)
    } else if kind_name == kind::TOKEN_USAGE.name() {
        boxed!(TokenUsagePayload)
    } else if kind_name == kind::MEMOIZATION_HIT.name() {
        boxed!(MemoizationHitPayload)
    } else if kind_name == kind::ERROR.name() {
        boxed!(ErrorPayload)
    } else if kind_name == kind::CONTEXT_COMPACTED.name() {
        boxed!(ContextCompactedPayload)
    } else if kind_name == kind::MODEL_CONTEXT_METRICS.name() {
        boxed!(ModelContextMetricsPayload)
    } else if kind_name == kind::CAPABILITY_EFFECT_RECEIPT.name() {
        let receipt = serde_json::from_value::<CapabilityEffectReceiptPayload>(payload_json)?;
        receipt
            .validate()
            .map_err(<serde_json::Error as serde::de::Error>::custom)?;
        Ok(Some(Box::new(receipt)))
    } else if kind_name == kind::MODEL_REROUTED.name() {
        boxed!(ModelReroutedPayload)
    } else if kind_name == kind::CANCELLED.name() {
        boxed!(CancelledPayload)
    } else if kind_name == kind::LOOP_DETECTED.name() {
        boxed!(LoopDetectedPayload)
    } else if kind_name == kind::CONTEXT_WINDOW_WARNING.name() {
        boxed!(ContextWindowWarningPayload)
    } else if kind_name == kind::SESSION_START.name() {
        boxed!(SessionStartPayload)
    } else if kind_name == kind::SESSION_END.name() {
        boxed!(SessionEndPayload)
    } else if kind_name == kind::TURN_BOUNDARY.name() {
        boxed!(TurnBoundaryPayload)
    } else if kind_name == kind::AGENT_SPAWNED.name() {
        boxed!(AgentSpawnedPayload)
    } else if kind_name == kind::COMMUNICATE_DISPATCHED.name() {
        boxed!(CommunicateDispatchedPayload)
    } else if kind_name == kind::GRAPH_EDGE.name() {
        boxed!(GraphEdgePayload)
    } else if kind_name == kind::TURN_STARTED.name() {
        boxed!(TurnStartedPayload)
    } else if kind_name == kind::TURN_COMPLETE.name() {
        boxed!(TurnCompletePayload)
    } else if kind_name == kind::TURN_ABORTED.name() {
        boxed!(TurnAbortedPayload)
    } else if kind_name == kind::SUBAGENT_SPAWN_BEGIN.name() {
        boxed!(SubagentSpawnBeginPayload)
    } else if kind_name == kind::SUBAGENT_SPAWN_END.name() {
        boxed!(SubagentSpawnEndPayload)
    } else if kind_name == kind::SUBAGENT_LLM_CALL_BEGIN.name() {
        boxed!(SubagentLlmCallBeginPayload)
    } else if kind_name == kind::SUBAGENT_LLM_CALL_END.name() {
        boxed!(SubagentLlmCallEndPayload)
    } else if kind_name == kind::TOOL_CALL_BEGIN.name() {
        boxed!(ToolCallBeginPayload)
    } else if kind_name == kind::TOOL_CALL_END.name() {
        boxed!(ToolCallEndPayload)
    } else if kind_name == kind::SUBAGENT_DONE.name() {
        boxed!(SubagentDonePayload)
    } else if kind_name == kind::SUBAGENT_FAILED.name() {
        boxed!(SubagentFailedPayload)
    } else if kind_name == kind::AGENT_MESSAGE.name() {
        boxed!(AgentMessagePayload)
    } else if kind_name == kind::APPROVAL_REQUEST.name() {
        boxed!(ApprovalRequestPayload)
    } else if kind_name == kind::APPROVAL_RESOLVED.name() {
        boxed!(ApprovalResolvedPayload)
    } else {
        Err(<serde_json::Error as serde::de::Error>::custom(format!(
            "core event kind `{kind_name}` has no payload decoder"
        )))
    }
}

// ===========================================================================
// LLM Layer payload structs
// ===========================================================================

/// Opaque event payload used when an event kind is not registered locally.
#[derive(Debug, Clone, PartialEq)]
pub struct UnknownEventPayload {
    kind: EventKind,
    payload_json: serde_json::Value,
}

impl UnknownEventPayload {
    /// Build an opaque payload from wire JSON. The `kind` discriminator should
    /// already be deleted from `payload_json`.
    pub fn from_json(kind_name: &str, payload_json: serde_json::Value) -> Self {
        Self {
            kind: event_kind_for_payload(kind_name),
            payload_json,
        }
    }

    /// Event kind wire name.
    pub fn kind_name(&self) -> &'static str {
        self.kind.name()
    }

    /// Original payload fields, excluding the `kind` discriminator.
    pub fn payload_json(&self) -> &serde_json::Value {
        &self.payload_json
    }

    /// Consume this payload and return original payload fields.
    pub fn into_payload_json(self) -> serde_json::Value {
        self.payload_json
    }
}

impl EventPayload for UnknownEventPayload {
    fn event_kind(&self) -> EventKind {
        self.kind
    }

    fn to_json(&self) -> serde_json::Value {
        self.payload_json.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Metadata for content that was intentionally deleted from an event payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactedContent {
    /// Whether the original content was redacted.
    pub redacted: bool,
    /// Stable redaction policy identifier.
    pub policy: String,
    /// Content hash with algorithm prefix.
    pub hash: String,
    /// Original serialized byte length.
    pub size_bytes: usize,
    /// Original character length when the source was text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub char_count: Option<usize>,
    /// Media type of the original content.
    pub content_type: String,
    /// Non-sensitive shape summary.
    pub summary: String,
}

impl RedactedContent {
    /// Create redaction metadata for plain text without retaining the text.
    pub fn from_text(text: &str) -> Self {
        Self {
            redacted: true,
            policy: REDACTION_POLICY_SUMMARY_HASH.to_string(),
            hash: tagged_blake3(text.as_bytes()),
            size_bytes: text.len(),
            char_count: Some(text.chars().count()),
            content_type: REDACTED_CONTENT_TYPE_TEXT.to_string(),
            summary: format!("text(chars={})", text.chars().count()),
        }
    }

    /// Create redaction metadata for JSON without retaining the value.
    pub fn from_json(value: &serde_json::Value) -> Self {
        let bytes = serde_json::to_vec(value).unwrap_or_default();
        Self {
            redacted: true,
            policy: REDACTION_POLICY_SUMMARY_HASH.to_string(),
            hash: tagged_blake3(&bytes),
            size_bytes: bytes.len(),
            char_count: None,
            content_type: REDACTED_CONTENT_TYPE_JSON.to_string(),
            summary: json_shape_summary(value),
        }
    }
}

fn tagged_blake3(bytes: &[u8]) -> String {
    format!(
        "{REDACTION_HASH_PREFIX_BLAKE3}{}",
        blake3::hash(bytes).to_hex()
    )
}

fn json_shape_summary(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => JSON_SUMMARY_NULL.to_string(),
        serde_json::Value::Bool(_) => JSON_SUMMARY_BOOL.to_string(),
        serde_json::Value::Number(_) => JSON_SUMMARY_NUMBER.to_string(),
        serde_json::Value::String(text) => {
            format!("{JSON_SUMMARY_STRING}(chars={})", text.chars().count())
        }
        serde_json::Value::Array(items) => format!("{JSON_SUMMARY_ARRAY}(len={})", items.len()),
        serde_json::Value::Object(fields) => format!("{JSON_SUMMARY_OBJECT}(len={})", fields.len()),
    }
}

/// Complete identity for one physical model request inside a logical LLM operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GenerationIdentity {
    /// Identifier shared by every retry and tool-loop step in the operation.
    pub call_id: String,
    /// One-based outer retry number.
    pub attempt: usize,
    /// One-based physical model-request number across the whole operation.
    pub step_number: usize,
}

impl GenerationIdentity {
    pub fn new(call_id: impl Into<String>, attempt: usize, step_number: usize) -> Self {
        Self {
            call_id: call_id.into(),
            attempt,
            step_number,
        }
    }
}

/// Identity shared by every event and dispatch derived from one model tool call.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCallCorrelation {
    /// Physical generation that produced the tool call.
    pub generation: GenerationIdentity,
    /// Provider-assigned model tool-call identifier.
    pub tool_call_id: String,
}

impl ToolCallCorrelation {
    pub fn new(generation: GenerationIdentity, tool_call_id: impl Into<String>) -> Self {
        Self {
            generation,
            tool_call_id: tool_call_id.into(),
        }
    }
}

/// Closed status vocabulary for tool-call completion events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Ok,
    Error,
    ApprovalPending,
}

impl ToolCallStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::ApprovalPending => "approval_pending",
        }
    }
}

/// Closed risk vocabulary exposed by approval request events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRiskLevel {
    Low,
    Medium,
    High,
}

impl ApprovalRiskLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// A single streaming token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPayload {
    /// The token text fragment.
    pub text: String,
    /// Physical generation that emitted this token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationIdentity>,
}
impl_event_payload!(TokenPayload, kind::TOKEN);

/// An extended-thinking / chain-of-thought block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtPayload {
    /// The thought text.
    pub text: String,
    /// Optional short summary of the thought.
    pub summary: Option<String>,
    /// Physical generation that emitted this thought.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationIdentity>,
}
impl_event_payload!(ThoughtPayload, kind::THOUGHT);

/// The model requested a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallPayload {
    /// Provider-assigned tool call ID.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Tool arguments as a JSON value.
    pub arguments: serde_json::Value,
    /// Physical generation and provider tool-call ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_correlation: Option<ToolCallCorrelation>,
}
impl_event_payload!(ToolCallPayload, kind::TOOL_CALL);

/// The model finished generating a complete response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmDonePayload {
    /// The full content of the response.
    pub content: String,
    /// Which model produced this response.
    pub model: String,
    /// Why the model stopped generating.
    pub finish_reason: FinishReasonPayload,
    /// Token usage for this response.
    pub usage: UsagePayload,
    /// Tool calls embedded in the response, if any.
    pub tool_calls: Vec<ToolCallPayload>,
    /// Provider-specific response ID.
    pub response_id: Option<String>,
    /// Physical generation that produced the accepted response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationIdentity>,
}
impl_event_payload!(LlmDonePayload, kind::LLM_DONE);

/// Detailed token accounting for one model-call step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmStepUsagePayload {
    /// Prompt/input tokens consumed by this model call.
    pub input_tokens: usize,
    /// Completion/output tokens consumed by this model call.
    pub output_tokens: usize,
    /// Input tokens served from a provider-side cache, when reported.
    pub cached_input_tokens: usize,
    /// Output tokens consumed by a provider reasoning channel, when reported.
    pub reasoning_output_tokens: usize,
}

/// Wall-clock timing evidence for one model-call step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmStepPerformancePayload {
    /// Full model-call latency observed by the runtime.
    pub latency_ms: f64,
    /// Provider-reported or runtime-estimated prompt-prefill duration.
    pub prefill_ms: f64,
    /// Provider-reported decoding duration.
    pub decode_ms: f64,
}

/// One model-call step completed inside a larger agent turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmStepCompletedPayload {
    /// Graph node that issued the model call.
    pub node_id: u64,
    /// One-based call number within this node's current tool loop.
    pub step_number: usize,
    /// Model that produced this response.
    pub model: String,
    /// Why this model call stopped generating.
    pub finish_reason: FinishReasonPayload,
    /// Detailed token accounting for this call.
    pub usage: LlmStepUsagePayload,
    /// Per-call latency and provider timing evidence.
    pub performance: LlmStepPerformancePayload,
    /// Number of tool calls requested by this response.
    pub tool_call_count: usize,
    /// Physical generation completed by this event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationIdentity>,
}
impl_event_payload!(LlmStepCompletedPayload, kind::LLM_STEP_COMPLETED);

/// A redacted prompt sent to an LLM backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmPromptPayload {
    /// The graph node ID that issued the prompt.
    pub node_id: u64,
    /// Optional stable or human-readable node name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// Redacted prompt metadata.
    pub prompt: RedactedContent,
    /// Physical generation receiving this prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationIdentity>,
}
impl_event_payload!(LlmPromptPayload, kind::LLM_PROMPT);

/// Why the model stopped generating.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinishReasonPayload {
    /// The reason string (e.g. `"stop"`, `"tool_use"`, `"length"`).
    pub reason: String,
}

/// Token usage accounting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsagePayload {
    /// Number of tokens in the prompt / input.
    pub input_tokens: usize,
    /// Number of tokens in the completion / output.
    pub output_tokens: usize,
    /// Physical generation accounted by a standalone usage event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationIdentity>,
}
impl_event_payload!(UsagePayload, kind::USAGE);

/// An API call is being retried.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPayload {
    /// Which attempt number this is (1-based).
    pub attempt: u32,
    /// Why the retry is happening.
    pub reason: String,
    /// Seconds to wait before retrying.
    pub retry_after_secs: f64,
    /// Which backend is being retried, if applicable.
    pub backend: Option<String>,
}
impl_event_payload!(RetryPayload, kind::RETRY);

/// A non-fatal warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningPayload {
    /// Machine-readable warning code.
    pub code: String,
    /// Human-readable warning message.
    pub message: String,
}
impl_event_payload!(WarningPayload, kind::WARNING);

/// A single citation from the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    /// Source URL.
    pub url: Option<String>,
    /// Source title.
    pub title: Option<String>,
    /// Start character index in the response.
    pub start_index: Option<usize>,
    /// End character index in the response.
    pub end_index: Option<usize>,
}

/// Citations returned by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationPayload {
    /// List of citations.
    pub citations: Vec<Citation>,
}
impl_event_payload!(CitationPayload, kind::CITATION);

/// An opaque, provider-specific event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEventPayload {
    /// Provider name (e.g. `"openai"`, `"anthropic"`).
    pub provider: String,
    /// Provider-specific event type.
    pub event_type: String,
    /// Arbitrary provider-specific data.
    pub data: serde_json::Value,
}
impl_event_payload!(ProviderEventPayload, kind::PROVIDER_EVENT);

// ===========================================================================
// Runtime Layer payload structs
// ===========================================================================

/// An operation (graph node) started executing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationStartPayload {
    /// The graph node ID.
    pub node_id: u64,
    /// The operation type.
    pub op_type: AISOperationType,
    /// Op-specific context (target agent for COMMUNICATE, tool names +
    /// model for ASK, agent_code for SPAWN_AGENT). Optional so
    /// extension is backwards-compatible — consumers that don't know
    /// about a key simply ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}
impl_event_payload!(OperationStartPayload, kind::OPERATION_START);

/// An operation finished executing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationEndPayload {
    /// The graph node ID.
    pub node_id: u64,
    /// The operation type.
    pub op_type: AISOperationType,
    /// How long the operation took, in milliseconds.
    pub duration_ms: u64,
    /// Whether the operation succeeded.
    pub success: bool,
}
impl_event_payload!(OperationEndPayload, kind::OPERATION_END);

/// A graph node produced an output value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutputPayload {
    /// The graph node ID.
    pub node_id: u64,
    /// Optional stable or human-readable node name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// Redacted output metadata.
    pub output: RedactedContent,
}
impl_event_payload!(NodeOutputPayload, kind::NODE_OUTPUT);

/// Runtime metrics recorded for a graph node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetricsPayload {
    /// The graph node ID.
    pub node_id: u64,
    /// Optional stable or human-readable node name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// Provider-neutral runtime metrics for the node.
    pub metrics: NodeMetrics,
}
impl_event_payload!(NodeMetricsPayload, kind::NODE_METRICS);

/// A tool invocation started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStartPayload {
    /// Tool name.
    pub name: String,
    /// Tool arguments as key-value pairs.
    pub args: HashMap<String, serde_json::Value>,
    /// Model tool call that initiated this invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_correlation: Option<ToolCallCorrelation>,
}
impl_event_payload!(ToolStartPayload, kind::TOOL_START);

/// A tool invocation finished.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEndPayload {
    /// Tool name.
    pub name: String,
    /// The tool result as a JSON value.
    pub result: serde_json::Value,
    /// Model tool call that initiated this invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_correlation: Option<ToolCallCorrelation>,
}
impl_event_payload!(ToolEndPayload, kind::TOOL_END);

// ───────────────────────────────────────────────────────────────────
// Multi-agent / topology enrichment payloads.
//
// Emitted alongside OPERATION_START/OPERATION_END so observers can
// reconstruct an agent/tool dispatch tree without scraping op
// attributes. None of the three is a terminal event (no _delta partner,
// but they describe topology resolved mid-run, not the end of a
// turn/session/execution — see `EventKind::is_terminal` on each).
// ───────────────────────────────────────────────────────────────────

/// A new agent was registered/spawned by a SPAWN_AGENT op.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnedPayload {
    /// Graph node id of the SPAWN_AGENT op that produced this agent.
    pub node_id: u64,
    /// Stable agent code (agent_name attribute on the spawn node).
    pub agent_code: String,
    /// Execution id of the parent dispatch that spawned this agent.
    pub parent_execution_id: String,
    /// Optional ACP profile when the agent runs as a subprocess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Optional process id when the agent was registered with the
    /// runtime process table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<String>,
    /// Scope policy controlling the agent's visibility window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_policy: Option<String>,
}
impl_event_payload!(AgentSpawnedPayload, kind::AGENT_SPAWNED);

/// A COMMUNICATE op dispatched a message to a target agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicateDispatchedPayload {
    /// Graph node id of the COMMUNICATE op.
    pub node_id: u64,
    /// Target agent — name or URL (URLs are kept verbatim).
    pub target_agent: String,
    /// Wire protocol (`local` | `http` | `https` | `acp` | `broadcast`).
    pub protocol: String,
    /// Optional, truncated excerpt of the outgoing message. Producers
    /// should cap to ~240 chars so observers can render an inline
    /// preview without holding the full payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_excerpt: Option<String>,
}
impl_event_payload!(CommunicateDispatchedPayload, kind::COMMUNICATE_DISPATCHED);

/// A graph edge resolved at runtime. Lets observers build the topology
/// view incrementally instead of inferring it from op + parent_span_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdgePayload {
    /// Source graph node id.
    pub from_node_id: u64,
    /// Target graph node id.
    pub to_node_id: u64,
    /// Edge kind: `dispatch` (spawn → child), `tool_invocation`
    /// (agent → tool), `synthesis_feed` (child → parent collector).
    ///
    /// Named `edge_kind`, not `kind`, so it is structurally independent
    /// from the envelope's own `"kind"` discriminator
    /// (`ApxmEvent`'s `Serialize` impl unconditionally sets `"kind"` to
    /// `"graph_edge"` on the serialized JSON object) — a field literally
    /// named `kind` would be silently overwritten before it ever reached
    /// the wire.
    pub edge_kind: String,
}
impl_event_payload!(GraphEdgePayload, kind::GRAPH_EDGE);

/// A plan was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCreatedPayload {
    /// Plan identifier.
    pub plan_id: String,
    /// Number of steps in the plan.
    pub steps: usize,
}
impl_event_payload!(PlanCreatedPayload, kind::PLAN_CREATED);

/// A plan step started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStepStartedPayload {
    /// Plan identifier.
    pub plan_id: String,
    /// Zero-based step index.
    pub step_index: usize,
}
impl_event_payload!(PlanStepStartedPayload, kind::PLAN_STEP_STARTED);

/// A plan step completed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStepCompletedPayload {
    /// Plan identifier.
    pub plan_id: String,
    /// Zero-based step index.
    pub step_index: usize,
    /// Whether the step succeeded.
    pub success: bool,
}
impl_event_payload!(PlanStepCompletedPayload, kind::PLAN_STEP_COMPLETED);

/// An LLM-emitted plan included a structured task DAG that the runtime
/// is about to lower into AIR and splice into the live execution DAG.
///
/// Emitted from the PLAN handler after the planner LLM response is
/// parsed and before the inner-plan linker compiles the DAG. Lets
/// observers tell apart the "LLM produced free-text steps" path from
/// the "LLM produced an executable workflow" path, and captures the
/// workflow shape for trace analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanWorkflowEmittedPayload {
    /// Plan identifier (the outer PLAN node's id).
    pub plan_id: String,
    /// Model that emitted the task DAG.
    pub generating_model: String,
    /// Number of tasks (nodes) in the emitted DAG.
    pub node_count: usize,
    /// Task ids as declared by the LLM (preserves the order the model
    /// wrote them; the linker may renumber).
    pub task_ids: Vec<u64>,
    /// Maximum fan-out across the DAG — i.e. the largest set of tasks
    /// that share the same `depends_on` and can therefore run in
    /// parallel. 1 means a linear chain; >1 quantifies extracted
    /// parallelism.
    pub parallel_fanout_max: usize,
}
impl_event_payload!(PlanWorkflowEmittedPayload, kind::PLAN_WORKFLOW_EMITTED);

/// A `.apxmw` workflow session started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStartedPayload {
    /// Workflow name from the `.apxmw` file.
    pub workflow_name: String,
    /// Workflow-root session directory.
    pub session_dir: String,
    /// Number of declared workflow steps.
    pub step_count: usize,
}
impl_event_payload!(WorkflowStartedPayload, kind::WORKFLOW_STARTED);

/// A `.apxmw` workflow step started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStepStartedPayload {
    /// Workflow name from the `.apxmw` file.
    pub workflow_name: String,
    /// Workflow-root session directory.
    pub workflow_session_dir: String,
    /// Step id from the `.apxmw` graph list.
    pub step_id: String,
    /// Zero-based index in workflow declaration order.
    pub step_index: usize,
    /// Number of declared workflow steps.
    pub step_count: usize,
}
impl_event_payload!(WorkflowStepStartedPayload, kind::WORKFLOW_STEP_STARTED);

/// A `.apxmw` workflow step completed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStepCompletedPayload {
    /// Workflow name from the `.apxmw` file.
    pub workflow_name: String,
    /// Workflow-root session directory.
    pub workflow_session_dir: String,
    /// Step id from the `.apxmw` graph list.
    pub step_id: String,
    /// Zero-based index in workflow declaration order.
    pub step_index: usize,
    /// Step status (`success`, `failed`, or `skipped`).
    pub status: String,
    /// Whether the step succeeded.
    pub success: bool,
    /// Step wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Child graph/artifact/workflow session directory, when one was produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_dir: Option<String>,
    /// Safe error string for failed steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
impl_event_payload!(WorkflowStepCompletedPayload, kind::WORKFLOW_STEP_COMPLETED);

/// A `.apxmw` workflow session finished.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowFinishedPayload {
    /// Workflow name from the `.apxmw` file.
    pub workflow_name: String,
    /// Workflow-root session directory.
    pub session_dir: String,
    /// Workflow status (`success`, `partial_failure`, or `failed`).
    pub status: String,
    /// Whether the workflow succeeded.
    pub success: bool,
    /// Workflow wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Number of recorded step results.
    pub step_count: usize,
}
impl_event_payload!(WorkflowFinishedPayload, kind::WORKFLOW_FINISHED);

/// A server-owned execution started and can now be followed by execution id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionStartedPayload {
    pub execution_id: String,
    /// Positional graph inputs supplied by the caller.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Verbatim user/cue text when the caller provides one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_text: Option<String>,
}
impl_event_payload!(ExecutionStartedPayload, kind::EXECUTION_STARTED);

/// A server-owned execution completed with its serialized response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteCompletePayload {
    pub result: serde_json::Value,
}
impl_event_payload!(ExecuteCompletePayload, kind::EXECUTE_COMPLETE);

/// A memory read event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryReadPayload {
    /// Memory scope (e.g. `"short_term"`, `"long_term"`).
    pub scope: String,
    /// The key that was read.
    pub key: String,
}
impl_event_payload!(MemoryReadPayload, kind::MEMORY_READ);

/// A memory write event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWritePayload {
    /// Memory scope.
    pub scope: String,
    /// The key that was written.
    pub key: String,
}
impl_event_payload!(MemoryWritePayload, kind::MEMORY_WRITE);

/// A checkpoint was saved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointSavedPayload {
    /// Checkpoint identifier.
    pub checkpoint_id: String,
}
impl_event_payload!(CheckpointSavedPayload, kind::CHECKPOINT_SAVED);

/// A checkpoint was restored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRestoredPayload {
    /// Checkpoint identifier.
    pub checkpoint_id: String,
}
impl_event_payload!(CheckpointRestoredPayload, kind::CHECKPOINT_RESTORED);

/// A scheduler decision event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerDecisionPayload {
    /// The graph node affected.
    pub node_id: u64,
    /// How long to delay, in milliseconds.
    pub delay_ms: u64,
    /// Reason for the scheduling decision.
    pub reason: String,
}
impl_event_payload!(SchedulerDecisionPayload, kind::SCHEDULER_DECISION);

/// A model-routing candidate rejected before final selection. Kept here without creating a
/// dependency from `apxm-core` back onto the runtime crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouteRejectionPayload {
    /// Candidate model name.
    pub candidate: String,
    /// Candidate's backend.
    pub backend: String,
    /// Typed rejection reason spelling (e.g. `circuit_breaker_open`,
    /// `requirements_not_satisfied`, `not_best_ranked`) — always sourced
    /// from `ModelRouteRejectionReason::as_str()` at the emit call site.
    pub reason_kind: String,
    /// Human-readable detail.
    pub reason: String,
}

/// Routing metadata emitted by `ModelRouter::select`: the chosen backend/model,
/// why it was chosen,
/// and every candidate that was passed over with its own reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouteDecisionPayload {
    /// Chosen backend name.
    pub backend: String,
    /// Resolved model name, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Whether this decision was constrained by circuit-breaker state.
    pub was_failover: bool,
    /// Why `backend`/`model` were chosen.
    pub reason: String,
    /// Candidates considered and passed over before this backend was chosen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_candidates: Vec<ModelRouteRejectionPayload>,
}
impl_event_payload!(ModelRouteDecisionPayload, kind::MODEL_ROUTE_DECISION);

/// An agent-routing candidate rejected before final selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRouteRejectionPayload {
    /// Candidate profile name.
    pub profile: String,
    /// Required capabilities the candidate was missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_capabilities: Vec<String>,
    /// Human-readable rejection reason.
    pub reason: String,
}

/// An `AgentRouter::route_requests` decision, made observable: the
/// chosen agent profile, why, and which candidates were rejected and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRouteDecisionPayload {
    /// Request id this decision answers.
    pub id: String,
    /// Chosen profile, if any (`None` means deterministic execution with no
    /// agent binding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Route source (`explicit` | `selected` | `deterministic`) — always
    /// sourced from `AgentRouteSource::as_str()`.
    pub source: String,
    /// Why this route was chosen.
    pub reason: String,
    /// Normalized required capabilities for this request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_capabilities: Vec<String>,
    /// Candidates rejected before this profile was chosen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_candidates: Vec<AgentRouteRejectionPayload>,
}
impl_event_payload!(AgentRouteDecisionPayload, kind::AGENT_ROUTE_DECISION);

/// A head-of-line blocking observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadOfLineBlockPayload {
    /// The running node likely causing the blockage.
    pub blocker_node: u64,
    /// The node that waited in the ready queue.
    pub blocked_node: u64,
    /// The observed wait time before dispatch, in milliseconds.
    pub wait_ms: u64,
    /// Human-readable explanation of the heuristic.
    pub reason: String,
}
impl_event_payload!(HeadOfLineBlockPayload, kind::HEAD_OF_LINE_BLOCK);

/// GPU utilization snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuUtilizationPayload {
    /// GPU device ID.
    pub gpu_id: u32,
    /// Compute utilization (0.0 - 100.0).
    pub utilization_pct: f32,
    /// Memory utilization (0.0 - 100.0).
    pub memory_pct: f32,
}
impl_event_payload!(GpuUtilizationPayload, kind::GPU_UTILIZATION);

/// Per-node token usage from the runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsagePayload {
    /// The graph node that consumed tokens.
    pub node_id: u64,
    /// Input tokens consumed.
    pub input_tokens: usize,
    /// Output tokens generated.
    pub output_tokens: usize,
    /// Physical model generation accounted by this usage event, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationIdentity>,
}
impl_event_payload!(TokenUsagePayload, kind::TOKEN_USAGE);

/// A memoization cache hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoizationHitPayload {
    /// The graph node whose result was memoized.
    pub node_id: u64,
}
impl_event_payload!(MemoizationHitPayload, kind::MEMOIZATION_HIT);

/// A runtime error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    /// Error message.
    pub message: String,
    /// Optional status/error code.
    pub status: Option<String>,
    /// Whether the error is recoverable.
    pub recoverable: bool,
}
impl_event_payload!(ErrorPayload, kind::ERROR);

// ===========================================================================
// Session Layer payload structs
// ===========================================================================

/// The context window was compacted (messages summarized/dropped).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCompactedPayload {
    /// Token count before compaction.
    pub original_tokens: usize,
    /// Token count after compaction.
    pub new_tokens: usize,
}
impl_event_payload!(ContextCompactedPayload, kind::CONTEXT_COMPACTED);

/// Invocation category for aggregate model-context metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelContextCallKind {
    /// A regular graph-node model invocation.
    Node,
    /// A model invocation that continues a tool interaction.
    ToolContinuation,
    /// A warmup model invocation.
    Warmup,
    /// A model invocation used to compact context.
    Compaction,
    /// A model invocation made by a hook.
    Hook,
}

/// Whether model-context aggregate metrics derive from a context plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelContextPlanStatus {
    /// Metrics were derived from a newly assembled context plan.
    Assembled,
    /// The invocation inherits an existing context plan.
    Inherited,
    /// The invocation has no context plan to aggregate.
    Unplanned,
}

/// Aggregate-only context packing metrics for a model invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelContextMetricsPayload {
    /// Graph node associated with the invocation, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<u64>,
    /// Invocation category.
    pub call_kind: ModelContextCallKind,
    /// Whether the optional aggregates derive from a context plan.
    pub plan_status: ModelContextPlanStatus,
    /// Maximum tokens available to the context plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    /// Tokens considered before context-plan admission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_tokens: Option<u64>,
    /// Tokens admitted to the rendered context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admitted_tokens: Option<u64>,
    /// Segments retained without truncation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_segments: Option<u64>,
    /// Segments retained in truncated form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated_segments: Option<u64>,
    /// Segments omitted because they exceeded the token budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omitted_token_budget_segments: Option<u64>,
    /// Empty segments omitted from the assembled context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omitted_empty_segments: Option<u64>,
}
impl_event_payload!(ModelContextMetricsPayload, kind::MODEL_CONTEXT_METRICS);

/// Capability dispatch surface that committed an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffectDispatchPath {
    /// A graph `INV_CAP` operation dispatched the capability.
    InvCap,
    /// An `ASK` model tool loop dispatched the capability.
    AskTool,
}

/// Capability implementation family that produced the effect evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffectImplementationKind {
    /// A runtime-native capability implementation.
    Native,
    /// A Python artifact handler.
    Python,
    /// A TypeScript artifact handler.
    Typescript,
    /// An authenticated host-dispatched implementation.
    Host,
}

/// Authority path that admitted the capability invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffectAdmissionKind {
    /// The capability is declared read-only.
    ReadOnly,
    /// The capability ran in a compatible sandbox.
    Sandbox,
    /// A runtime-minted capability grant admitted the invocation.
    Grant,
}

/// Trusted proof that an effect can be replay-checked without its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffectIdempotencyProof {
    /// The remote owner deduplicated this idempotency key.
    RemoteDeduplicated,
    /// A transaction boundary verified the committed effect.
    TransactionVerified,
}

/// Approval evidence recorded for an effect receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffectApprovalStatus {
    /// The capability did not require approval.
    NotRequired,
    /// A durable approval record admitted the invocation.
    Approved,
}

/// Immutable receipt status for a verified capability effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffectReceiptStatus {
    /// The effect and its trusted evidence are durably committed.
    Committed,
}

/// Content-free evidence for a durably committed capability effect.
///
/// The receipt deliberately carries identifiers, digests, and opaque
/// references only. It never serializes arguments, results, prompts,
/// credentials, request headers, paths, URLs, scopes, subjects, signatures,
/// or other raw content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityEffectReceiptPayload {
    /// Runtime-generated durable receipt identifier.
    pub receipt_id: String,
    /// Runtime execution that owns the receipt.
    pub execution_id: String,
    /// Graph node that dispatched the capability.
    pub node_id: u64,
    /// Runtime invocation identifier unique within the execution.
    pub invocation_id: String,
    /// Capability binding admitted for execution.
    pub capability_binding: String,
    /// Dispatch surface that invoked the capability.
    pub dispatch_path: CapabilityEffectDispatchPath,
    /// Implementation family that supplied verified evidence.
    pub implementation_kind: CapabilityEffectImplementationKind,
    /// Immutable implementation reference or fingerprint.
    pub implementation_ref: String,
    /// Digest of the canonical capability request.
    pub request_digest: String,
    /// Authority path that admitted this invocation.
    pub admission_kind: CapabilityEffectAdmissionKind,
    /// Selected runtime grant when admission used one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    /// Approval evidence when the capability required explicit approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_status: Option<CapabilityEffectApprovalStatus>,
    /// Durable approval record identifier for an approved invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    /// Trusted idempotency proof returned by the implementation owner.
    pub idempotency_proof: CapabilityEffectIdempotencyProof,
    /// Digest of the idempotency key used for the effect.
    pub idempotency_key_digest: String,
    /// Opaque external or transactional reference to the committed effect.
    pub effect_ref: String,
    /// The receipt is emitted only after durable commitment.
    pub status: CapabilityEffectReceiptStatus,
}

impl CapabilityEffectReceiptPayload {
    /// Validate the public receipt constraints after typed JSON decoding.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("receipt_id", self.receipt_id.as_str()),
            ("execution_id", self.execution_id.as_str()),
            ("invocation_id", self.invocation_id.as_str()),
            ("capability_binding", self.capability_binding.as_str()),
            ("implementation_ref", self.implementation_ref.as_str()),
            ("request_digest", self.request_digest.as_str()),
            (
                "idempotency_key_digest",
                self.idempotency_key_digest.as_str(),
            ),
            ("effect_ref", self.effect_ref.as_str()),
        ] {
            if value.is_empty() {
                return Err(format!(
                    "capability_effect_receipt.{field} must not be empty"
                ));
            }
        }

        for (field, value) in [
            ("grant_id", self.grant_id.as_deref()),
            ("approval_id", self.approval_id.as_deref()),
        ] {
            if value.is_some_and(str::is_empty) {
                return Err(format!(
                    "capability_effect_receipt.{field} must not be empty"
                ));
            }
        }

        if matches!(self.admission_kind, CapabilityEffectAdmissionKind::Grant)
            && self.grant_id.as_deref().map_or(true, str::is_empty)
        {
            return Err(
                "capability_effect_receipt.grant_id is required for grant admission".into(),
            );
        }
        if matches!(
            self.approval_status,
            Some(CapabilityEffectApprovalStatus::Approved)
        ) && self.approval_id.as_deref().map_or(true, str::is_empty)
        {
            return Err(
                "capability_effect_receipt.approval_id is required for approved authorization"
                    .into(),
            );
        }

        Ok(())
    }
}
impl_event_payload!(
    CapabilityEffectReceiptPayload,
    kind::CAPABILITY_EFFECT_RECEIPT
);

/// The model was rerouted to a different backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReroutedPayload {
    /// Originally requested model.
    pub original_model: String,
    /// The model that will actually be used.
    pub new_model: String,
    /// Why the reroute happened.
    pub reason: String,
}
impl_event_payload!(ModelReroutedPayload, kind::MODEL_REROUTED);

/// The request was cancelled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelledPayload {
    /// Optional reason for cancellation.
    pub reason: Option<String>,
}
impl_event_payload!(CancelledPayload, kind::CANCELLED);

/// A loop/cycle was detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDetectedPayload {
    /// Description of the detected pattern.
    pub pattern: String,
    /// How many iterations of the loop were observed.
    pub iterations: usize,
}
impl_event_payload!(LoopDetectedPayload, kind::LOOP_DETECTED);

/// The context window is approaching its limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWindowWarningPayload {
    /// Current token count.
    pub current_tokens: usize,
    /// Maximum token capacity.
    pub max_tokens: usize,
    /// Current utilization as a percentage (0.0 - 100.0).
    pub utilization_pct: f64,
}
impl_event_payload!(ContextWindowWarningPayload, kind::CONTEXT_WINDOW_WARNING);

/// A new session started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartPayload {
    /// Session identifier.
    pub session_id: String,
}
impl_event_payload!(SessionStartPayload, kind::SESSION_START);

/// A session ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEndPayload {
    /// Session identifier.
    pub session_id: String,
    /// Total number of turns in the session.
    pub total_turns: usize,
}
impl_event_payload!(SessionEndPayload, kind::SESSION_END);

/// Direction of a conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnDirection {
    /// User request.
    Request,
    /// Model response.
    Response,
}

/// A turn boundary was reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnBoundaryPayload {
    /// Turn number (1-based).
    pub turn_number: usize,
    /// Whether this is a request or response boundary.
    pub direction: TurnDirection,
}
impl_event_payload!(TurnBoundaryPayload, kind::TURN_BOUNDARY);

// ===========================================================================
// Layer 2 — agent-layer payload structs
//
// Emitted alongside the Layer 1 graph events whenever the executor is
// inside an agent scope. See `crates/runtime/engine/src/executor/
// agent_scope.rs` for the scope primitive and CLAUDE.md §10 for the
// canonical pairing rules. Field shapes follow the host app's dispatch
// event-kind payloads so the relay can stop translating.
// ===========================================================================

/// The outermost executor entry began — a user turn started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStartedPayload {
    /// APXM execution id for this turn.
    pub execution_id: String,
    /// Optional host-facing turn id (when one was supplied by the caller).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Optional human-readable label for the top-level agent (e.g. "Cleo").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator_label: Option<String>,
}
impl_event_payload!(TurnStartedPayload, kind::TURN_STARTED);

/// The outermost executor returned successfully.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCompletePayload {
    /// APXM execution id for the completed turn.
    pub execution_id: String,
    /// Wall-clock duration of the turn, in milliseconds.
    pub duration_ms: u64,
    /// Whether the turn produced a coordinator answer.
    pub had_answer: bool,
}
impl_event_payload!(TurnCompletePayload, kind::TURN_COMPLETE);

/// The outermost executor terminated abnormally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnAbortedPayload {
    /// APXM execution id for the aborted turn.
    pub execution_id: String,
    /// Wall-clock duration before abort, in milliseconds.
    pub duration_ms: u64,
    /// Coarse classification of the abort reason
    /// (`"cancelled"`, `"error"`, `"timeout"`, …).
    pub reason: String,
    /// Safe (PII-scrubbed) message about what happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message_safe: Option<String>,
}
impl_event_payload!(TurnAbortedPayload, kind::TURN_ABORTED);

/// A SPAWN_AGENT node is creating a new sub-agent execution scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpawnBeginPayload {
    /// Stable agent code (the `agent_name` attribute on the spawn node).
    pub agent_code: String,
    /// Optional human-readable agent name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// Optional agent type/category (e.g. `"module_agent"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Optional module key the agent belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_key: Option<String>,
    /// Optional autonomy policy controlling write-side behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy_policy: Option<String>,
    /// Span id of the parent agent scope (None for top-level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
}
impl_event_payload!(SubagentSpawnBeginPayload, kind::SUBAGENT_SPAWN_BEGIN);

/// The new sub-agent scope is fully constructed and ready to dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpawnEndPayload {
    /// Stable agent code matching the begin event.
    pub agent_code: String,
}
impl_event_payload!(SubagentSpawnEndPayload, kind::SUBAGENT_SPAWN_END);

/// An ASK node began an LLM call inside an agent scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentLlmCallBeginPayload {
    /// Stable code of the agent issuing the call.
    pub agent_code: String,
    /// Model identifier as routed (may differ from the requested model).
    pub model: String,
    /// Backend identifier (e.g. `"vllm"`, `"openai"`, `"ollama"`).
    pub backend: String,
    /// Count of tools exposed to the model for this call.
    pub tool_manifest_count: usize,
    /// Physical generation being dispatched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationIdentity>,
}
impl_event_payload!(SubagentLlmCallBeginPayload, kind::SUBAGENT_LLM_CALL_BEGIN);

/// An ASK node inside an agent scope returned an LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentLlmCallEndPayload {
    /// Stable code of the agent issuing the call.
    pub agent_code: String,
    /// Why the model stopped generating.
    pub finish_reason: String,
    /// Token accounting for this call.
    pub usage: UsagePayload,
    /// Length of the returned content, in characters (safe to surface).
    pub content_len: usize,
    /// Physical generation returned by this event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationIdentity>,
}
impl_event_payload!(SubagentLlmCallEndPayload, kind::SUBAGENT_LLM_CALL_END);

/// An INV_CAP node began a tool invocation inside an agent scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallBeginPayload {
    /// Stable code of the agent issuing the tool call.
    pub agent_code: String,
    /// Tool name being invoked.
    pub tool_name: String,
    /// Argument keys (no values — payload stays redaction-safe).
    pub argument_keys: Vec<String>,
    /// Model tool call that initiated this invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_correlation: Option<ToolCallCorrelation>,
}
impl_event_payload!(ToolCallBeginPayload, kind::TOOL_CALL_BEGIN);

/// An INV_CAP node inside an agent scope returned a result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEndPayload {
    /// Stable code of the agent issuing the tool call.
    pub agent_code: String,
    /// Tool name that ran.
    pub tool_name: String,
    /// Result keys (no values — safe-to-surface only).
    pub result_keys: Vec<String>,
    /// Coarse completion status.
    pub status: ToolCallStatus,
    /// Wall-clock duration of the tool call, in milliseconds.
    pub latency_ms: u64,
    /// Model tool call that initiated this invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_correlation: Option<ToolCallCorrelation>,
}
impl_event_payload!(ToolCallEndPayload, kind::TOOL_CALL_END);

/// A sub-agent scope exited cleanly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentDonePayload {
    /// Stable agent code that just exited.
    pub agent_code: String,
    /// Total number of tool calls dispatched during the scope.
    pub total_tool_calls: usize,
    /// Aggregated token usage across the scope.
    pub usage_total: UsagePayload,
    /// Optional short evidence excerpt for downstream rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_excerpt: Option<String>,
}
impl_event_payload!(SubagentDonePayload, kind::SUBAGENT_DONE);

/// A sub-agent scope exited with an error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentFailedPayload {
    /// Stable agent code that failed.
    pub agent_code: String,
    /// Coarse classification (`"timeout"`, `"capability_denied"`, …).
    pub error_class: String,
    /// Safe-to-surface error message (no PII / prompt fragments).
    pub error_message_safe: String,
}
impl_event_payload!(SubagentFailedPayload, kind::SUBAGENT_FAILED);

/// The coordinator emitted a final answer payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessagePayload {
    /// Coordinator-produced text.
    pub text: String,
    /// Optional stable item id (provider-assigned where available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    /// Optional provider response id this answer is part of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// Aggregated token usage for the coordinator's final turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsagePayload>,
}
impl_event_payload!(AgentMessagePayload, kind::AGENT_MESSAGE);

/// A human-in-the-loop approval gate was triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequestPayload {
    /// Agent that requested the action requiring approval.
    pub agent_code: String,
    /// Tool name being gated.
    pub tool_name: String,
    /// Stable approval id (host-issued where available).
    pub approval_id: String,
    /// Coarse risk classification.
    pub risk_level: ApprovalRiskLevel,
    /// Model tool call guarded by this approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_correlation: Option<ToolCallCorrelation>,
}
impl_event_payload!(ApprovalRequestPayload, kind::APPROVAL_REQUEST);

/// A previously-requested approval was resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResolvedPayload {
    /// Stable approval id this resolution refers to.
    pub approval_id: String,
    /// Closed resolution outcome.
    pub decision: ApprovalResolution,
    /// Model tool call guarded by this approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_correlation: Option<ToolCallCorrelation>,
}
impl_event_payload!(ApprovalResolvedPayload, kind::APPROVAL_RESOLVED);
