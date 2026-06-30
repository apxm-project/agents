//! Pipeline builder for the passes.
//!
//! Optimization levels:
//!   O0 - Required normalization and executable lowering only; no optimization
//!   O1 - Basic safe cleanup plus priority metadata
//!   O2 - Standard: O1 + scheduling metadata and shared-prefix analysis
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
            // O0 is the no-optimization baseline, not a "skip executable
            // lowering" mode. Normalize establishes canonical attribute
            // spelling, and BuildPrompt establishes the runtime
            // template/input_names contract for LLM ops with context.
            passes.push(NORMALIZE.to_string());
            passes.push(BUILD_PROMPT.to_string());
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
                OptimizationTarget::Tokens => [
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
                OptimizationTarget::Latency | OptimizationTarget::Parallelism => [
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
                _ => [
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
/// pipelines.
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
/// Single source of truth for the MLIR-pass-manager build path
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
