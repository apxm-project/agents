//! Scheduler configuration.
//!
//! This module defines configuration options for the dataflow scheduler,
//! including parallelism settings, retry behavior, and resource limits.

use apxm_core::types::LatencyTierConfig;
use serde::{Deserialize, Serialize};

/// Configuration for the dataflow scheduler.
///
/// Controls parallelism, work-stealing behavior, retry logic, and resource limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerConfig {
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,

    #[serde(default = "default_max_inflight")]
    pub max_inflight: usize,

    /// Separate concurrency cap for LLM operations (Ask/Think/Reason).
    ///
    /// Decoupled from `max_inflight` so that compute-bound parallelism (CPU
    /// cores) can stay tight while LLM requests can fan out wide enough to
    /// keep continuous-batching backends saturated.
    #[serde(default = "default_llm_inflight")]
    pub llm_inflight: usize,

    /// Concurrency cap for long-WAITING ops (PAUSE/RESUME/recv) that block on an
    /// external event rather than consuming CPU or an LLM slot. Generous and
    /// separate so a burst of human-in-the-loop pauses cannot exhaust the compute
    /// or LLM pools and stall real work (the documented PAUSE deadlock). These
    /// ops do near-zero compute while parked, so a high cap is safe.
    #[serde(default = "default_blocking_inflight")]
    pub blocking_inflight: usize,

    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Exponential backoff: initial_delay * 2^attempt, capped by `retry_backoff_max_ms`.
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: u64,

    #[serde(default = "default_retry_backoff_max_ms")]
    pub retry_backoff_max_ms: u64,

    #[serde(default = "default_watchdog_interval_ms")]
    pub watchdog_interval_ms: u64,

    #[serde(default = "default_deadlock_timeout_ms")]
    pub deadlock_timeout_ms: u64,

    /// 0 means unlimited.
    #[serde(default)]
    pub max_cost: usize,

    #[serde(default = "default_queue_capacity")]
    pub queue_capacity: usize,

    /// Overrides compile-time node latencies when a node's `"backend"` attribute matches a key here.
    #[serde(default)]
    pub latency_tiers: LatencyTierConfig,

    #[serde(default)]
    pub collect_all_outputs: bool,

    /// Streams producer output to downstream consumers before completion (ASK->ASK only).
    #[serde(default)]
    pub pipeline: PipelineConfig,

    /// Gates the undocumented-by-default sequential fallback
    /// (`ExecutorEngine::execute_dag_inner`): when the parallel dataflow
    /// scheduler errors, `false` (the default) propagates the error instead
    /// of silently re-running the DAG sequentially. `true` is an explicit,
    /// logged opt-in that restores sequential execution for a deployment that
    /// depends on it. Off by default in CI so a real scheduler bug fails loud
    /// instead of hiding behind a
    /// bimodal-latency fallback nobody pages on.
    #[serde(default)]
    pub allow_sequential_fallback: bool,
}

/// Configuration for token pipelining (research feature).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineConfig {
    #[serde(default)]
    pub enabled: bool,

    /// Minimum tokens to accumulate before starting downstream request.
    #[serde(default = "default_min_tokens")]
    pub min_tokens: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_tokens: default_min_tokens(),
        }
    }
}

fn default_min_tokens() -> usize {
    100
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrency: default_max_concurrency(),
            max_inflight: default_max_inflight(),
            llm_inflight: default_llm_inflight(),
            blocking_inflight: default_blocking_inflight(),
            max_retries: default_max_retries(),
            retry_backoff_ms: default_retry_backoff_ms(),
            retry_backoff_max_ms: default_retry_backoff_max_ms(),
            watchdog_interval_ms: default_watchdog_interval_ms(),
            deadlock_timeout_ms: default_deadlock_timeout_ms(),
            max_cost: 0,
            queue_capacity: default_queue_capacity(),
            latency_tiers: LatencyTierConfig::default(),
            collect_all_outputs: false,
            pipeline: PipelineConfig::default(),
            allow_sequential_fallback: false,
        }
    }
}

impl SchedulerConfig {
    /// Create a new scheduler configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    pub fn with_max_inflight(mut self, max_inflight: usize) -> Self {
        self.max_inflight = max_inflight;
        self
    }

    pub fn with_llm_inflight(mut self, llm_inflight: usize) -> Self {
        self.llm_inflight = llm_inflight;
        self
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_retry_backoff(mut self, initial_ms: u64, max_ms: u64) -> Self {
        self.retry_backoff_ms = initial_ms;
        self.retry_backoff_max_ms = max_ms;
        self
    }

    pub fn with_deadlock_timeout(mut self, timeout_ms: u64) -> Self {
        self.deadlock_timeout_ms = timeout_ms;
        self
    }

    pub fn with_max_cost(mut self, max_cost: usize) -> Self {
        self.max_cost = max_cost;
        self
    }

    pub fn with_latency_tiers(mut self, latency_tiers: LatencyTierConfig) -> Self {
        self.latency_tiers = latency_tiers;
        self
    }

    pub fn with_collect_all_outputs(mut self, collect_all_outputs: bool) -> Self {
        self.collect_all_outputs = collect_all_outputs;
        self
    }

    /// Explicit, logged opt-in to the demoted sequential fallback. Off by
    /// default (`SchedulerConfig::default()`); see the field doc comment.
    pub fn with_allow_sequential_fallback(mut self, allow: bool) -> Self {
        self.allow_sequential_fallback = allow;
        self
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_concurrency == 0 {
            return Err("max_concurrency must be > 0".to_string());
        }

        if self.max_inflight == 0 {
            return Err("max_inflight must be > 0".to_string());
        }

        if self.llm_inflight == 0 {
            return Err("llm_inflight must be > 0".to_string());
        }

        if self.retry_backoff_ms == 0 {
            return Err("retry_backoff_ms must be > 0".to_string());
        }

        if self.retry_backoff_max_ms < self.retry_backoff_ms {
            return Err("retry_backoff_max_ms must be >= retry_backoff_ms".to_string());
        }

        if self.watchdog_interval_ms == 0 {
            return Err("watchdog_interval_ms must be > 0".to_string());
        }

        if self.deadlock_timeout_ms < self.watchdog_interval_ms {
            return Err("deadlock_timeout_ms must be >= watchdog_interval_ms".to_string());
        }

        Ok(())
    }
}

// Default functions for serde
fn default_max_concurrency() -> usize {
    num_cpus::get().max(1)
}

fn default_max_inflight() -> usize {
    default_max_concurrency() * 2
}

fn default_llm_inflight() -> usize {
    32
}

fn default_blocking_inflight() -> usize {
    // Generous: parked PAUSE/RESUME/recv ops do ~no compute, so this only bounds
    // pathological unbounded growth, not real parallelism.
    4096
}

fn default_max_retries() -> u32 {
    3
}

fn default_retry_backoff_ms() -> u64 {
    500
}

fn default_retry_backoff_max_ms() -> u64 {
    60_000
}

fn default_watchdog_interval_ms() -> u64 {
    1_000
}

fn default_deadlock_timeout_ms() -> u64 {
    120_000 // 2 minutes - allows for multiple sequential LLM calls
}

fn default_queue_capacity() -> usize {
    10_000
}
