//! Token accounting -- aggregates LLM token costs at node, flow, and agent levels.
//!
//! Each LLM call reports its token usage via [`TokenAccountant::record`].  The
//! accountant maintains running totals that can be snapshotted at any time via
//! [`TokenAccountant::snapshot`] for inclusion in `--emit-metrics` output.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use apxm_core::types::TokenUsage;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Per-scope aggregate of input/output tokens.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsageSummary {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub total_tokens: usize,
    pub call_count: usize,
    #[serde(default)]
    pub cached_input_tokens: usize,
    #[serde(default)]
    pub reasoning_output_tokens: usize,
}

impl TokenUsageSummary {
    fn record_usage(&mut self, usage: &TokenUsage) {
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        self.total_tokens += usage.total_tokens;
        self.cached_input_tokens += usage.cached_input_tokens;
        self.reasoning_output_tokens += usage.reasoning_output_tokens;
        self.call_count += 1;
    }
}

/// Thread-safe token accountant.
///
/// Shared via `Arc<TokenAccountant>` and cloned into child contexts so that
/// all LLM calls across a single execution roll up to one aggregated view.
#[derive(Debug)]
pub struct TokenAccountant {
    per_node: RwLock<HashMap<u64, TokenUsageSummary>>,
    per_flow: RwLock<HashMap<String, TokenUsageSummary>>,
    per_agent: RwLock<HashMap<String, TokenUsageSummary>>,
    total_input: AtomicUsize,
    total_output: AtomicUsize,
    total_cached_input: AtomicUsize,
    total_reasoning_output: AtomicUsize,
    total_calls: AtomicUsize,
}

impl Default for TokenAccountant {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenAccountant {
    pub fn new() -> Self {
        Self {
            per_node: RwLock::new(HashMap::new()),
            per_flow: RwLock::new(HashMap::new()),
            per_agent: RwLock::new(HashMap::new()),
            total_input: AtomicUsize::new(0),
            total_output: AtomicUsize::new(0),
            total_cached_input: AtomicUsize::new(0),
            total_reasoning_output: AtomicUsize::new(0),
            total_calls: AtomicUsize::new(0),
        }
    }

    /// Record token usage for a single LLM call.
    pub fn record(
        &self,
        node_id: u64,
        input_tokens: usize,
        output_tokens: usize,
        flow_name: Option<&str>,
        agent_name: Option<&str>,
    ) {
        self.record_usage(
            node_id,
            &TokenUsage::new(input_tokens, output_tokens),
            flow_name,
            agent_name,
        );
    }

    /// Record complete token usage for a single LLM call.
    pub fn record_usage(
        &self,
        node_id: u64,
        usage: &TokenUsage,
        flow_name: Option<&str>,
        agent_name: Option<&str>,
    ) {
        self.total_input
            .fetch_add(usage.input_tokens, Ordering::Relaxed);
        self.total_output
            .fetch_add(usage.output_tokens, Ordering::Relaxed);
        self.total_cached_input
            .fetch_add(usage.cached_input_tokens, Ordering::Relaxed);
        self.total_reasoning_output
            .fetch_add(usage.reasoning_output_tokens, Ordering::Relaxed);
        self.total_calls.fetch_add(1, Ordering::Relaxed);

        {
            let mut map = self.per_node.write();
            map.entry(node_id).or_default().record_usage(usage);
        }

        if let Some(name) = flow_name {
            let mut map = self.per_flow.write();
            map.entry(name.to_string()).or_default().record_usage(usage);
        }

        if let Some(name) = agent_name {
            let mut map = self.per_agent.write();
            map.entry(name.to_string()).or_default().record_usage(usage);
        }
    }

    /// Lookup current usage for one node without taking a full snapshot.
    /// Returns None if no LLM call has been recorded for this node yet.
    pub fn get_node(&self, node_id: u64) -> Option<TokenUsageSummary> {
        self.per_node.read().get(&node_id).cloned()
    }

    /// Take a point-in-time snapshot for serialization / metrics emission.
    pub fn snapshot(&self) -> TokenAccountingSnapshot {
        let total = TokenUsageSummary {
            input_tokens: self.total_input.load(Ordering::Relaxed),
            output_tokens: self.total_output.load(Ordering::Relaxed),
            total_tokens: self.total_input.load(Ordering::Relaxed)
                + self.total_output.load(Ordering::Relaxed),
            call_count: self.total_calls.load(Ordering::Relaxed),
            cached_input_tokens: self.total_cached_input.load(Ordering::Relaxed),
            reasoning_output_tokens: self.total_reasoning_output.load(Ordering::Relaxed),
        };

        TokenAccountingSnapshot {
            per_node: self.per_node.read().clone(),
            per_flow: self.per_flow.read().clone(),
            per_agent: self.per_agent.read().clone(),
            total,
        }
    }
}

/// Serializable snapshot of token accounting state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenAccountingSnapshot {
    pub per_node: HashMap<u64, TokenUsageSummary>,
    pub per_flow: HashMap<String, TokenUsageSummary>,
    pub per_agent: HashMap<String, TokenUsageSummary>,
    pub total: TokenUsageSummary,
}

impl TokenAccountingSnapshot {
    /// Export as a JSON value for `--emit-metrics`.
    pub fn to_json(&self) -> serde_json::Value {
        use apxm_core::constants::session::metrics_keys as mk;
        serde_json::json!({
            mk::TOKEN_ACCOUNTING: {
                mk::TOTAL: self.total,
                mk::PER_NODE: self.per_node,
                mk::PER_FLOW: self.per_flow,
                mk::PER_AGENT: self.per_agent,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_snapshot() {
        let accountant = TokenAccountant::new();

        accountant.record(1, 100, 50, Some("flow_a"), Some("agent_x"));
        accountant.record(2, 200, 80, Some("flow_a"), Some("agent_x"));
        accountant.record(3, 50, 20, Some("flow_b"), Some("agent_y"));

        let snap = accountant.snapshot();

        assert_eq!(snap.total.input_tokens, 350);
        assert_eq!(snap.total.output_tokens, 150);
        assert_eq!(snap.total.total_tokens, 500);
        assert_eq!(snap.total.call_count, 3);

        assert_eq!(snap.per_node.len(), 3);
        assert_eq!(snap.per_node[&1].input_tokens, 100);

        assert_eq!(snap.per_flow.len(), 2);
        assert_eq!(snap.per_flow["flow_a"].call_count, 2);
        assert_eq!(snap.per_flow["flow_a"].input_tokens, 300);

        assert_eq!(snap.per_agent.len(), 2);
        assert_eq!(snap.per_agent["agent_x"].total_tokens, 430);
    }

    #[test]
    fn test_record_usage_preserves_detailed_subcounts() {
        let accountant = TokenAccountant::new();
        let usage = TokenUsage::new(100, 40).with_details(60, 25);

        accountant.record_usage(7, &usage, Some("flow_a"), Some("agent_x"));
        let snap = accountant.snapshot();

        assert_eq!(snap.total.input_tokens, 100);
        assert_eq!(snap.total.output_tokens, 40);
        assert_eq!(snap.total.cached_input_tokens, 60);
        assert_eq!(snap.total.reasoning_output_tokens, 25);
        assert_eq!(snap.per_node[&7].cached_input_tokens, 60);
        assert_eq!(snap.per_flow["flow_a"].reasoning_output_tokens, 25);
        assert_eq!(snap.per_agent["agent_x"].cached_input_tokens, 60);
    }

    #[test]
    fn test_empty_snapshot() {
        let accountant = TokenAccountant::new();
        let snap = accountant.snapshot();
        assert_eq!(snap.total.call_count, 0);
        assert!(snap.per_node.is_empty());
    }

    #[test]
    fn test_snapshot_json() {
        let accountant = TokenAccountant::new();
        accountant.record(1, 10, 5, None, None);
        let snap = accountant.snapshot();
        let json = snap.to_json();
        assert!(
            json["token_accounting"]["total"]["total_tokens"]
                .as_u64()
                .is_some()
        );
    }
}
