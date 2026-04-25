//! All core APXM event payload structs plus the shared payload trait.

use std::any::Any;
use std::collections::HashMap;
use std::io;

use serde::{Deserialize, Serialize};

use super::kind::{self, EventKind};
use crate::types::operations::AISOperationType;

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
    mut payload_json: serde_json::Value,
) -> serde_json::Result<Box<dyn EventPayload>> {
    if let serde_json::Value::Object(obj) = &mut payload_json {
        obj.remove("kind");
    }

    macro_rules! boxed {
        ($ty:ty) => {
            Ok(Box::new(serde_json::from_value::<$ty>(payload_json)?) as Box<dyn EventPayload>)
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
    } else {
        Err(serde_json::Error::io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown event kind: {kind_name}"),
        )))
    }
}

// ===========================================================================
// LLM Layer payload structs
// ===========================================================================

/// A single streaming token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPayload {
    /// The token text fragment.
    pub text: String,
}
impl_event_payload!(TokenPayload, kind::TOKEN);

/// An extended-thinking / chain-of-thought block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtPayload {
    /// The thought text.
    pub text: String,
    /// Optional short summary of the thought.
    pub summary: Option<String>,
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
}
impl_event_payload!(LlmDonePayload, kind::LLM_DONE);

/// Why the model stopped generating.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// A tool invocation started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStartPayload {
    /// Tool name.
    pub name: String,
    /// Tool arguments as key-value pairs.
    pub args: HashMap<String, serde_json::Value>,
}
impl_event_payload!(ToolStartPayload, kind::TOOL_START);

/// A tool invocation finished.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEndPayload {
    /// Tool name.
    pub name: String,
    /// The tool result as a JSON value.
    pub result: serde_json::Value,
}
impl_event_payload!(ToolEndPayload, kind::TOOL_END);

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
