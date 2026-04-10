//! Model definitions for Anthropic.
//!
//! Uses a simple string-based newtype with a list of well-known models for documentation.
//! Any model string is valid — validation happens at the API level.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Well-known Anthropic Claude models (for documentation/autocomplete hints).
/// This list is NOT exhaustive — any model ID is valid.
pub const WELL_KNOWN_MODELS: &[&str] = &[
    "claude-opus-4-5",
    "claude-sonnet-4-5",
    "claude-3-7-sonnet",
    "claude-3-5-sonnet-20241022",
    "claude-3-opus-20240229",
    "claude-3-sonnet-20240229",
    "claude-3-haiku-20240307",
];

/// A model identifier string (e.g. "claude-sonnet-4-5", "claude-opus-4").
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
        Self("claude-sonnet-4-5".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_id() {
        let model = ModelId::from("claude-sonnet-4-5");
        assert_eq!(model.as_str(), "claude-sonnet-4-5");
        assert_eq!(model.to_string(), "claude-sonnet-4-5");
    }

    #[test]
    fn test_default() {
        assert_eq!(ModelId::default().as_str(), "claude-sonnet-4-5");
    }

    #[test]
    fn test_any_string_valid() {
        // New models should work without code changes
        let future_model = ModelId::from("claude-opus-5-ultra");
        assert_eq!(future_model.as_str(), "claude-opus-5-ultra");
    }
}
