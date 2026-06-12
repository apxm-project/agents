//! Operation categories for AIS operations.

use serde::{Deserialize, Serialize};

/// Categories for AIS operations, used by the scheduler to determine behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationCategory {
    /// Metadata operations.
    Metadata,
    /// Memory operations.
    Memory,
    /// Reasoning operations.
    Reasoning,
    /// Tool operations.
    Tools,
    /// Control flow operations.
    ControlFlow,
    /// Synchronization operations.
    Synchronization,
    /// Error handling operations.
    ErrorHandling,
    /// Communication operations.
    Communication,
    /// Coordination operations.
    Coordination,
    /// Identity operations.
    Identity,
    /// Internal operations.
    Internal,
}

impl OperationCategory {
    /// Returns true if operations in this category require LLM calls.
    pub fn requires_llm(&self) -> bool {
        matches!(self, OperationCategory::Reasoning)
    }

    /// Returns true if operations in this category are I/O bound.
    pub fn is_io_bound(&self) -> bool {
        matches!(
            self,
            OperationCategory::Memory
                | OperationCategory::Reasoning
                | OperationCategory::Tools
                | OperationCategory::Communication
                | OperationCategory::Coordination
        )
    }

    /// Returns true if operations affect control flow.
    pub fn affects_control_flow(&self) -> bool {
        matches!(
            self,
            OperationCategory::ControlFlow | OperationCategory::ErrorHandling
        )
    }

    /// Returns true if this is a metadata category (no execution).
    pub fn is_metadata(&self) -> bool {
        matches!(self, OperationCategory::Metadata)
    }
}
