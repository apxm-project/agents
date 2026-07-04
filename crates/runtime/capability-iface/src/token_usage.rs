//! Token usage summary — moved here (from `apxm-runtime`'s
//! `executor::token_accounting`) because it is the payload type of
//! [`crate::events::ExecutionEventEmitter::emit_operation_end`], which
//! capability's interceptor pipeline calls into.

use apxm_core::types::TokenUsage;
use serde::{Deserialize, Serialize};

/// Per-scope aggregate of input/output tokens.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsageSummary {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub total_tokens: usize,
    pub call_count: usize,
    #[serde(default)]
    pub cached_input_tokens: usize,
    #[serde(default)]
    pub reasoning_output_tokens: usize,
}

impl TokenUsageSummary {
    /// Fold one LLM call's usage into this running summary.
    ///
    /// `pub` (rather than the `apxm-runtime`-internal visibility this had
    /// before the move) because `apxm-runtime`'s `TokenAccountant` — a
    /// separate crate now — is the only caller.
    pub fn record_usage(&mut self, usage: &TokenUsage) {
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        self.total_tokens += usage.total_tokens;
        self.cached_input_tokens += usage.cached_input_tokens;
        self.reasoning_output_tokens += usage.reasoning_output_tokens;
        self.call_count += 1;
    }
}
