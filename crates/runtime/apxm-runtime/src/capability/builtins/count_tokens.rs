//! `count_tokens` — estimate the token size of a string (chars/4 heuristic,
//! matching the chat compaction budget). Read-only and pure.

use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::CapabilityMetadata,
};
use apxm_core::types::values::{Number, Value};
use async_trait::async_trait;
use std::collections::HashMap;

pub struct CountTokensCapability {
    metadata: CapabilityMetadata,
}

impl CountTokensCapability {
    pub fn new() -> Self {
        Self {
            metadata: CapabilityMetadata::new(
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

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn estimates_tokens_chars_over_four() {
        let cap = CountTokensCapability::new();
        let mut args = HashMap::new();
        args.insert("text".to_string(), Value::String("a".repeat(40)));
        let out = cap.execute(args).await.unwrap();
        assert_eq!(out, Value::Number(Number::Integer(10)));
    }

    #[tokio::test]
    async fn missing_text_is_zero() {
        let cap = CountTokensCapability::new();
        let out = cap.execute(HashMap::new()).await.unwrap();
        assert_eq!(out, Value::Number(Number::Integer(0)));
    }

    #[test]
    fn is_read_only_for_safe_parallel_dispatch() {
        assert!(CountTokensCapability::new().metadata().read_only);
    }
}
