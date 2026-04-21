//! Canonical semantic-contract record: every backend call described in a
//! deterministic, comparable form. Used by tier-1 semantic-equivalence tests.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallEvent {
    /// AIR node id this call originated from.
    pub node_id: u64,
    /// Source-level node name (e.g. "security_review").
    pub node_name: String,
    /// AIS op kind (e.g. "ASK", "THINK", "TOOL_DISPATCH").
    pub op: String,
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
    pub fn new() -> Self { Self::default() }
    pub fn push(&mut self, evt: CallEvent) { self.events.push(evt); }
    pub fn len(&self) -> usize { self.events.len() }
    pub fn is_empty(&self) -> bool { self.events.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_trace_roundtrip_json() {
        let mut t = CallTrace::new();
        t.push(CallEvent {
            node_id: 1,
            node_name: "ask_a".into(),
            op: "ASK".into(),
            prompt: "hello".into(),
            model: "mock".into(),
            params: "temp=0,top_p=1".into(),
            parent_deps: vec![],
        });
        let json = serde_json::to_string(&t).expect("serialize");
        let back: CallTrace = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, back);
        assert_eq!(back.len(), 1);
    }
}
