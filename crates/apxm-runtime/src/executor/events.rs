//! Execution event emission hooks.

use std::collections::HashMap;

use apxm_core::types::values::Value;
use serde::{Deserialize, Serialize};

/// Runtime execution event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEvent {
    LlmToken {
        content: String,
    },
    ToolStart {
        name: String,
        args: HashMap<String, Value>,
    },
    ToolEnd {
        name: String,
        result: Value,
    },
    TokenUsage {
        node_id: u64,
        input_tokens: usize,
        output_tokens: usize,
    },
    MemoizationHit {
        node_id: u64,
    },
}

/// Optional observer for execution events.
pub trait ExecutionEventEmitter: Send + Sync {
    fn emit_llm_token(&self, content: &str);
    fn emit_tool_start(&self, name: &str, args: &HashMap<String, Value>);
    fn emit_tool_end(&self, name: &str, result: &Value);
    fn emit_token_usage(&self, _node_id: u64, _input_tokens: usize, _output_tokens: usize) {}
    fn emit_memoization_hit(&self, _node_id: u64) {}
}
