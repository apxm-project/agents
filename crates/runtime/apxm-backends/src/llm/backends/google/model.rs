//! Model definitions for Google Gemini.
//!
//! Uses a simple string-based newtype with a list of well-known models for documentation.
//! Any model string is valid — validation happens at the API level.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Well-known Google Gemini models (for documentation/autocomplete hints).
/// This list is NOT exhaustive — any model ID is valid.
pub const WELL_KNOWN_MODELS: &[&str] = &[
    "gemini-2.5-flash",
    "gemini-2.0-pro",
    "gemini-1.5-pro",
    "gemini-1.5-flash",
];

/// A model identifier string (e.g. "gemini-2.5-flash").
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
        Self("gemini-2.5-flash".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_id() {
        let model = ModelId::from("gemini-2.5-flash");
        assert_eq!(model.as_str(), "gemini-2.5-flash");
        assert_eq!(model.to_string(), "gemini-2.5-flash");
    }

    #[test]
    fn test_default() {
        assert_eq!(ModelId::default().as_str(), "gemini-2.5-flash");
    }

    #[test]
    fn test_any_string_valid() {
        // New models should work without code changes
        let future_model = ModelId::from("gemini-3.0-ultra");
        assert_eq!(future_model.as_str(), "gemini-3.0-ultra");
    }
}
