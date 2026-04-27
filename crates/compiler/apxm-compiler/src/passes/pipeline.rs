//! Pipeline builder for the passes.
//!
//! Optimization levels:
//!   O0 - No optimization (passthrough)
//!   O1 - Basic: normalize, build-prompt, safe cleanup, tool checks
//!   O2 - Standard: O1 + template specialization, dead context elimination,
//!        scheduling metadata, and shared-prefix analysis
//!   O3 - Aggressive: O2-safe passes iterated to fixed-point convergence

use super::PassManager;
use super::bind_tool_handlers::BIND_TOOL_HANDLERS_PASS_NAME;
use super::tool_binding::TOOL_BINDING_PASS_NAME;
use apxm_core::error::compiler::Result;
use apxm_core::types::compiler::metadata as passes;
use apxm_core::types::{OptimizationLevel, OptimizationTarget};

/// Maximum iterations for O3 fixed-point convergence.
const MAX_CONVERGENCE_ITERATIONS: usize = 10;

// Short aliases for pass names — downstream compiler consumers read these
// through apxm_core::types::compiler::metadata, which re-exports the canonical
// AIS authoring definitions.
const NORMALIZE: &str = passes::NORMALIZE.name;
const BUILD_PROMPT: &str = passes::BUILD_PROMPT.name;
#[cfg(test)]
const DSPY_OPTIMIZE: &str = passes::DSPY_OPTIMIZE.name;
const ASSIGN_PRIORITY: &str = passes::ASSIGN_PRIORITY.name;
const SCHEDULING: &str = passes::SCHEDULING.name;
const SHARED_PREFIX_ANALYSIS: &str = passes::SHARED_PREFIX_ANALYSIS.name;
#[cfg(test)]
const FUSE_ASK_OPS: &str = passes::FUSE_ASK_OPS.name;
#[cfg(test)]
const CONDENSE_OPS: &str = passes::CONDENSE_OPS.name;
const UNCONSUMED_VALUE_WARNING: &str = passes::UNCONSUMED_VALUE_WARNING.name;
const TEMPLATE_SPECIALIZATION: &str = passes::TEMPLATE_SPECIALIZATION.name;
const DEAD_CONTEXT_ELIMINATION: &str = passes::DEAD_CONTEXT_ELIMINATION.name;
#[cfg(test)]
const SCHEMA_NARROWING: &str = passes::SCHEMA_NARROWING.name;
#[cfg(test)]
const PROMPT_CANONICALIZATION: &str = passes::PROMPT_CANONICALIZATION.name;
const CANONICALIZER: &str = passes::CANONICALIZER.name;
const CSE: &str = passes::CSE.name;
const SYMBOL_DCE: &str = passes::SYMBOL_DCE.name;

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
const RUST_ONLY_PASSES: &[&str] = &[TOOL_BINDING, BIND_TOOL_HANDLERS];

pub fn is_mlir_pass(name: &str) -> bool {
    !RUST_ONLY_PASSES.contains(&name)
}

pub fn build_pipeline(pm: &mut PassManager, level: OptimizationLevel) -> Result<()> {
    build_pipeline_with_config(pm, level, false, OptimizationTarget::Balanced, false)
}

pub fn build_pipeline_with_config(
    pm: &mut PassManager,
    level: OptimizationLevel,
    no_cse_llm: bool,
    target: OptimizationTarget,
    warn: bool,
) -> Result<()> {
    for name in build_pass_list_with_warn(level, no_cse_llm, target, warn) {
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
/// The `target` parameter controls safe pass ordering only. Heuristic-free
/// semantic rewrites stay out of the automatic O-levels until their contracts
/// are typed and enforced by the compiler.
///
/// Passes intentionally excluded from default O1/O2/O3 pipelines:
/// - `fuse-ask-ops`: mutates ASK chains without semantic-quality heuristics.
/// - `condense-ops`: changes memory-query/write grouping without a typed
///   memory batching capability contract.
/// - `schema-narrowing`: current implementation is not real field-use
///   narrowing and can affect output validation.
/// - `prompt-canonicalization`: rewrites prompt layout for backend cache
///   behavior and needs an explicit backend/graph-hint contract.
/// - `cse`: generic MLIR CSE is not LLM-safe until deterministic/memoizable
///   contracts are typed. Use explicit pass lists for ablation only.
///
/// `dspy-optimize` is injected by the pipeline only when compiler prompt
/// tuning is explicitly configured. It is not part of this pure base list
/// because discovering that config must not make pass-list construction touch
/// backend credentials, training data, or compiler cache state.
///
pub fn build_pass_list(
    level: OptimizationLevel,
    _no_cse_llm: bool,
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
                    TEMPLATE_SPECIALIZATION,
                    DEAD_CONTEXT_ELIMINATION,
                    CANONICALIZER,
                    TOOL_BINDING,
                    BIND_TOOL_HANDLERS,
                ]
                .iter()
                .map(|s| s.to_string()),
            );

            passes.push(SYMBOL_DCE.to_string());
            passes.push(ASSIGN_PRIORITY.to_string());
        }
        OptimizationLevel::O2 => {
            passes.extend(
                [
                    NORMALIZE,
                    BUILD_PROMPT,
                    TEMPLATE_SPECIALIZATION,
                    DEAD_CONTEXT_ELIMINATION,
                ]
                .iter()
                .map(|s| s.to_string()),
            );

            // Target-specific pass ordering for O2
            match target {
                OptimizationTarget::Tokens => {
                    // Prioritize context reduction
                    passes.extend(
                        [CANONICALIZER, TOOL_BINDING, BIND_TOOL_HANDLERS]
                            .iter()
                            .map(|s| s.to_string()),
                    );
                }
                OptimizationTarget::Cost => {
                    // Prioritize safe dead-code cleanup.
                    passes.extend(
                        [CANONICALIZER, TOOL_BINDING, BIND_TOOL_HANDLERS]
                            .iter()
                            .map(|s| s.to_string()),
                    );
                }
                OptimizationTarget::Latency | OptimizationTarget::Parallelism => {
                    // Prioritize backend-agnostic graph scheduling. Shared-prefix
                    // analysis only emits metadata for prompts that are already
                    // prefix-compatible.
                    passes.extend(
                        [SCHEDULING, CANONICALIZER, TOOL_BINDING, BIND_TOOL_HANDLERS]
                            .iter()
                            .map(|s| s.to_string()),
                    );
                }
                OptimizationTarget::Balanced => {
                    // Default ordering
                    passes.extend(
                        [CANONICALIZER, TOOL_BINDING, BIND_TOOL_HANDLERS]
                            .iter()
                            .map(|s| s.to_string()),
                    );
                }
            }

            passes.push(SYMBOL_DCE.to_string());
            if !passes.iter().any(|pass| pass == SCHEDULING) {
                passes.push(SCHEDULING.to_string());
            }
            passes.push(SHARED_PREFIX_ANALYSIS.to_string());
            passes.push(ASSIGN_PRIORITY.to_string());
        }
        OptimizationLevel::O3 => {
            passes.extend(
                [
                    NORMALIZE,
                    BUILD_PROMPT,
                    TEMPLATE_SPECIALIZATION,
                    DEAD_CONTEXT_ELIMINATION,
                ]
                .iter()
                .map(|s| s.to_string()),
            );

            let convergence_passes: Vec<String> = match target {
                OptimizationTarget::Tokens => vec![
                    TEMPLATE_SPECIALIZATION,
                    DEAD_CONTEXT_ELIMINATION,
                    SCHEDULING,
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
                    DEAD_CONTEXT_ELIMINATION,
                    SCHEDULING,
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
                passes.push(SYMBOL_DCE.to_string());
            }
            if !passes.iter().any(|pass| pass == SCHEDULING) {
                passes.push(SCHEDULING.to_string());
            }
            passes.push(SHARED_PREFIX_ANALYSIS.to_string());
            passes.push(ASSIGN_PRIORITY.to_string());
        }
    }

    passes
}

/// Like [`build_pass_list`] but appends `unconsumed-value-warning` when `warn` is true.
///
/// The warning pass is purely diagnostic — it produces no IR mutation — so it is
/// always inserted at the very end of the pipeline regardless of opt level.
/// It is opt-in via the CLI `--warn` flag and never appears in the default O1/O2/O3
/// pipelines (see compiler-audit.md).
pub fn build_pass_list_with_warn(
    level: OptimizationLevel,
    no_cse_llm: bool,
    target: OptimizationTarget,
    warn: bool,
) -> Vec<String> {
    let mut passes = build_pass_list(level, no_cse_llm, target);
    if warn {
        passes.push(UNCONSUMED_VALUE_WARNING.to_string());
    }
    passes
}

/// Materialize the final pass list for a [`PipelineConfig`].
///
/// Order of operations:
/// 1. If `pass_list_override` is `Some`, that vector becomes the base list
///    (opt-level / target / warn-unconsumed are ignored).
/// 2. Otherwise, the base list comes from [`build_pass_list_with_warn`].
/// 3. `no_cse_llm` and `disable_passes` filter the resulting list.
///
/// Single source of truth used by both the MLIR-pass-manager build path
/// ([`build_pipeline_with_config`]) and the diagnostics path
/// (`process_module_with_diagnostics` in `api/pipeline.rs`).
pub fn resolve_pass_list(config: &apxm_core::types::PipelineConfig) -> Vec<String> {
    let mut passes = if let Some(override_list) = config.pass_list_override.as_ref() {
        override_list.clone()
    } else {
        build_pass_list_with_warn(
            config.opt_level,
            config.no_cse_llm,
            config.target,
            config.warn_unconsumed,
        )
    };
    if config.no_cse_llm {
        passes.retain(|p| p != CSE);
    }
    if !config.disable_passes.is_empty() {
        let drop: std::collections::HashSet<&str> =
            config.disable_passes.iter().map(String::as_str).collect();
        passes.retain(|p| !drop.contains(p.as_str()));
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
        assert_eq!(passes[2], TEMPLATE_SPECIALIZATION);
        assert!(passes.contains(&ASSIGN_PRIORITY.to_string()));
        assert_eq!(passes.last().unwrap(), ASSIGN_PRIORITY);
        let symbol_idx = passes.iter().position(|p| p == SYMBOL_DCE).unwrap();
        let priority_idx = passes.iter().position(|p| p == ASSIGN_PRIORITY).unwrap();
        assert!(symbol_idx < priority_idx);
        assert!(!passes.contains(&CSE.to_string()));
        assert_default_excludes_semantic_rewrites(&passes);
    }

    #[test]
    fn no_cse_llm_filters_explicit_cse_override() {
        let config = apxm_core::types::PipelineConfig {
            no_cse_llm: true,
            pass_list_override: Some(vec![NORMALIZE.to_string(), CSE.to_string()]),
            ..Default::default()
        };
        let passes = resolve_pass_list(&config);
        assert_eq!(passes, vec![NORMALIZE.to_string()]);
    }

    #[test]
    fn o2_pass_list_matches_spec() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Balanced);
        assert!(passes.contains(&ASSIGN_PRIORITY.to_string()));
        assert!(passes.contains(&TEMPLATE_SPECIALIZATION.to_string()));
        assert!(passes.contains(&DEAD_CONTEXT_ELIMINATION.to_string()));
        assert_default_excludes_semantic_rewrites(&passes);
    }

    #[test]
    fn o3_iterates_convergence_loop() {
        let passes = build_pass_list(OptimizationLevel::O3, false, OptimizationTarget::Balanced);
        // Check preamble ordering
        assert_eq!(passes[0], NORMALIZE);
        assert_eq!(passes[1], BUILD_PROMPT);
        assert_eq!(passes[2], TEMPLATE_SPECIALIZATION);
        assert!(passes.contains(&ASSIGN_PRIORITY.to_string()));
        assert_default_excludes_semantic_rewrites(&passes);
        // UNCONSUMED_VALUE_WARNING is opt-in via --warn (Task 6); it must NOT appear
        // by default at any opt level. See unconsumed_value_warning_off_by_default.
        // Convergence loop produces many more passes than O2
        let o2_passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Balanced);
        assert!(passes.len() > o2_passes.len() * 3);
    }

    #[test]
    fn target_latency_does_not_enable_unproven_fusion() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Latency);
        assert!(!passes.contains(&FUSE_ASK_OPS.to_string()));
        assert!(passes.contains(&SCHEDULING.to_string()));
        let o3 = build_pass_list(OptimizationLevel::O3, false, OptimizationTarget::Latency);
        assert!(!o3.contains(&FUSE_ASK_OPS.to_string()));
        assert!(o3.contains(&SCHEDULING.to_string()));
    }

    #[test]
    fn target_cost_keeps_generic_cse_disabled() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Cost);
        assert!(!passes.contains(&CSE.to_string()));
        assert!(passes.contains(&DEAD_CONTEXT_ELIMINATION.to_string()));
    }

    #[test]
    fn test_target_tokens_enables_dce() {
        let passes = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Tokens);
        let dce_idx = passes
            .iter()
            .position(|p| p == DEAD_CONTEXT_ELIMINATION)
            .unwrap();
        let canonicalizer_idx = passes.iter().position(|p| p == CANONICALIZER).unwrap();
        assert!(dce_idx < canonicalizer_idx);
        // At O3 the DCE-before-scheduling ordering still holds.
        let o3 = build_pass_list(OptimizationLevel::O3, false, OptimizationTarget::Tokens);
        let dce_idx_o3 = o3
            .iter()
            .position(|p| p == DEAD_CONTEXT_ELIMINATION)
            .unwrap();
        let scheduling_idx_o3 = o3.iter().position(|p| p == SCHEDULING).unwrap();
        assert!(dce_idx_o3 < scheduling_idx_o3);
    }

    #[test]
    fn test_balanced_matches_default() {
        let balanced = build_pass_list(OptimizationLevel::O2, false, OptimizationTarget::Balanced);
        // Check structural ordering, not exact count
        assert_eq!(balanced[0], NORMALIZE);
        assert_eq!(balanced[1], BUILD_PROMPT);
        assert_eq!(balanced[2], TEMPLATE_SPECIALIZATION);
        assert!(balanced.contains(&ASSIGN_PRIORITY.to_string()));
        assert_default_excludes_semantic_rewrites(&balanced);
    }

    #[test]
    fn assign_priority_runs_after_safe_cleanup() {
        for target in [
            OptimizationTarget::Balanced,
            OptimizationTarget::Latency,
            OptimizationTarget::Cost,
            OptimizationTarget::Tokens,
        ] {
            for level in [OptimizationLevel::O1, OptimizationLevel::O2] {
                let passes = build_pass_list(level, false, target);
                let priority_idx = passes.iter().position(|p| p == ASSIGN_PRIORITY).unwrap();
                let symbol_idx = passes.iter().position(|p| p == SYMBOL_DCE).unwrap();
                assert!(
                    priority_idx > symbol_idx,
                    "ASSIGN_PRIORITY must run after SYMBOL_DCE at {level:?}/{target:?}"
                );
            }
        }
    }

    #[test]
    fn default_o_levels_do_not_run_generic_cse() {
        // Generic MLIR CSE remains explicit-only until LLM determinism and
        // memoization are represented in the graph contract.
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
                assert!(
                    !passes.iter().any(|p| p == CSE),
                    "generic CSE must not run by default at {level:?}/{target:?}"
                );
            }
        }
    }

    #[test]
    fn no_cse_llm_skips_default_cse() {
        for level in [
            OptimizationLevel::O1,
            OptimizationLevel::O2,
            OptimizationLevel::O3,
        ] {
            let passes = build_pass_list(level, true, OptimizationTarget::Balanced);
            assert!(
                !passes.contains(&CSE.to_string()),
                "no_cse_llm at {level:?} must elide every CSE (incl. the early one)"
            );
        }
    }

    #[test]
    fn o2_runs_dead_context_before_canonicalizer() {
        for target in [
            OptimizationTarget::Balanced,
            OptimizationTarget::Latency,
            OptimizationTarget::Cost,
            OptimizationTarget::Tokens,
        ] {
            let passes = build_pass_list(OptimizationLevel::O2, false, target);
            let dce_idx = passes
                .iter()
                .position(|p| p == DEAD_CONTEXT_ELIMINATION)
                .expect("DEAD_CONTEXT_ELIMINATION must appear at O2");
            let canonicalizer_idx = passes
                .iter()
                .position(|p| p == CANONICALIZER)
                .expect("CANONICALIZER must appear at O2");
            assert!(
                dce_idx < canonicalizer_idx,
                "dead context must be pruned before canonicalizer at O2/{target:?}"
            );
        }
    }

    #[test]
    fn explicit_semantic_passes_are_available_via_override() {
        let explicit = [
            PROMPT_CANONICALIZATION,
            SCHEMA_NARROWING,
            FUSE_ASK_OPS,
            CONDENSE_OPS,
        ];
        let config = apxm_core::types::PipelineConfig {
            pass_list_override: Some(explicit.iter().map(|p| (*p).to_string()).collect()),
            ..Default::default()
        };
        let expected: Vec<String> = explicit.iter().map(|p| (*p).to_string()).collect();
        assert_eq!(resolve_pass_list(&config), expected);
    }

    #[test]
    fn base_o_levels_do_not_include_side_effecting_dspy_optimize() {
        for level in [
            OptimizationLevel::O1,
            OptimizationLevel::O2,
            OptimizationLevel::O3,
        ] {
            let passes = build_pass_list(level, false, OptimizationTarget::Balanced);
            assert!(!passes.iter().any(|p| p == DSPY_OPTIMIZE));
        }
    }

    #[test]
    fn dspy_optimize_is_available_via_explicit_override() {
        let config = apxm_core::types::PipelineConfig {
            pass_list_override: Some(vec![BUILD_PROMPT.to_string(), DSPY_OPTIMIZE.to_string()]),
            ..Default::default()
        };
        assert_eq!(
            resolve_pass_list(&config),
            vec![BUILD_PROMPT.to_string(), DSPY_OPTIMIZE.to_string()]
        );
    }

    #[test]
    fn default_dspy_does_not_mutate_explicit_pass_list() {
        let config = apxm_core::types::PipelineConfig {
            pass_list_override: Some(vec![NORMALIZE.to_string(), BUILD_PROMPT.to_string()]),
            ..Default::default()
        };
        assert_eq!(
            resolve_pass_list(&config),
            vec![NORMALIZE.to_string(), BUILD_PROMPT.to_string()]
        );
    }

    #[test]
    fn semantic_rewrites_are_absent_from_o2_targets() {
        for target in [
            OptimizationTarget::Balanced,
            OptimizationTarget::Latency,
            OptimizationTarget::Cost,
            OptimizationTarget::Tokens,
            OptimizationTarget::Parallelism,
        ] {
            let passes = build_pass_list(OptimizationLevel::O2, false, target);
            assert_default_excludes_semantic_rewrites(&passes);
        }
    }

    #[test]
    fn rust_only_passes_are_filtered_from_mlir_dispatch() {
        // Sanity: Rust-only passes must not look like MLIR passes.
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
    fn unconsumed_value_warning_off_by_default() {
        use OptimizationTarget::Balanced;
        for level in [
            OptimizationLevel::O1,
            OptimizationLevel::O2,
            OptimizationLevel::O3,
        ] {
            let passes = build_pass_list(level, false, Balanced);
            assert!(
                !passes.iter().any(|p| p == UNCONSUMED_VALUE_WARNING),
                "UNCONSUMED_VALUE_WARNING must not appear by default at {level:?}"
            );
        }
    }

    #[test]
    fn unconsumed_value_warning_added_when_requested() {
        use OptimizationTarget::Balanced;
        let passes = build_pass_list_with_warn(OptimizationLevel::O1, false, Balanced, true);
        assert!(passes.iter().any(|p| p == UNCONSUMED_VALUE_WARNING));
    }

    #[test]
    fn scheduling_pass_starts_at_o2() {
        use OptimizationTarget::*;
        for target in [Balanced, Latency, Cost, Tokens, Parallelism] {
            let o1 = build_pass_list(OptimizationLevel::O1, false, target);
            let o2 = build_pass_list(OptimizationLevel::O2, false, target);
            let o3 = build_pass_list(OptimizationLevel::O3, false, target);
            assert!(
                !o1.iter().any(|p| p == SCHEDULING),
                "SCHEDULING must not appear in O1 (target={target:?})"
            );
            assert!(
                o2.iter().any(|p| p == SCHEDULING),
                "SCHEDULING must appear in O2 (target={target:?})"
            );
            assert!(
                o3.iter().any(|p| p == SCHEDULING),
                "SCHEDULING must remain in O3 (target={target:?})"
            );
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
                        panic!("BIND_TOOL_HANDLERS missing from pipeline at {level:?}/{target:?}")
                    });
                assert!(
                    bth_idx > tb_idx,
                    "BIND_TOOL_HANDLERS must run after TOOL_BINDING at {level:?}/{target:?}"
                );
            }
        }
    }

    fn assert_default_excludes_semantic_rewrites(pass_list: &[String]) {
        for pass_name in [
            PROMPT_CANONICALIZATION,
            SCHEMA_NARROWING,
            FUSE_ASK_OPS,
            CONDENSE_OPS,
        ] {
            assert!(
                !pass_list.contains(&pass_name.to_string()),
                "{pass_name} must remain explicit until its production contract is enforced"
            );
        }
    }
}
