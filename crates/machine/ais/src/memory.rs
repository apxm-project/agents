//! Memory Tier Definitions
//!
//! A-PXM uses a three-tier memory hierarchy:
//!
//! 1. **STM (Short-Term Memory)**: Fast access to recent context and tool output
//! 2. **LTM (Long-Term Memory)**: Persistent knowledge store
//! 3. **Episodic**: Execution traces for reflection and debugging

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// Error type for memory tier parsing.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum MemoryTierParseError {
    #[error("Unknown memory tier: {0}")]
    UnknownTier(String),
}

/// Memory tier in the A-PXM memory hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum MemoryTier {
    /// Short-Term Memory: Fast access to recent context.
    /// Typically implemented as an LRU cache with O(1) access.
    #[default]
    Stm,

    /// Long-Term Memory: Persistent knowledge store.
    /// Typically backed by a database with vector similarity search.
    Ltm,

    /// Episodic Memory: Execution traces for reflection.
    /// Append-only log of state transitions and events.
    Episodic,
}

impl MemoryTier {
    /// Get the tier name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryTier::Stm => "stm",
            MemoryTier::Ltm => "ltm",
            MemoryTier::Episodic => "episodic",
        }
    }
}

impl FromStr for MemoryTier {
    type Err = MemoryTierParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stm" | "short-term" | "short_term" => Ok(MemoryTier::Stm),
            "ltm" | "long-term" | "long_term" => Ok(MemoryTier::Ltm),
            "episodic" | "trace" | "log" => Ok(MemoryTier::Episodic),
            _ => Err(MemoryTierParseError::UnknownTier(s.to_string())),
        }
    }
}

impl MemoryTier {
    /// Check if tier supports semantic search.
    pub fn supports_semantic_search(&self) -> bool {
        match self {
            MemoryTier::Stm | MemoryTier::Episodic => false,
            MemoryTier::Ltm => true,
        }
    }

    /// Check if tier is append-only.
    pub fn is_append_only(&self) -> bool {
        matches!(self, MemoryTier::Episodic)
    }
}

impl fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
