//! Canonical semantic-contract record: every backend call described in a
//! deterministic, comparable form. Used by tier-1 semantic-equivalence tests.

use serde::{Deserialize, Serialize};

use crate::types::operations::AISOperationType;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallEvent {
    /// AIR node id this call originated from.
    pub node_id: u64,
    /// Source-level node name (e.g. "security_review").
    pub node_name: String,
    /// AIS op kind that originated the backend call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<AISOperationType>,
    /// Fully-substituted prompt text seen by the backend.
    pub prompt: String,
    /// Resolved model id (e.g. "gpt-4o-mini" or vLLM tag).
    pub model: String,
    /// Sampling params normalized to a stable string form.
    pub params: String,
    /// Sorted upstream node ids whose outputs fed this call.
    pub parent_deps: Vec<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallTrace {
    pub events: Vec<CallEvent>,
}

impl CallTrace {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, evt: CallEvent) {
        self.events.push(evt);
    }
    pub fn len(&self) -> usize {
        self.events.len()
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

