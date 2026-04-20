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
use super::bind_tool_handlers::BIND_TOOL_HANDLERS_PASS_NAME;
use super::tool_binding::TOOL_BINDING_PASS_NAME;
use super::vllm_hints::VLLM_HINTS_PASS_NAME;
use apxm_ais::passes;
use apxm_core::error::compiler::Result;
use apxm_core::types::{OptimizationLevel, OptimizationTarget};

/// Maximum iterations for O3 fixed-point convergence.
const MAX_CONVERGENCE_ITERATIONS: usize = 10;

// Short aliases for pass names — single source of truth from apxm_ais::passes.
const NORMALIZE: &str = passes::NORMALIZE.name;
const BUILD_PROMPT: &str = passes::BUILD_PROMPT.name;
const DSPY_OPTIMIZE: &str = passes::DSPY_OPTIMIZE.name;
const ASSIGN_PRIORITY: &str = passes::ASSIGN_PRIORITY.name;
const SCHEDULING: &str = passes::SCHEDULING.name;
const FUSE_ASK_OPS: &str = passes::FUSE_ASK_OPS.name;
const CONDENSE_OPS: &str = passes::CONDENSE_OPS.name;
const UNCONSUMED_VALUE_WARNING: &str = passes::UNCONSUMED_VALUE_WARNING.name;
const TEMPLATE_SPECIALIZATION: &str = passes::TEMPLATE_SPECIALIZATION.name;
const DEAD_CONTEXT_ELIMINATION: &str = passes::DEAD_CONTEXT_ELIMINATION.name;
const SCHEMA_NARROWING: &str = passes::SCHEMA_NARROWING.name;
const PROMPT_CANONICALIZATION: &str = passes::PROMPT_CANONICALIZATION.name;
const CANONICALIZER: &str = passes::CANONICALIZER.name;
const CSE: &str = passes::CSE.name;
const SYMBOL_DCE: &str = passes::SYMBOL_DCE.name;

/// Rust-only post-MLIR pass that stamps `_vllm_*` hint attrs onto LLM nodes.
/// Filtered out before being handed to the MLIR PassManager (see
/// [`build_pipeline_with_config`]); appears in [`build_pass_list`] purely so
/// downstream callers (diagnostics, ordering tests) see it in pipeline order.
const VLLM_HINTS: &str = VLLM_HINTS_PASS_NAME;

/// Rust-only validation pass that checks tool capability bindings.
/// Runs after the canonicalizer to validate INV_TOOL/REGISTER_CAPABILITY
/// consistency.
const TOOL_BINDING: &str = TOOL_BINDING_PASS_NAME;

/// Rust-only pass that copies `python_handler_id` from REGISTER_CAPABILITY
/// onto matching INV_TOOL nodes. Runs after tool-binding-check.
const BIND_TOOL_HANDLERS: &str = BIND_TOOL_HANDLERS_PASS_NAME;

/// Names that are tracked in the pipeline list but are *not* dispatched
/// through the MLIR PassManager — they run as Rust-side transforms on the
/// `AirModule` instead.
const RUST_ONLY_PASSES: &[&str] = &[VLLM_HINTS, TOOL_BINDING, BIND_TOOL_HANDLERS];

fn is_mlir_pass(name: &str) -> bool {
    !RUST_ONLY_PASSES.contains(&name)
}

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
        if is_mlir_pass(&name) {
            pm.add_pass(&name)?;
        }
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
                    NORMALIZE,
                    BUILD_PROMPT,
                    DSPY_OPTIMIZE,
                    UNCONSUMED_VALUE_WARNING,
                    SCHEDULING,
                    FUSE_ASK_OPS,
                    ASSIGN_PRIORITY,
                    VLLM_HINTS,
                    CANONICALIZER,
                    TOOL_BINDING,
                    BIND_TOOL_HANDLERS,
                ]
                .iter()
                .map(|s| s.to_string()),
            );

            // Target-specific adjustments for O1
            if matches!(
                target,
                OptimizationTarget::Cost | OptimizationTarget::Tokens
            ) {
                // Add more aggressive DCE for cost/tokens targets
                passes.insert(passes.len() - 3, DEAD_CONTEXT_ELIMINATION.to_string());
            }

            if !no_cse_llm {
                passes.push(CSE.to_string());
            }
            passes.push(SYMBOL_DCE.to_string());
        }
        OptimizationLevel::O2 => {
            passes.extend(
                [
                    NORMALIZE,
                    BUILD_PROMPT,
                    DSPY_OPTIMIZE,
                    PROMPT_CANONICALIZATION,
                    TEMPLATE_SPECIALIZATION,
                    UNCONSUMED_VALUE_WARNING,
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
                            DEAD_CONTEXT_ELIMINATION,
                            SCHEMA_NARROWING,
                            SCHEDULING,
                            FUSE_ASK_OPS,
                            CONDENSE_OPS,
                            ASSIGN_PRIORITY,
                            VLLM_HINTS,
                            CANONICALIZER,
                            TOOL_BINDING,
                            BIND_TOOL_HANDLERS,
                        ]
                        .iter()
                        .map(|s| s.to_string()),
                    );
                }
                OptimizationTarget::Cost => {
                    // Prioritize CSE and dead code elimination
                    passes.extend(
                        [
                            SCHEMA_NARROWING,
                            SCHEDULING,
                            FUSE_ASK_OPS,
                            CONDENSE_OPS,
                            ASSIGN_PRIORITY,
                            VLLM_HINTS,
                            DEAD_CONTEXT_ELIMINATION,
                            CANONICALIZER,
                            TOOL_BINDING,
                            BIND_TOOL_HANDLERS,
                        ]
                        .iter()
                        .map(|s| s.to_string()),
                    );
                }
                OptimizationTarget::Latency | OptimizationTarget::Parallelism => {
                    // Prioritize fusion and scheduling
                    passes.extend(
                        [
                            SCHEDULING,
                            SCHEMA_NARROWING,
                            FUSE_ASK_OPS,
                            CONDENSE_OPS,
                            ASSIGN_PRIORITY,
                            VLLM_HINTS,
                            DEAD_CONTEXT_ELIMINATION,
                            CANONICALIZER,
                            TOOL_BINDING,
                            BIND_TOOL_HANDLERS,
                        ]
                        .iter()
                        .map(|s| s.to_string()),
                    );
                }
                OptimizationTarget::Balanced => {
                    // Default ordering
                    passes.extend(
                        [
                            SCHEMA_NARROWING,
                            SCHEDULING,
                            FUSE_ASK_OPS,
                            CONDENSE_OPS,
                            ASSIGN_PRIORITY,
                            VLLM_HINTS,
                            DEAD_CONTEXT_ELIMINATION,
                            CANONICALIZER,
                            TOOL_BINDING,
                            BIND_TOOL_HANDLERS,
                        ]
                        .iter()
                        .map(|s| s.to_string()),
                    );
                }
            }

            if !no_cse_llm {
                passes.push(CSE.to_string());
            }
            passes.push(SYMBOL_DCE.to_string());
        }
        OptimizationLevel::O3 => {
            passes.extend(
                [
                    NORMALIZE,
                    BUILD_PROMPT,
                    DSPY_OPTIMIZE,
                    PROMPT_CANONICALIZATION,
                    UNCONSUMED_VALUE_WARNING,
                ]
                .iter()
                .map(|s| s.to_string()),
            );

            let convergence_passes: Vec<String> = match target {
                OptimizationTarget::Tokens => vec![
                    DEAD_CONTEXT_ELIMINATION,
                    TEMPLATE_SPECIALIZATION,
                    SCHEMA_NARROWING,
                    SCHEDULING,
                    FUSE_ASK_OPS,
                    CONDENSE_OPS,
                    ASSIGN_PRIORITY,
                    VLLM_HINTS,
                    CANONICALIZER,
                    TOOL_BINDING,
                    BIND_TOOL_HANDLERS,
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                OptimizationTarget::Latency | OptimizationTarget::Parallelism => vec![
                    SCHEDULING,
                    TEMPLATE_SPECIALIZATION,
                    SCHEMA_NARROWING,
                    FUSE_ASK_OPS,
                    CONDENSE_OPS,
                    ASSIGN_PRIORITY,
                    VLLM_HINTS,
                    DEAD_CONTEXT_ELIMINATION,
                    CANONICALIZER,
                    TOOL_BINDING,
                    BIND_TOOL_HANDLERS,
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                _ => vec![
                    TEMPLATE_SPECIALIZATION,
                    SCHEMA_NARROWING,
                    SCHEDULING,
                    FUSE_ASK_OPS,
                    CONDENSE_OPS,
                    ASSIGN_PRIORITY,
                    VLLM_HINTS,
                    DEAD_CONTEXT_ELIMINATION,
                    CANONICALIZER,
                    TOOL_BINDING,
                    BIND_TOOL_HANDLERS,
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            };

            for _ in 0..MAX_CONVERGENCE_ITERATIONS {
                passes.extend(convergence_passes.clone());
                if !no_cse_llm {
                    passes.push(CSE.to_string());
                }
                passes.push(SYMBOL_DCE.to_string());
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
        // Check structural ordering, not exact count
        assert_eq!(passes[0], NORMALIZE);
        assert_eq!(passes[1], BUILD_PROMPT);
        assert_eq!(passes[2], DSPY_OPTIMIZE);
        assert!(passes.contains(&ASSIGN_PRIORITY.to_string()));
        assert_eq!(passes.last().unwrap(), SYMBOL_DCE);
        assert!(passes.contains(&CSE.to_string()));
        assert!(passes.contains(&FUSE_ASK_OPS.to_string()));
    }

    #[test]
    fn o1_no_cse_llm_skips_cse() {
        let passes = build_pass_list(OptimizationLevel::O1, true, OptimizationTarget::Balanced);
        assert!(!passes.contains(&CSE.to_string()));
        // Other passes still present
        assert!(passes.contains(&FUSE_ASK_OPS.to_string()));
        assert!(passes.contains(&NORMALIZE.to_string()));
    }

    #[test]
    fn o2_pass_list_matches_spec() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Balanced);
        assert!(passes.contains(&DSPY_OPTIMIZE.to_string()));
        assert!(passes.contains(&ASSIGN_PRIORITY.to_string()));
        assert!(passes.contains(&PROMPT_CANONICALIZATION.to_string()));
        assert!(passes.contains(&TEMPLATE_SPECIALIZATION.to_string()));
        assert!(passes.contains(&DEAD_CONTEXT_ELIMINATION.to_string()));
        assert!(passes.contains(&SCHEMA_NARROWING.to_string()));
        assert!(passes.contains(&CONDENSE_OPS.to_string()));
    }

    #[test]
    fn o3_iterates_convergence_loop() {
        let passes = build_pass_list(OptimizationLevel::O3, false, OptimizationTarget::Balanced);
        // Check preamble ordering
        assert_eq!(passes[0], NORMALIZE);
        assert_eq!(passes[1], BUILD_PROMPT);
        assert_eq!(passes[2], DSPY_OPTIMIZE);
        assert!(passes.contains(&ASSIGN_PRIORITY.to_string()));
        assert!(passes.contains(&UNCONSUMED_VALUE_WARNING.to_string()));
        // Convergence loop produces many more passes than O2
        let o2_passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Balanced);
        assert!(passes.len() > o2_passes.len() * 3);
    }

    #[test]
    fn test_target_latency_enables_fusion() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Latency);
        // Scheduling should come before fusion for latency target
        let scheduling_idx = passes.iter().position(|p| p == SCHEDULING).unwrap();
        let fusion_idx = passes.iter().position(|p| p == FUSE_ASK_OPS).unwrap();
        assert!(scheduling_idx < fusion_idx);
        assert!(passes.contains(&FUSE_ASK_OPS.to_string()));
    }

    #[test]
    fn test_target_cost_enables_cse() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Cost);
        assert!(passes.contains(&CSE.to_string()));
        assert!(passes.contains(&DEAD_CONTEXT_ELIMINATION.to_string()));
    }

    #[test]
    fn test_target_tokens_enables_dce() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Tokens);
        // Dead context elimination should come before scheduling for tokens target
        let dce_idx = passes
            .iter()
            .position(|p| p == DEAD_CONTEXT_ELIMINATION)
            .unwrap();
        let scheduling_idx = passes.iter().position(|p| p == SCHEDULING).unwrap();
        assert!(dce_idx < scheduling_idx);
    }

    #[test]
    fn test_balanced_matches_default() {
        let balanced = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Balanced);
        // Check structural ordering, not exact count
        assert_eq!(balanced[0], NORMALIZE);
        assert_eq!(balanced[1], BUILD_PROMPT);
        assert_eq!(balanced[2], DSPY_OPTIMIZE);
        assert!(balanced.contains(&ASSIGN_PRIORITY.to_string()));
        assert!(balanced.contains(&PROMPT_CANONICALIZATION.to_string()));
        assert!(balanced.contains(&SCHEMA_NARROWING.to_string()));
    }

    #[test]
    fn assign_priority_runs_after_fusion() {
        for target in [
            OptimizationTarget::Balanced,
            OptimizationTarget::Latency,
            OptimizationTarget::Cost,
            OptimizationTarget::Tokens,
        ] {
            for level in [OptimizationLevel::O1, OptimizationLevel::O2] {
                let passes = build_pass_list(level, false, target);
                let fusion_idx = passes.iter().position(|p| p == FUSE_ASK_OPS).unwrap();
                let priority_idx = passes.iter().position(|p| p == ASSIGN_PRIORITY).unwrap();
                assert!(
                    priority_idx > fusion_idx,
                    "ASSIGN_PRIORITY must run after FUSE_ASK_OPS at {level:?}/{target:?}"
                );
            }
        }
    }

    #[test]
    fn vllm_hints_runs_at_o1_plus_after_assign_priority() {
        for target in [
            OptimizationTarget::Balanced,
            OptimizationTarget::Latency,
            OptimizationTarget::Cost,
            OptimizationTarget::Tokens,
        ] {
            for level in [
                OptimizationLevel::O1,
                OptimizationLevel::O2,
                OptimizationLevel::O3,
            ] {
                let passes = build_pass_list(level, false, target);
                let priority_idx = passes
                    .iter()
                    .position(|p| p == ASSIGN_PRIORITY)
                    .expect("ASSIGN_PRIORITY must appear at O1+");
                let vllm_idx = passes
                    .iter()
                    .position(|p| p == VLLM_HINTS)
                    .unwrap_or_else(|| {
                        panic!("VLLM_HINTS missing from O1+ pipeline at {level:?}/{target:?}")
                    });
                assert!(
                    vllm_idx > priority_idx,
                    "VLLM_HINTS must run after ASSIGN_PRIORITY at {level:?}/{target:?}"
                );
            }
        }
    }

    #[test]
    fn o0_does_not_emit_vllm_hints() {
        let passes = build_pass_list(OptimizationLevel::O0, false, OptimizationTarget::Balanced);
        assert!(!passes.contains(&VLLM_HINTS.to_string()));
    }

    #[test]
    fn rust_only_passes_are_filtered_from_mlir_dispatch() {
        // Sanity: Rust-only passes must not look like MLIR passes.
        assert!(!is_mlir_pass(VLLM_HINTS));
        assert!(!is_mlir_pass(TOOL_BINDING));
        assert!(!is_mlir_pass(BIND_TOOL_HANDLERS));
        // All other pass names should still be MLIR-dispatched.
        for n in [
            NORMALIZE,
            BUILD_PROMPT,
            ASSIGN_PRIORITY,
            CANONICALIZER,
            CSE,
            SYMBOL_DCE,
        ] {
            assert!(is_mlir_pass(n), "{n} should be an MLIR pass");
        }
    }

    #[test]
    fn tool_binding_runs_after_canonicalizer_at_o1_plus() {
        for target in [
            OptimizationTarget::Balanced,
            OptimizationTarget::Latency,
            OptimizationTarget::Cost,
            OptimizationTarget::Tokens,
        ] {
            for level in [
                OptimizationLevel::O1,
                OptimizationLevel::O2,
                OptimizationLevel::O3,
            ] {
                let passes = build_pass_list(level, false, target);
                let canon_idx = passes
                    .iter()
                    .position(|p| p == CANONICALIZER)
                    .expect("CANONICALIZER must appear at O1+");
                let tb_idx = passes
                    .iter()
                    .position(|p| p == TOOL_BINDING)
                    .unwrap_or_else(|| {
                        panic!("TOOL_BINDING missing from pipeline at {level:?}/{target:?}")
                    });
                assert!(
                    tb_idx > canon_idx,
                    "TOOL_BINDING must run after CANONICALIZER at {level:?}/{target:?}"
                );
            }
        }
    }

    #[test]
    fn bind_tool_handlers_runs_after_tool_binding_at_o1_plus() {
        for target in [
            OptimizationTarget::Balanced,
            OptimizationTarget::Latency,
            OptimizationTarget::Cost,
            OptimizationTarget::Tokens,
        ] {
            for level in [
                OptimizationLevel::O1,
                OptimizationLevel::O2,
                OptimizationLevel::O3,
            ] {
                let passes = build_pass_list(level, false, target);
                let tb_idx = passes
                    .iter()
                    .position(|p| p == TOOL_BINDING)
                    .expect("TOOL_BINDING must appear at O1+");
                let bth_idx = passes
                    .iter()
                    .position(|p| p == BIND_TOOL_HANDLERS)
                    .unwrap_or_else(|| {
                        panic!(
                            "BIND_TOOL_HANDLERS missing from pipeline at {level:?}/{target:?}"
                        )
                    });
                assert!(
                    bth_idx > tb_idx,
                    "BIND_TOOL_HANDLERS must run after TOOL_BINDING at {level:?}/{target:?}"
                );
            }
        }
    }
}
