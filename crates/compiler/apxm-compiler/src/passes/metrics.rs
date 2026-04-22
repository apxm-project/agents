//! Per-pass compiler diagnostics.
//!
//! [`PassMetrics`] captures timing and IR-size data for a single compiler pass.
//! [`PipelineDiagnostics`] aggregates metrics across an entire pipeline run.

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
    fn empty_diagnostics() {
        let diag = PipelineDiagnostics::new();
        assert_eq!(diag.pass_count(), 0);
        assert_eq!(diag.total_ops_eliminated(), 0);
        assert!(diag.active_passes().is_empty());
    }

    #[test]
    fn diagnostics_aggregation() {
        let diag = PipelineDiagnostics {
            passes: vec![
                PassMetrics {
                    pass_name: "normalize".into(),
                    duration_ms: 1.5,
                    ops_before: 10,
                    ops_after: 10,
                    ops_delta: 0,
                    fired_count: 0,
                    ir_size_delta: 0,
                    tokens_saved: None,
                },
                PassMetrics {
                    pass_name: "fuse-ask-ops".into(),
                    duration_ms: 2.3,
                    ops_before: 10,
                    ops_after: 7,
                    ops_delta: -3,
                    fired_count: 0,
                    ir_size_delta: 0,
                    tokens_saved: None,
                },
                PassMetrics {
                    pass_name: "symbol-dce".into(),
                    duration_ms: 0.8,
                    ops_before: 7,
                    ops_after: 5,
                    ops_delta: -2,
                    fired_count: 0,
                    ir_size_delta: 0,
                    tokens_saved: None,
                },
            ],
            total_duration_ms: 4.6,
            initial_ops: 10,
            final_ops: 5,
        };

        assert_eq!(diag.pass_count(), 3);
        assert_eq!(diag.total_ops_eliminated(), 5);
        assert_eq!(diag.active_passes(), vec!["fuse-ask-ops", "symbol-dce"]);
    }

    #[test]
    fn pass_metrics_serialize_roundtrip() {
        let metrics = PassMetrics {
            pass_name: "canonicalizer".into(),
            duration_ms: 3.14,
            ops_before: 20,
            ops_after: 18,
            ops_delta: -2,
            fired_count: 0,
            ir_size_delta: 0,
            tokens_saved: None,
        };
        let json = serde_json::to_string(&metrics).unwrap();
        let deserialized: PassMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pass_name, "canonicalizer");
        assert_eq!(deserialized.ops_delta, -2);
    }

    #[test]
    fn pipeline_diagnostics_serialize_roundtrip() {
        let diag = PipelineDiagnostics {
            passes: vec![PassMetrics {
                pass_name: "cse".into(),
                duration_ms: 1.0,
                ops_before: 5,
                ops_after: 4,
                ops_delta: -1,
                fired_count: 0,
                ir_size_delta: 0,
                tokens_saved: None,
            }],
            total_duration_ms: 1.0,
            initial_ops: 5,
            final_ops: 4,
        };
        let json = serde_json::to_string_pretty(&diag).unwrap();
        let deserialized: PipelineDiagnostics = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pass_count(), 1);
        assert_eq!(deserialized.final_ops, 4);
    }

    #[test]
    fn pass_metrics_carries_ablation_fields() {
        let m = PassMetrics {
            pass_name: "fuse-ask-ops".into(),
            duration_ms: 2.0,
            ops_before: 10,
            ops_after: 7,
            ops_delta: -3,
            fired_count: 3,
            ir_size_delta: -42,
            tokens_saved: Some(180),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: PassMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(back.fired_count, 3);
        assert_eq!(back.ir_size_delta, -42);
        assert_eq!(back.tokens_saved, Some(180));
    }

    #[test]
    fn pipeline_diagnostics_total_tokens_saved() {
        let diag = PipelineDiagnostics {
            passes: vec![
                PassMetrics {
                    pass_name: "fuse-ask-ops".into(),
                    duration_ms: 1.0,
                    ops_before: 10,
                    ops_after: 7,
                    ops_delta: -3,
                    fired_count: 3,
                    ir_size_delta: -40,
                    tokens_saved: Some(180),
                },
                PassMetrics {
                    pass_name: "dead-context-elimination".into(),
                    duration_ms: 1.0,
                    ops_before: 7,
                    ops_after: 7,
                    ops_delta: 0,
                    fired_count: 2,
                    ir_size_delta: -10,
                    tokens_saved: Some(60),
                },
                PassMetrics {
                    pass_name: "canonicalizer".into(),
                    duration_ms: 1.0,
                    ops_before: 7,
                    ops_after: 7,
                    ops_delta: 0,
                    fired_count: 0,
                    ir_size_delta: 0,
                    tokens_saved: None,
                },
            ],
            total_duration_ms: 3.0,
            initial_ops: 10,
            final_ops: 7,
        };
        assert_eq!(diag.total_tokens_saved(), 240);
        assert_eq!(
            diag.fired_passes(),
            vec!["fuse-ask-ops", "dead-context-elimination"]
        );
    }
}
