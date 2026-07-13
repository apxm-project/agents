//! Per-pass compiler diagnostics.
//!
//! [`PassMetrics`] captures timing and IR-size data for a single compiler pass.
//! [`PipelineDiagnostics`] aggregates metrics across an entire pipeline run.

use super::plan::PipelineStageKind;
use apxm_core::constants::session::metrics_keys;
use apxm_core::metrics::MetricsSource;
use serde::{Deserialize, Serialize};

/// The outcome recorded for one planned stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStageStatus {
    /// The stage ran at its owning execution boundary.
    #[default]
    Executed,
    /// The stage belongs to artifact generation and has not run during MLIR compilation.
    Deferred,
}

/// The bounded-convergence result for one repeated stage group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvergenceStatus {
    /// The group reached a fixed point before consuming its bound.
    Converged,
    /// The group consumed its configured bound while the module still changed.
    IterationLimitReached,
}

/// Diagnostics for one bounded-convergence group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvergenceDiagnostics {
    /// Stable convergence-group identifier.
    pub group_name: String,
    /// Number of iterations that executed.
    pub iterations: usize,
    /// Maximum iterations permitted by the plan.
    pub max_iterations: usize,
    /// Whether execution reached a fixed point or its configured bound.
    pub status: ConvergenceStatus,
}

/// Metrics collected for a single planned compiler stage execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassMetrics {
    /// Stable stage name (for MLIR stages, the registered pass name).
    pub pass_name: String,
    /// The boundary responsible for this stage.
    #[serde(default)]
    pub stage_kind: PipelineStageKind,
    /// One-based O3 convergence iteration, when this stage is repeated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<usize>,
    /// Whether the stage executed or was deferred to artifact generation.
    #[serde(default)]
    pub status: PipelineStageStatus,
    /// Whether configuration may remove this stage.
    #[serde(default)]
    pub mandatory: bool,
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
    /// Bounded-convergence results in execution order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub convergence: Vec<ConvergenceDiagnostics>,
}

impl PipelineDiagnostics {
    /// Create an empty diagnostics container.
    pub fn new() -> Self {
        Self {
            passes: Vec::new(),
            total_duration_ms: 0.0,
            initial_ops: 0,
            final_ops: 0,
            convergence: Vec::new(),
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

    /// Replace deferred artifact-stage entries with the metrics collected by
    /// artifact emission. A caller that compiled without diagnostics may add
    /// the metrics directly; either path records one truthful stage entry.
    pub fn record_artifact_stage_metrics(
        &mut self,
        metrics: impl IntoIterator<Item = PassMetrics>,
    ) {
        for metric in metrics {
            let duration_ms = metric.duration_ms;
            if let Some(deferred) = self.passes.iter_mut().find(|entry| {
                entry.status == PipelineStageStatus::Deferred
                    && entry.pass_name == metric.pass_name
                    && entry.stage_kind == metric.stage_kind
            }) {
                *deferred = metric;
            } else {
                self.passes.push(metric);
            }
            self.total_duration_ms += duration_ms;
        }
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
                metrics_keys::SUMMARY_ACTIVE_PASSES: self.active_passes(),
                "convergence": self.convergence
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_stage_boundary_iteration_and_status() {
        let diagnostics = PipelineDiagnostics {
            passes: vec![PassMetrics {
                pass_name: "bind-capability-handlers".to_string(),
                stage_kind: PipelineStageKind::ArtifactFinalization,
                iteration: Some(2),
                status: PipelineStageStatus::Deferred,
                mandatory: true,
                duration_ms: 0.0,
                ops_before: 3,
                ops_after: 3,
                ops_delta: 0,
                fired_count: 0,
                ir_size_delta: 0,
                tokens_saved: None,
            }],
            total_duration_ms: 0.0,
            initial_ops: 3,
            final_ops: 3,
            convergence: vec![ConvergenceDiagnostics {
                group_name: "o3-cleanup".to_string(),
                iterations: 2,
                max_iterations: 10,
                status: ConvergenceStatus::Converged,
            }],
        };

        let json = diagnostics.to_json();
        let stage = &json[metrics_keys::COMPILER_PASSES][0];
        assert_eq!(stage["stage_kind"], "artifact_finalization");
        assert_eq!(stage["iteration"], 2);
        assert_eq!(stage["status"], "deferred");
        assert_eq!(
            json[metrics_keys::COMPILER_SUMMARY]["convergence"][0]["status"],
            "converged"
        );
    }

    #[test]
    fn executed_artifact_metrics_replace_the_deferred_plan_entry() {
        let mut diagnostics = PipelineDiagnostics::new();
        diagnostics.passes.push(PassMetrics {
            pass_name: "refine-token-estimates".to_string(),
            stage_kind: PipelineStageKind::ArtifactFinalization,
            iteration: None,
            status: PipelineStageStatus::Deferred,
            mandatory: true,
            duration_ms: 0.0,
            ops_before: 2,
            ops_after: 2,
            ops_delta: 0,
            fired_count: 0,
            ir_size_delta: 0,
            tokens_saved: None,
        });

        diagnostics.record_artifact_stage_metrics([PassMetrics {
            pass_name: "refine-token-estimates".to_string(),
            stage_kind: PipelineStageKind::ArtifactFinalization,
            iteration: None,
            status: PipelineStageStatus::Executed,
            mandatory: true,
            duration_ms: 3.0,
            ops_before: 2,
            ops_after: 2,
            ops_delta: 0,
            fired_count: 0,
            ir_size_delta: 0,
            tokens_saved: None,
        }]);

        assert_eq!(diagnostics.passes.len(), 1);
        assert_eq!(diagnostics.passes[0].status, PipelineStageStatus::Executed);
        assert_eq!(diagnostics.total_duration_ms, 3.0);
    }
}
