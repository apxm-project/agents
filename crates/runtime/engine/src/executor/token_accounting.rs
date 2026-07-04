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

// `TokenUsageSummary` moved to `apxm-capability-iface` — it's the payload
// type of `ExecutionEventEmitter::emit_operation_end`, which now lives there
// too. Re-exported here so `crate::executor::token_accounting::TokenUsageSummary`
// keeps working unchanged.
pub use apxm_capability_iface::TokenUsageSummary;

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

/// Process-wide token meter shared by every entry point that spends tokens,
/// including deliberate executor bypasses like `/v1/generate` (RT-8: "route
/// its token accounting through the same meters the engine uses"). Unlike
/// [`ExecutionContext::token_accountant`] (fresh per execution, reset to zero
/// at execution start so a run's own snapshot is self-contained), this is one
/// process-lifetime accumulator: it is the seam that lets a caller assert
/// "every token spent by this process — executor path or fast path — landed
/// in one place," without changing the per-execution snapshot semantics the
/// executor and its tests already depend on.
static GLOBAL_TOKEN_METER: std::sync::OnceLock<TokenAccountant> = std::sync::OnceLock::new();

/// The process-wide token meter. Both the executor's LLM call handlers
/// (`executor/handlers/llm/mod.rs`, `.../tool_dispatch.rs`) and
/// `apxm-server`'s `/v1/generate` fast path record into this same instance.
pub fn global_meter() -> &'static TokenAccountant {
    GLOBAL_TOKEN_METER.get_or_init(TokenAccountant::new)
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
