//! LLM response types.

use super::{FinishReason, TokenUsage, ToolCall};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-call wall-time breakdown for an LLM round-trip.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct TimingBreakdown {
    pub prefill_ms: f64,
    pub decode_ms: f64,
}

/// Response from an LLM backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMResponse {
    /// The generated content/text
    pub content: String,
    /// Which model produced this response
    pub model: String,
    /// Token usage information
    pub usage: TokenUsage,
    /// Why generation stopped
    pub finish_reason: FinishReason,
    /// Provider-specific metadata
    pub metadata: HashMap<String, serde_json::Value>,
    /// Tool calls requested by the model (if any)
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Optional prefill/decode wall-time split. Set by streaming backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<TimingBreakdown>,
}

impl LLMResponse {
    /// Create a new response.
    pub fn new(
        content: impl Into<String>,
        model: impl Into<String>,
        usage: TokenUsage,
        finish_reason: FinishReason,
    ) -> Self {
        LLMResponse {
            content: content.into(),
            model: model.into(),
            usage,
            finish_reason,
            metadata: HashMap::new(),
            tool_calls: Vec::new(),
            timing: None,
        }
    }

    /// Add provider metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Add tool calls to the response.
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = tool_calls;
        self
    }

    /// Check if the response contains tool calls.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    /// Check if generation completed normally (not via error/truncation).
    pub fn completed_normally(&self) -> bool {
        matches!(
            self.finish_reason,
            FinishReason::Stop | FinishReason::Length
        )
    }

    /// Check if the model wants to use tools.
    pub fn wants_tool_use(&self) -> bool {
        self.finish_reason == FinishReason::ToolUse || self.has_tool_calls()
    }
}

