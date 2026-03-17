//! Pipeline builder for the passes.
//!
//! Optimization levels:
//!   O0 - No optimization (passthrough)
//!   O1 - Basic: normalize, build-prompt, scheduling, fusion, canonicalization, CSE, DCE
//!   O2 - Standard: O1 + template specialization, dead context elimination, schema narrowing
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
    match level {
        OptimizationLevel::O0 => {
            // No optimization passes at O0
        }
        OptimizationLevel::O1 => {
            pm.normalize()?
                .build_prompt()?
                .unconsumed_value_warning()?
                .scheduling()?
                .fuse_ask_ops()?
                .canonicalizer()?;
            if !no_cse_llm {
                pm.cse()?;
            }
            pm.symbol_dce()?;
        }
        OptimizationLevel::O2 => {
            pm.normalize()?
                .build_prompt()?
                .template_specialization()?
                .unconsumed_value_warning()?
                .scheduling()?
                .fuse_ask_ops()?
                .dead_context_elimination()?
                .schema_narrowing()?
                .canonicalizer()?;
            if !no_cse_llm {
                pm.cse()?;
            }
            pm.symbol_dce()?;
        }
        OptimizationLevel::O3 => {
            pm.normalize()?.build_prompt()?.unconsumed_value_warning()?;
            for _ in 0..MAX_CONVERGENCE_ITERATIONS {
                pm.template_specialization()?
                    .scheduling()?
                    .fuse_ask_ops()?
                    .dead_context_elimination()?
                    .schema_narrowing()?
                    .canonicalizer()?;
                if !no_cse_llm {
                    pm.cse()?;
                }
                pm.symbol_dce()?;
            }
        }
    }
    Ok(())
}
