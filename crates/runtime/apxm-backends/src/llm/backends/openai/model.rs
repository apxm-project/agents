//! Model definitions for OpenAI.
//!
//! Uses a simple string-based newtype with a list of well-known models for documentation.
//! Any model string is valid — validation happens at the API level.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Well-known OpenAI models (for documentation/autocomplete hints).
/// This list is NOT exhaustive — any model ID is valid.
pub const WELL_KNOWN_MODELS: &[&str] = &[
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-4-turbo",
    "gpt-4",
    "gpt-3.5-turbo",
    "gpt-5",
    "gpt-5-mini",
    "gpt-5-nano",
    "gpt-5.1",
    "gpt-5.2",
    "o1",
    "o1-mini",
    "o1-preview",
];

/// A model identifier string (e.g. "gpt-4o", "gpt-5").
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
        Self("gpt-4o-mini".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_id() {
        let model = ModelId::from("gpt-4o");
        assert_eq!(model.as_str(), "gpt-4o");
        assert_eq!(model.to_string(), "gpt-4o");
    }

    #[test]
    fn test_default() {
        assert_eq!(ModelId::default().as_str(), "gpt-4o-mini");
    }

    #[test]
    fn test_any_string_valid() {
        // New models should work without code changes
        let future_model = ModelId::from("gpt-6");
        assert_eq!(future_model.as_str(), "gpt-6");
    }
}
