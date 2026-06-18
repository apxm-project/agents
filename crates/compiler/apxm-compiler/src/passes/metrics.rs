//! Per-pass compiler diagnostics.
//!
//! [`PassMetrics`] captures timing and IR-size data for a single compiler pass.
//! [`PipelineDiagnostics`] aggregates metrics across an entire pipeline run.

use apxm_core::constants::session::metrics_keys;
use apxm_core::metrics::MetricsSource;
use serde::{Deserialize, Serialize};

/// Metrics collected for a single compiler pass execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassMetrics {
    /// Name of the pass (e.g. "normalize", "fuse-ask-ops").
    pub pass_name: String,
    /// Wall-clock duration of the pass in milliseconds.
    pub duration_ms: f64,
    /// Number of MLIR operations before the pass ran.
    pub ops_before: usize,
    /// Number of MLIR operations after the pass ran.
    pub ops_after: usize,
    /// Net change in operation count (positive = growth, negative = elimination).
    pub ops_delta: isize,
    /// Number of pattern matches the pass fired (0 if pass did not run or did not surface stats).
    #[serde(default)]
    pub fired_count: usize,
    /// Net change in serialized IR text length, in bytes.
    #[serde(default)]
    pub ir_size_delta: isize,
    /// Estimated template tokens saved by this pass; `None` if the pass does not
    /// produce a token estimate (e.g. structural passes like canonicalizer).
    #[serde(default)]
    pub tokens_saved: Option<usize>,
}

/// Aggregated diagnostics for a full pipeline execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineDiagnostics {
    /// Per-pass metrics in execution order.
    pub passes: Vec<PassMetrics>,
    /// Total wall-clock time for all passes in milliseconds.
    pub total_duration_ms: f64,
    /// Operation count before the first pass.
    pub initial_ops: usize,
    /// Operation count after the last pass.
    pub final_ops: usize,
}

impl PipelineDiagnostics {
    /// Create an empty diagnostics container.
    pub fn new() -> Self {
        Self {
            passes: Vec::new(),
            total_duration_ms: 0.0,
            initial_ops: 0,
            final_ops: 0,
        }
    }

    /// Number of passes recorded.
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    /// Total operations eliminated across all passes.
    pub fn total_ops_eliminated(&self) -> usize {
        self.passes
            .iter()
            .filter(|p| p.ops_delta < 0)
            .map(|p| (-p.ops_delta) as usize)
            .sum()
    }

    /// Names of passes that changed the IR (ops_delta != 0).
    pub fn active_passes(&self) -> Vec<&str> {
        self.passes
            .iter()
            .filter(|p| p.ops_delta != 0)
            .map(|p| p.pass_name.as_str())
            .collect()
    }

    /// Sum of `tokens_saved` across all passes that reported a value.
    pub fn total_tokens_saved(&self) -> usize {
        self.passes.iter().filter_map(|p| p.tokens_saved).sum()
    }

    /// Names of passes that fired at least one pattern (`fired_count > 0`).
    pub fn fired_passes(&self) -> Vec<&str> {
        self.passes
            .iter()
            .filter(|p| p.fired_count > 0)
            .map(|p| p.pass_name.as_str())
            .collect()
    }

    /// Serialize to the unified metrics schema: `{ "passes": [...], "summary": {...} }`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            metrics_keys::COMPILER_PASSES: self.passes,
            metrics_keys::COMPILER_SUMMARY: {
                metrics_keys::SUMMARY_TOTAL_PASSES: self.pass_count(),
                metrics_keys::SUMMARY_INITIAL_OPS: self.initial_ops,
                metrics_keys::SUMMARY_FINAL_OPS: self.final_ops,
                metrics_keys::SUMMARY_TOTAL_OPS_ELIMINATED: self.total_ops_eliminated(),
                metrics_keys::SUMMARY_TOTAL_TOKENS_SAVED: self.total_tokens_saved(),
                metrics_keys::SUMMARY_FIRED_PASSES: self.fired_passes(),
                metrics_keys::SUMMARY_ACTIVE_PASSES: self.active_passes()
            }
        })
    }
}

/// Wraps `PipelineDiagnostics` as a `MetricsSource` for the unified report.
pub struct CompilerMetricsSource<'a> {
    pub diagnostics: &'a PipelineDiagnostics,
}

impl MetricsSource for CompilerMetricsSource<'_> {
    fn section_name(&self) -> &'static str {
        metrics_keys::SECTION_COMPILER
    }

    fn collect(&self) -> serde_json::Value {
        self.diagnostics.to_json()
    }
}

impl Default for PipelineDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}
