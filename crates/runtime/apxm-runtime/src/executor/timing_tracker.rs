//! Per-node prefill/decode wall-time tracker.
//!
//! Mirrors [`TokenAccountant`](super::token_accounting::TokenAccountant): the LLM
//! handler records timing per node, and the dispatcher reads it back when emitting
//! the operation-end event so it lands in `CompletedNodeInfo`.

use std::collections::HashMap;

use apxm_core::types::TimingBreakdown;
use parking_lot::RwLock;

/// Thread-safe tracker mapping `node_id` to its accumulated [`TimingBreakdown`].
#[derive(Debug, Default)]
pub struct TimingTracker {
    per_node: RwLock<HashMap<u64, TimingBreakdown>>,
}

impl TimingTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a single LLM round-trip's timing for `node_id`. Repeated calls for
    /// the same node accumulate (used by the tool-loop path).
    pub fn record(&self, node_id: u64, prefill_ms: f64, decode_ms: f64) {
        let mut map = self.per_node.write();
        let entry = map.entry(node_id).or_default();
        entry.prefill_ms += prefill_ms;
        entry.decode_ms += decode_ms;
    }

    /// Lookup the cumulative breakdown for a node. Returns `None` if no LLM call
    /// has been recorded for it.
    pub fn get_node(&self, node_id: u64) -> Option<TimingBreakdown> {
        self.per_node.read().get(&node_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_get_returns_breakdown() {
        let tracker = TimingTracker::new();
        tracker.record(7, 12.5, 0.0);
        let got = tracker.get_node(7).expect("entry recorded");
        assert!((got.prefill_ms - 12.5).abs() < f64::EPSILON);
        assert_eq!(got.decode_ms, 0.0);
    }

    #[test]
    fn record_accumulates() {
        let tracker = TimingTracker::new();
        tracker.record(1, 10.0, 2.0);
        tracker.record(1, 5.0, 3.0);
        let got = tracker.get_node(1).expect("entry recorded");
        assert!((got.prefill_ms - 15.0).abs() < f64::EPSILON);
        assert!((got.decode_ms - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn missing_node_is_none() {
        let tracker = TimingTracker::new();
        assert!(tracker.get_node(99).is_none());
    }
}
