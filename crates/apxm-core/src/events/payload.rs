//! All 33 `EventPayload` variants, organized into three layers:
//! LLM (9), Runtime (16), and Session (8).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Top-level enum
// ---------------------------------------------------------------------------

/// Discriminated-union payload for every APXM event.
///
/// Tagged with `"kind"` in JSON so consumers can match on the variant name.
/// Marked `#[non_exhaustive]` so new variants can be added without a
/// semver-breaking change.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventPayload {
    // ── LLM Layer (9) ────────────────────────────────────────────────
    /// A single streaming token from the LLM.
    Token(TokenPayload),
    /// An extended-thinking / chain-of-thought block.
    Thought(ThoughtPayload),
    /// The model requested a tool call.
    ToolCall(ToolCallPayload),
    /// The model finished generating a complete response.
    LlmDone(LlmDonePayload),
    /// Token usage accounting from the LLM.
    Usage(UsagePayload),
    /// An API call is being retried.
    Retry(RetryPayload),
    /// A non-fatal warning from the backend.
    Warning(WarningPayload),
    /// Citations returned by the model.
    Citation(CitationPayload),
    /// An opaque provider-specific event.
    ProviderEvent(ProviderEventPayload),

    // ── Runtime Layer (16) — mirrors ExecutionEvent variants ─────────
    /// An operation (graph node) started executing.
    OperationStart(OperationStartPayload),
    /// An operation finished executing.
    OperationEnd(OperationEndPayload),
    /// A tool invocation started.
    ToolStart(ToolStartPayload),
    /// A tool invocation finished.
    ToolEnd(ToolEndPayload),
    /// A plan was created by the planner.
    PlanCreated(PlanCreatedPayload),
    /// A plan step started.
    PlanStepStarted(PlanStepStartedPayload),
    /// A plan step completed.
    PlanStepCompleted(PlanStepCompletedPayload),
    /// A read from the memory subsystem.
    MemoryRead(MemoryReadPayload),
    /// A write to the memory subsystem.
    MemoryWrite(MemoryWritePayload),
    /// A checkpoint was saved.
    CheckpointSaved(CheckpointSavedPayload),
    /// A checkpoint was restored.
    CheckpointRestored(CheckpointRestoredPayload),
    /// The scheduler made a scheduling decision.
    SchedulerDecision(SchedulerDecisionPayload),
    /// GPU utilization snapshot.
    GpuUtilization(GpuUtilizationPayload),
    /// Token usage per graph node.
    TokenUsage(TokenUsagePayload),
    /// A memoization cache hit.
    MemoizationHit(MemoizationHitPayload),
    /// A runtime error event.
    Error(ErrorPayload),

    // ── Session Layer (8) ────────────────────────────────────────────
    /// The context window was compacted.
    ContextCompacted(ContextCompactedPayload),
    /// The model was rerouted to a different backend.
    ModelRerouted(ModelReroutedPayload),
    /// The request was cancelled.
    Cancelled(CancelledPayload),
    /// A loop/cycle was detected in the conversation.
    LoopDetected(LoopDetectedPayload),
    /// The context window is approaching capacity.
    ContextWindowWarning(ContextWindowWarningPayload),
    /// A new session started.
    SessionStart(SessionStartPayload),
    /// A session ended.
    SessionEnd(SessionEndPayload),
    /// A turn boundary (request/response) was reached.
    TurnBoundary(TurnBoundaryPayload),
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

/// An extended-thinking / chain-of-thought block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtPayload {
    /// The thought text.
    pub text: String,
    /// Optional short summary of the thought.
    pub summary: Option<String>,
}

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

/// A non-fatal warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningPayload {
    /// Machine-readable warning code.
    pub code: String,
    /// Human-readable warning message.
    pub message: String,
}

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

// ===========================================================================
// Runtime Layer payload structs (mirror ExecutionEvent)
// ===========================================================================

/// An operation (graph node) started executing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationStartPayload {
    /// The graph node ID.
    pub node_id: u64,
    /// The operation type (e.g. `"ASK"`, `"THINK"`).
    pub op_type: String,
}

/// An operation finished executing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationEndPayload {
    /// The graph node ID.
    pub node_id: u64,
    /// The operation type.
    pub op_type: String,
    /// How long the operation took, in milliseconds.
    pub duration_ms: u64,
    /// Whether the operation succeeded.
    pub success: bool,
}

/// A tool invocation started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStartPayload {
    /// Tool name.
    pub name: String,
    /// Tool arguments as key-value pairs.
    pub args: HashMap<String, serde_json::Value>,
}

/// A tool invocation finished.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEndPayload {
    /// Tool name.
    pub name: String,
    /// The tool result as a JSON value.
    pub result: serde_json::Value,
}

/// A plan was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCreatedPayload {
    /// Plan identifier.
    pub plan_id: String,
    /// Number of steps in the plan.
    pub steps: usize,
}

/// A plan step started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStepStartedPayload {
    /// Plan identifier.
    pub plan_id: String,
    /// Zero-based step index.
    pub step_index: usize,
}

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

/// A memory read event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryReadPayload {
    /// Memory scope (e.g. `"short_term"`, `"long_term"`).
    pub scope: String,
    /// The key that was read.
    pub key: String,
}

/// A memory write event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWritePayload {
    /// Memory scope.
    pub scope: String,
    /// The key that was written.
    pub key: String,
}

/// A checkpoint was saved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointSavedPayload {
    /// Checkpoint identifier.
    pub checkpoint_id: String,
}

/// A checkpoint was restored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRestoredPayload {
    /// Checkpoint identifier.
    pub checkpoint_id: String,
}

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

/// A memoization cache hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoizationHitPayload {
    /// The graph node whose result was memoized.
    pub node_id: u64,
}

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

/// The request was cancelled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelledPayload {
    /// Optional reason for cancellation.
    pub reason: Option<String>,
}

/// A loop/cycle was detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDetectedPayload {
    /// Description of the detected pattern.
    pub pattern: String,
    /// How many iterations of the loop were observed.
    pub iterations: usize,
}

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

/// A new session started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartPayload {
    /// Session identifier.
    pub session_id: String,
}

/// A session ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEndPayload {
    /// Session identifier.
    pub session_id: String,
    /// Total number of turns in the session.
    pub total_turns: usize,
}

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
