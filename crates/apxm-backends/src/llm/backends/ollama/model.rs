//! Model definitions for Ollama.
//!
//! Uses a simple string-based newtype with a list of common models for documentation.
//! Any model string is valid — validation happens at the API level.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Common Ollama models (for documentation/autocomplete hints).
/// This list is NOT exhaustive — any model ID is valid.
pub const COMMON_MODELS: &[&str] = &[
    "gpt-oss:120b-cloud",
    "llama3.3",
    "llama3.2",
    "llama3.1",
    "qwen2.5",
    "mistral",
    "phi3",
    "deepseek-r1",
];

/// A model identifier string (e.g. "llama3.3", "gpt-oss:120b-cloud").
/// Any string is valid — validation happens at the backend API level.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId(pub String);

impl ModelId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ModelId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ModelId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl Default for ModelId {
    fn default() -> Self {
        Self("gpt-oss:120b-cloud".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_id() {
        let model = ModelId::from("llama3.3");
        assert_eq!(model.as_str(), "llama3.3");
        assert_eq!(model.to_string(), "llama3.3");
    }

    #[test]
    fn test_default() {
        assert_eq!(ModelId::default().as_str(), "gpt-oss:120b-cloud");
    }

    #[test]
    fn test_any_string_valid() {
        // New models should work without code changes
        let custom_model = ModelId::from("my-custom-model:latest");
        assert_eq!(custom_model.as_str(), "my-custom-model:latest");
    }
}
