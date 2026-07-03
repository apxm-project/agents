//! `count_tokens` — estimate the token size of a string (chars/4 heuristic,
//! matching the chat compaction budget). Read-only and pure.

use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::RuntimeCapability,
};
use apxm_core::types::values::{Number, Value};
use async_trait::async_trait;
use std::collections::HashMap;

pub struct CountTokensCapability {
    metadata: RuntimeCapability,
}

impl CountTokensCapability {
    pub fn new() -> Self {
        Self {
            metadata: RuntimeCapability::new(
                "count_tokens",
                "Estimate the token count of a string (chars/4). Use to decide \
                 when to compact/summarize conversation context.",
                serde_json::json!({
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }),
            )
            .with_returns("integer (estimated tokens)")
            .with_groups(vec!["text".to_string()])
            .with_read_only(),
        }
    }

    fn estimate(text: &str) -> i64 {
        (text.len() / 4) as i64
    }
}

impl Default for CountTokensCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for CountTokensCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
        Ok(Value::Number(Number::Integer(Self::estimate(text))))
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}
