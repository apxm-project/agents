//! Pipeline builder for the passes.
//!
//! Optimization levels:
//!   O0 - No optimization (passthrough)
//!   O1 - Basic: normalize, build-prompt, scheduling, fusion, canonicalization, CSE, DCE
//!   O2 - Standard: O1 + template specialization, dead context elimination, schema narrowing, condense-ops
//!   O3 - Aggressive: O2 passes iterated to fixed-point convergence

use super::PassManager;
use apxm_core::error::compiler::Result;
use apxm_core::types::OptimizationLevel;

/// Maximum iterations for O3 fixed-point convergence.
const MAX_CONVERGENCE_ITERATIONS: usize = 10;

pub fn build_pipeline(pm: &mut PassManager, level: OptimizationLevel) -> Result<()> {
    build_pipeline_with_config(pm, level, false)
}

pub fn build_pipeline_with_config(
    pm: &mut PassManager,
    level: OptimizationLevel,
    no_cse_llm: bool,
) -> Result<()> {
    for name in build_pass_list(level, no_cse_llm) {
        pm.add_pass(&name)?;
    }
    Ok(())
}

/// Return the ordered list of pass names for a given optimization level and config.
///
/// This is the single source of truth for pipeline composition. Both
/// [`build_pipeline_with_config`] (which feeds passes to the MLIR pass manager) and
/// [`PassManager::run_with_metrics`] (which runs passes individually for diagnostics)
/// derive their pass sequence from this function.
pub fn build_pass_list(level: OptimizationLevel, no_cse_llm: bool) -> Vec<String> {
    let mut passes = Vec::new();

    match level {
        OptimizationLevel::O0 => {
            // No optimization passes at O0
        }
        OptimizationLevel::O1 => {
            passes.extend([
                "normalize",
                "build-prompt",
                "unconsumed-value-warning",
                "scheduling",
                "fuse-ask-ops",
                "canonicalizer",
            ].iter().map(|s| s.to_string()));
            if !no_cse_llm {
                passes.push("cse".to_string());
            }
            passes.push("symbol-dce".to_string());
        }
        OptimizationLevel::O2 => {
            passes.extend([
                "normalize",
                "build-prompt",
                "template-specialization",
                "unconsumed-value-warning",
                "schema-narrowing",
                "scheduling",
                "fuse-ask-ops",
                "condense-ops",
                "dead-context-elimination",
                "canonicalizer",
            ].iter().map(|s| s.to_string()));
            if !no_cse_llm {
                passes.push("cse".to_string());
            }
            passes.push("symbol-dce".to_string());
        }
        OptimizationLevel::O3 => {
            passes.extend([
                "normalize",
                "build-prompt",
                "unconsumed-value-warning",
            ].iter().map(|s| s.to_string()));
            for _ in 0..MAX_CONVERGENCE_ITERATIONS {
                passes.extend([
                    "template-specialization",
                    "schema-narrowing",
                    "scheduling",
                    "fuse-ask-ops",
                    "condense-ops",
                    "dead-context-elimination",
                    "canonicalizer",
                ].iter().map(|s| s.to_string()));
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
        let passes = build_pass_list(OptimizationLevel::O0, false);
        assert!(passes.is_empty());
    }

    #[test]
    fn o1_pass_list_matches_spec() {
        let passes = build_pass_list(OptimizationLevel::O1, false);
        assert_eq!(passes.len(), 8);
        assert_eq!(passes[0], "normalize");
        assert_eq!(passes[1], "build-prompt");
        assert_eq!(passes.last().unwrap(), "symbol-dce");
        assert!(passes.contains(&"cse".to_string()));
    }

    #[test]
    fn o1_no_cse_llm_skips_cse() {
        let passes = build_pass_list(OptimizationLevel::O1, true);
        assert_eq!(passes.len(), 7);
        assert!(!passes.contains(&"cse".to_string()));
    }

    #[test]
    fn o2_pass_list_matches_spec() {
        let passes = build_pass_list(OptimizationLevel::O2, false);
        assert_eq!(passes.len(), 12);
        assert!(passes.contains(&"template-specialization".to_string()));
        assert!(passes.contains(&"dead-context-elimination".to_string()));
        assert!(passes.contains(&"schema-narrowing".to_string()));
        assert!(passes.contains(&"condense-ops".to_string()));
    }

    #[test]
    fn o3_iterates_convergence_loop() {
        let passes = build_pass_list(OptimizationLevel::O3, false);
        // 3 initial + 10 * 9 convergence passes = 93
        assert_eq!(passes.len(), 93);
        // First three are the preamble
        assert_eq!(passes[0], "normalize");
        assert_eq!(passes[1], "build-prompt");
        assert_eq!(passes[2], "unconsumed-value-warning");
        // Then convergence iterations start
        assert_eq!(passes[3], "template-specialization");
    }
}
