//! Pipeline builder for the passes.

use super::PassManager;
use apxm_core::error::compiler::Result;
use apxm_core::types::OptimizationLevel;

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
        OptimizationLevel::O1 | OptimizationLevel::O2 | OptimizationLevel::O3 => {
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
    }
    Ok(())
}
