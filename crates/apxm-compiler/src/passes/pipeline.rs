//! Pipeline builder for the passes.
//!
//! Optimization levels:
//!   O0 - No optimization (passthrough)
//!   O1 - Basic: normalize, build-prompt, scheduling, fusion, canonicalization, CSE, DCE
//!   O2 - Standard: O1 + template specialization, dead context elimination, schema narrowing, condense-ops
//!   O3 - Aggressive: O2 passes iterated to fixed-point convergence
//!
//! NOTE: Currently, the C++ FuseAskOps pass uses a default maxTemplateTokens of 2000.
//! To make this target-aware, we need to extend the FFI to accept pass options.

use super::PassManager;
use apxm_core::error::compiler::Result;
use apxm_core::types::{OptimizationLevel, OptimizationTarget};

/// Maximum iterations for O3 fixed-point convergence.
const MAX_CONVERGENCE_ITERATIONS: usize = 10;

pub fn build_pipeline(pm: &mut PassManager, level: OptimizationLevel) -> Result<()> {
    build_pipeline_with_config(pm, level, false, OptimizationTarget::Balanced)
}

pub fn build_pipeline_with_config(
    pm: &mut PassManager,
    level: OptimizationLevel,
    no_cse_llm: bool,
    target: OptimizationTarget,
) -> Result<()> {
    for name in build_pass_list(level, no_cse_llm, target) {
        pm.add_pass(&name)?;
    }
    Ok(())
}

/// Return the ordered list of pass names for a given optimization level, config, and target.
///
/// This is the single source of truth for pipeline composition. Both
/// [`build_pipeline_with_config`] (which feeds passes to the MLIR pass manager) and
/// [`PassManager::run_with_metrics`] (which runs passes individually for diagnostics)
/// derive their pass sequence from this function.
///
/// The `target` parameter controls what to optimize for:
/// - `Latency`: More aggressive fusion, prioritize parallel scheduling
/// - `Cost`: More aggressive CSE and dead code elimination
/// - `Tokens`: Prioritize dead-context-elimination and schema-narrowing
/// - `Parallelism`: Aggressive scheduling, remove sequential constraints
/// - `Balanced`: Default behavior (no special tuning)
///
pub fn build_pass_list(
    level: OptimizationLevel,
    no_cse_llm: bool,
    target: OptimizationTarget,
) -> Vec<String> {
    let mut passes = Vec::new();

    match level {
        OptimizationLevel::O0 => {
            // No optimization passes at O0
        }
        OptimizationLevel::O1 => {
            passes.extend(
                [
                    "normalize",
                    "build-prompt",
                    "assign-priority",
                    "unconsumed-value-warning",
                    "scheduling",
                    "fuse-ask-ops",
                    "canonicalizer",
                ]
                .iter()
                .map(|s| s.to_string()),
            );

            // Target-specific adjustments for O1
            if matches!(target, OptimizationTarget::Cost | OptimizationTarget::Tokens) {
                // Add more aggressive DCE for cost/tokens targets
                passes.insert(passes.len() - 1, "dead-context-elimination".to_string());
            }

            if !no_cse_llm {
                passes.push("cse".to_string());
            }
            passes.push("symbol-dce".to_string());
        }
        OptimizationLevel::O2 => {
            passes.extend(
                [
                    "normalize",
                    "build-prompt",
                    "assign-priority",
                    "prompt-canonicalization",
                    "template-specialization",
                    "unconsumed-value-warning",
                ]
                .iter()
                .map(|s| s.to_string()),
            );

            // Target-specific pass ordering for O2
            match target {
                OptimizationTarget::Tokens => {
                    // Prioritize context reduction
                    passes.extend(
                        [
                            "dead-context-elimination",
                            "schema-narrowing",
                            "scheduling",
                            "fuse-ask-ops",
                            "condense-ops",
                            "canonicalizer",
                        ]
                        .iter()
                        .map(|s| s.to_string()),
                    );
                }
                OptimizationTarget::Cost => {
                    // Prioritize CSE and dead code elimination
                    passes.extend(
                        [
                            "schema-narrowing",
                            "scheduling",
                            "fuse-ask-ops",
                            "condense-ops",
                            "dead-context-elimination",
                            "canonicalizer",
                        ]
                        .iter()
                        .map(|s| s.to_string()),
                    );
                }
                OptimizationTarget::Latency | OptimizationTarget::Parallelism => {
                    // Prioritize fusion and scheduling
                    passes.extend(
                        [
                            "scheduling",
                            "schema-narrowing",
                            "fuse-ask-ops",
                            "condense-ops",
                            "dead-context-elimination",
                            "canonicalizer",
                        ]
                        .iter()
                        .map(|s| s.to_string()),
                    );
                }
                OptimizationTarget::Balanced => {
                    // Default ordering
                    passes.extend(
                        [
                            "schema-narrowing",
                            "scheduling",
                            "fuse-ask-ops",
                            "condense-ops",
                            "dead-context-elimination",
                            "canonicalizer",
                        ]
                        .iter()
                        .map(|s| s.to_string()),
                    );
                }
            }

            if !no_cse_llm {
                passes.push("cse".to_string());
            }
            passes.push("symbol-dce".to_string());
        }
        OptimizationLevel::O3 => {
            passes.extend(
                ["normalize", "build-prompt", "assign-priority", "prompt-canonicalization", "unconsumed-value-warning"]
                    .iter()
                    .map(|s| s.to_string()),
            );

            let convergence_passes: Vec<String> = match target {
                OptimizationTarget::Tokens => {
                    vec![
                        "dead-context-elimination",
                        "template-specialization",
                        "schema-narrowing",
                        "scheduling",
                        "fuse-ask-ops",
                        "condense-ops",
                        "canonicalizer",
                    ]
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
                }
                OptimizationTarget::Latency | OptimizationTarget::Parallelism => {
                    vec![
                        "scheduling",
                        "template-specialization",
                        "schema-narrowing",
                        "fuse-ask-ops",
                        "condense-ops",
                        "dead-context-elimination",
                        "canonicalizer",
                    ]
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
                }
                _ => {
                    vec![
                        "template-specialization",
                        "schema-narrowing",
                        "scheduling",
                        "fuse-ask-ops",
                        "condense-ops",
                        "dead-context-elimination",
                        "canonicalizer",
                    ]
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
                }
            };

            for _ in 0..MAX_CONVERGENCE_ITERATIONS {
                passes.extend(convergence_passes.clone());
                if !no_cse_llm {
                    passes.push("cse".to_string());
                }
                passes.push("symbol-dce".to_string());
            }
        }
    }

    passes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o0_produces_no_passes() {
        let passes = build_pass_list(OptimizationLevel::O0, false, OptimizationTarget::Balanced);
        assert!(passes.is_empty());
    }

    #[test]
    fn o1_pass_list_matches_spec() {
        let passes = build_pass_list(OptimizationLevel::O1, false, OptimizationTarget::Balanced);
        assert_eq!(passes.len(), 9);
        assert_eq!(passes[0], "normalize");
        assert_eq!(passes[1], "build-prompt");
        assert_eq!(passes[2], "assign-priority");
        assert_eq!(passes.last().unwrap(), "symbol-dce");
        assert!(passes.contains(&"cse".to_string()));
    }

    #[test]
    fn o1_no_cse_llm_skips_cse() {
        let passes = build_pass_list(OptimizationLevel::O1, true, OptimizationTarget::Balanced);
        assert_eq!(passes.len(), 8);
        assert!(!passes.contains(&"cse".to_string()));
    }

    #[test]
    fn o2_pass_list_matches_spec() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Balanced);
        assert_eq!(passes.len(), 14);
        assert!(passes.contains(&"assign-priority".to_string()));
        assert!(passes.contains(&"prompt-canonicalization".to_string()));
        assert!(passes.contains(&"template-specialization".to_string()));
        assert!(passes.contains(&"dead-context-elimination".to_string()));
        assert!(passes.contains(&"schema-narrowing".to_string()));
        assert!(passes.contains(&"condense-ops".to_string()));
    }

    #[test]
    fn o3_iterates_convergence_loop() {
        let passes = build_pass_list(OptimizationLevel::O3, false, OptimizationTarget::Balanced);
        // 5 initial + 10 * 9 convergence passes = 95
        assert_eq!(passes.len(), 95);
        // First five are the preamble
        assert_eq!(passes[0], "normalize");
        assert_eq!(passes[1], "build-prompt");
        assert_eq!(passes[2], "assign-priority");
        assert_eq!(passes[3], "prompt-canonicalization");
        assert_eq!(passes[4], "unconsumed-value-warning");
        // Then convergence iterations start
        assert_eq!(passes[5], "template-specialization");
    }

    #[test]
    fn test_target_latency_enables_fusion() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Latency);
        assert_eq!(passes.len(), 14);
        // Scheduling should come early for latency target
        let scheduling_idx = passes.iter().position(|p| p == "scheduling").unwrap();
        let fusion_idx = passes.iter().position(|p| p == "fuse-ask-ops").unwrap();
        assert!(scheduling_idx < fusion_idx);
        assert!(passes.contains(&"fuse-ask-ops".to_string()));
    }

    #[test]
    fn test_target_cost_enables_cse() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Cost);
        assert_eq!(passes.len(), 14);
        // CSE should be present for cost target
        assert!(passes.contains(&"cse".to_string()));
        assert!(passes.contains(&"dead-context-elimination".to_string()));
    }

    #[test]
    fn test_target_tokens_enables_dce() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Tokens);
        assert_eq!(passes.len(), 14);
        // Dead context elimination should come early for tokens target
        let dce_idx = passes
            .iter()
            .position(|p| p == "dead-context-elimination")
            .unwrap();
        let scheduling_idx = passes.iter().position(|p| p == "scheduling").unwrap();
        assert!(dce_idx < scheduling_idx);
    }

    #[test]
    fn test_balanced_matches_default() {
        let balanced = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Balanced);
        assert_eq!(balanced.len(), 14);
        // Balanced should have standard ordering
        assert_eq!(balanced[0], "normalize");
        assert_eq!(balanced[1], "build-prompt");
        assert_eq!(balanced[2], "assign-priority");
        assert_eq!(balanced[3], "prompt-canonicalization");
        assert!(balanced.contains(&"schema-narrowing".to_string()));
    }
}
