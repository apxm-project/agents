//! Profile-guided optimization infrastructure.
//!
//! Collects runtime execution profiles and applies them to graphs before
//! MLIR lowering. The profile annotates nodes with observed latency,
//! error-rate, and token-usage data so that downstream passes (scheduling,
//! fusion) can make better decisions.
//!
//! # File format
//!
//! Profiles are serialized as JSON via [`ExecutionProfile::save_to_file`] and
//! loaded with [`ExecutionProfile::load_from_file`].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::air_builder::AirModule;
use apxm_core::{constants::graph::attrs as graph_attrs, types::Value};

/// Error-rate threshold above which a retry attribute is injected.
const ERROR_RATE_RETRY_THRESHOLD: f64 = 0.05;

/// Default retry count injected for high-error-rate nodes.
const DEFAULT_RETRY_COUNT: i64 = 2;

/// Runtime execution profile collected from previous runs.
///
/// Keyed by node *name* (not ID) so profiles remain valid across
/// re-compilations that change node numbering.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionProfile {
    /// Per-node execution statistics: node name -> stats.
    pub node_stats: HashMap<String, NodeProfile>,
    /// Total number of executions that contributed to this profile.
    pub execution_count: u64,
}

/// Per-node runtime statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeProfile {
    /// Average latency in milliseconds.
    pub avg_latency_ms: u64,
    /// 99th-percentile latency in milliseconds.
    pub p99_latency_ms: u64,
    /// Total number of times this node was invoked.
    pub call_count: u64,
    /// Average token count consumed per invocation.
    pub avg_tokens: u64,
    /// Fraction of invocations that resulted in an error (0.0 .. 1.0).
    pub error_rate: f64,
}

impl ExecutionProfile {
    /// Load a profile from a JSON file on disk.
    pub fn load_from_file(path: &Path) -> Result<Self, ProfileError> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| ProfileError::Io(path.display().to_string(), e))?;
        let profile: Self = serde_json::from_str(&data)
            .map_err(|e| ProfileError::Parse(path.display().to_string(), e.to_string()))?;
        Ok(profile)
    }

    /// Persist the profile as JSON.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ProfileError> {
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| ProfileError::Parse("serialization".to_string(), e.to_string()))?;
        std::fs::write(path, data).map_err(|e| ProfileError::Io(path.display().to_string(), e))?;
        Ok(())
    }

    /// Merge another profile into this one.
    ///
    /// Statistics are combined as weighted averages using the respective
    /// execution counts. Call counts and execution counts are summed.
    pub fn merge(&mut self, other: &ExecutionProfile) {
        let total = self.execution_count + other.execution_count;
        if total == 0 {
            return;
        }

        for (name, other_stats) in &other.node_stats {
            let entry = self.node_stats.entry(name.clone());
            match entry {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    let existing = e.get_mut();
                    let combined_calls = existing.call_count + other_stats.call_count;
                    if combined_calls > 0 {
                        existing.avg_latency_ms = weighted_avg(
                            existing.avg_latency_ms,
                            existing.call_count,
                            other_stats.avg_latency_ms,
                            other_stats.call_count,
                        );
                        existing.avg_tokens = weighted_avg(
                            existing.avg_tokens,
                            existing.call_count,
                            other_stats.avg_tokens,
                            other_stats.call_count,
                        );
                        existing.error_rate = weighted_avg_f64(
                            existing.error_rate,
                            existing.call_count,
                            other_stats.error_rate,
                            other_stats.call_count,
                        );
                        existing.p99_latency_ms =
                            existing.p99_latency_ms.max(other_stats.p99_latency_ms);
                    }
                    existing.call_count = combined_calls;
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(other_stats.clone());
                }
            }
        }
        self.execution_count = total;
    }

    /// Apply profile data to a module.
    ///
    /// For each node in the module whose name appears in the profile:
    ///   - Writes `__profile_latency_ms` and `__profile_p99_latency_ms` attributes.
    ///   - Writes `__profile_error_rate` and `__profile_avg_tokens` attributes.
    ///   - If `error_rate > 0.05`, injects a `retry_count` attribute (if not
    ///     already set).
    ///   - If `avg_tokens > token_budget`, writes a `__profile_token_warning`
    ///     attribute with a human-readable message.
    ///
    /// Returns the number of nodes that were annotated.
    pub fn apply_to_module(&self, module: &mut AirModule, token_budget: Option<u64>) -> usize {
        let mut annotated = 0;
        for node in &mut module.nodes {
            if let Some(stats) = self.node_stats.get(&node.name) {
                // Latency annotations
                node.attributes.insert(
                    graph_attrs::PROFILE_LATENCY_MS.to_string(),
                    Value::Number(apxm_core::types::Number::Integer(
                        stats.avg_latency_ms as i64,
                    )),
                );
                node.attributes.insert(
                    graph_attrs::PROFILE_P99_LATENCY_MS.to_string(),
                    Value::Number(apxm_core::types::Number::Integer(
                        stats.p99_latency_ms as i64,
                    )),
                );

                // Error-rate annotation
                node.attributes.insert(
                    graph_attrs::PROFILE_ERROR_RATE.to_string(),
                    Value::Number(apxm_core::types::Number::Float(stats.error_rate)),
                );

                // Token usage annotation
                node.attributes.insert(
                    graph_attrs::PROFILE_AVG_TOKENS.to_string(),
                    Value::Number(apxm_core::types::Number::Integer(stats.avg_tokens as i64)),
                );

                // Inject retry_count for high-error-rate nodes
                if stats.error_rate > ERROR_RATE_RETRY_THRESHOLD
                    && !node.attributes.contains_key(graph_attrs::RETRY_COUNT)
                {
                    node.attributes.insert(
                        graph_attrs::RETRY_COUNT.to_string(),
                        Value::Number(apxm_core::types::Number::Integer(DEFAULT_RETRY_COUNT)),
                    );
                }

                // Token budget warning
                if let Some(budget) = token_budget {
                    if stats.avg_tokens > budget {
                        node.attributes.insert(
                            graph_attrs::PROFILE_TOKEN_WARNING.to_string(),
                            Value::String(format!(
                                "avg_tokens ({}) exceeds budget ({})",
                                stats.avg_tokens, budget
                            )),
                        );
                    }
                }

                annotated += 1;
            }
        }
        annotated
    }
}

/// Profile-related errors.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("I/O error on profile file '{0}': {1}")]
    Io(String, #[source] std::io::Error),
    #[error("failed to parse profile '{0}': {1}")]
    Parse(String, String),
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn weighted_avg(a: u64, w_a: u64, b: u64, w_b: u64) -> u64 {
    let total = w_a + w_b;
    if total == 0 {
        return 0;
    }
    (a as u128 * w_a as u128 + b as u128 * w_b as u128)
        .checked_div(total as u128)
        .unwrap_or(0) as u64
}

fn weighted_avg_f64(a: f64, w_a: u64, b: f64, w_b: u64) -> f64 {
    let total = w_a + w_b;
    if total == 0 {
        return 0.0;
    }
    (a * w_a as f64 + b * w_b as f64) / total as f64
}
