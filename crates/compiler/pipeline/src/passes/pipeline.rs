//! Typed pipeline construction.
//!
//! The [`PipelinePlan`] is the sole composition source for compiler work. It
//! separates MLIR stages from artifact validation and finalization, so a pass
//! name never implies execution at a boundary that cannot run it.

use super::PassManager;
use super::plan::{
    O3_MAX_CLEANUP_ITERATIONS, PipelineConvergenceGroup, PipelinePlan, PipelineStage,
    PipelineStageKind,
};
use apxm_core::error::compiler::Result;
use apxm_core::types::compiler::CompilerAnalysisKind;
use apxm_core::types::compiler::metadata as passes;
use apxm_core::types::{OptimizationLevel, OptimizationTarget, PipelineConfig};

// Short aliases for pass names — downstream compiler consumers read these
// through apxm_core::types::compiler::metadata, which re-exports the canonical
// AIS authoring definitions.
const NORMALIZE: &str = passes::NORMALIZE.name;
const BUILD_PROMPT: &str = passes::BUILD_PROMPT.name;
const ASSIGN_PRIORITY: &str = passes::ASSIGN_PRIORITY.name;
const SCHEDULING: &str = passes::SCHEDULING.name;
const SHARED_PREFIX_ANALYSIS: &str = passes::SHARED_PREFIX_ANALYSIS.name;
const UNCONSUMED_VALUE_WARNING: &str = passes::UNCONSUMED_VALUE_WARNING.name;
const TEMPLATE_SPECIALIZATION: &str = passes::TEMPLATE_SPECIALIZATION.name;
const DEAD_CONTEXT_ELIMINATION: &str = passes::DEAD_CONTEXT_ELIMINATION.name;
const PURE_DEAD_NODE_ELIMINATION: &str = passes::PURE_DEAD_NODE_ELIMINATION.name;
const SCHEMA_NARROWING: &str = passes::SCHEMA_NARROWING.name;
const CANONICALIZER: &str = passes::CANONICALIZER.name;
const CSE: &str = passes::CSE.name;
const SYMBOL_DCE: &str = passes::SYMBOL_DCE.name;

// These stages are performed after MLIR artifact emission in api/module.rs.
// They remain in the typed plan so diagnostics can describe their real owner
// without pretending that the MLIR pass manager executes Rust artifact work.
const TEMPLATE_PLACEHOLDER_VALIDATION: &str = "validate-template-placeholders";
const TOKEN_ESTIMATE_REFINEMENT: &str = "refine-token-estimates";
const CAPABILITY_BINDING: &str = "capability-binding-check";
const BIND_CAPABILITY_HANDLERS: &str = "bind-capability-handlers";
const O3_CLEANUP_GROUP: &str = "o3-cleanup";

const ALL_ANALYSES: &[CompilerAnalysisKind] = &[
    CompilerAnalysisKind::PromptContract,
    CompilerAnalysisKind::EffectAuthority,
    CompilerAnalysisKind::DagUse,
    CompilerAnalysisKind::TokenCost,
    CompilerAnalysisKind::ProfileCost,
    CompilerAnalysisKind::BackendLegality,
];
const PROMPT_AND_COST: &[CompilerAnalysisKind] = &[
    CompilerAnalysisKind::PromptContract,
    CompilerAnalysisKind::TokenCost,
    CompilerAnalysisKind::BackendLegality,
];
const EFFECT_DAG_AND_PROFILE: &[CompilerAnalysisKind] = &[
    CompilerAnalysisKind::EffectAuthority,
    CompilerAnalysisKind::DagUse,
    CompilerAnalysisKind::ProfileCost,
];
const EFFECT_DAG_TOKEN_AND_PROFILE: &[CompilerAnalysisKind] = &[
    CompilerAnalysisKind::EffectAuthority,
    CompilerAnalysisKind::DagUse,
    CompilerAnalysisKind::TokenCost,
    CompilerAnalysisKind::ProfileCost,
];
const ALL_PRESERVED: &[CompilerAnalysisKind] = ALL_ANALYSES;

/// Return the execution boundary for a stage named in an explicit pass list.
pub fn stage_kind_for_name(name: &str) -> PipelineStageKind {
    match name {
        NORMALIZE | BUILD_PROMPT => PipelineStageKind::RequiredLowering,
        SCHEDULING | SHARED_PREFIX_ANALYSIS | ASSIGN_PRIORITY => PipelineStageKind::MlirAnalysis,
        UNCONSUMED_VALUE_WARNING | SCHEMA_NARROWING => PipelineStageKind::Diagnostic,
        TEMPLATE_PLACEHOLDER_VALIDATION | CAPABILITY_BINDING => {
            PipelineStageKind::ArtifactValidation
        }
        TOKEN_ESTIMATE_REFINEMENT | BIND_CAPABILITY_HANDLERS => {
            PipelineStageKind::ArtifactFinalization
        }
        _ => PipelineStageKind::MlirRewrite,
    }
}

/// Whether a named pass is dispatched through MLIR.
///
/// This compatibility helper recognizes the two artifact-owned names that
/// historically appeared in pass lists. New execution should use
/// [`PipelineStageKind::executes_in_mlir`] from the typed plan instead.
pub fn is_mlir_pass(name: &str) -> bool {
    !matches!(
        name,
        TEMPLATE_PLACEHOLDER_VALIDATION
            | TOKEN_ESTIMATE_REFINEMENT
            | CAPABILITY_BINDING
            | BIND_CAPABILITY_HANDLERS
    )
}

/// Configure a manager with the default typed plan for an optimization level.
pub fn build_pipeline(pm: &mut PassManager, level: OptimizationLevel) -> Result<()> {
    build_pipeline_with_config(pm, level, false, OptimizationTarget::Balanced, false)
}

/// Configure a manager with the typed plan derived from legacy pipeline inputs.
pub fn build_pipeline_with_config(
    pm: &mut PassManager,
    level: OptimizationLevel,
    no_cse_llm: bool,
    target: OptimizationTarget,
    warn: bool,
) -> Result<()> {
    pm.set_plan(build_pipeline_plan_with_warn(
        level, no_cse_llm, target, warn,
    ));
    Ok(())
}

/// Build the typed default plan for one optimization level.
pub fn build_pipeline_plan(
    level: OptimizationLevel,
    _no_cse_llm: bool,
    target: OptimizationTarget,
) -> PipelinePlan {
    let mut plan = PipelinePlan::new();

    match level {
        OptimizationLevel::O0 => {
            plan.push_stage(required_lowering(NORMALIZE));
            plan.push_stage(required_lowering(BUILD_PROMPT));
        }
        OptimizationLevel::O1 => {
            append_required_lowering(&mut plan);
            append_stages(
                &mut plan,
                [
                    rewrite(TEMPLATE_SPECIALIZATION),
                    rewrite(DEAD_CONTEXT_ELIMINATION),
                    rewrite(CANONICALIZER),
                    rewrite(PURE_DEAD_NODE_ELIMINATION),
                    rewrite(SYMBOL_DCE),
                    analysis(ASSIGN_PRIORITY),
                ],
            );
        }
        OptimizationLevel::O2 => {
            append_required_lowering(&mut plan);
            append_stages(
                &mut plan,
                [
                    rewrite(TEMPLATE_SPECIALIZATION),
                    rewrite(DEAD_CONTEXT_ELIMINATION),
                ],
            );
            append_o2_target_stages(&mut plan, target);
            append_stages(
                &mut plan,
                [
                    rewrite(PURE_DEAD_NODE_ELIMINATION),
                    rewrite(SYMBOL_DCE),
                    analysis(SHARED_PREFIX_ANALYSIS),
                    analysis(ASSIGN_PRIORITY),
                ],
            );
        }
        OptimizationLevel::O3 => {
            append_required_lowering(&mut plan);
            append_stages(
                &mut plan,
                [
                    rewrite(TEMPLATE_SPECIALIZATION),
                    rewrite(DEAD_CONTEXT_ELIMINATION),
                ],
            );
            plan.push_convergence_group(PipelineConvergenceGroup::new(
                O3_CLEANUP_GROUP,
                o3_cleanup_stages(target),
                O3_MAX_CLEANUP_ITERATIONS,
            ));
            append_stages(
                &mut plan,
                [analysis(SHARED_PREFIX_ANALYSIS), analysis(ASSIGN_PRIORITY)],
            );
        }
    }

    append_artifact_stages(&mut plan);
    plan
}

/// Build a typed plan and append the opt-in diagnostic stage when requested.
pub fn build_pipeline_plan_with_warn(
    level: OptimizationLevel,
    no_cse_llm: bool,
    target: OptimizationTarget,
    warn: bool,
) -> PipelinePlan {
    let mut plan = build_pipeline_plan(level, no_cse_llm, target);
    if warn {
        insert_before_artifact_stages(&mut plan, diagnostic(UNCONSUMED_VALUE_WARNING));
    }
    plan
}

/// Return MLIR pass names for callers that still consume the legacy list API.
///
/// O3 exposes one cleanup iteration here; the manager executes that group until
/// convergence instead of materializing ten copies of its pass names.
pub fn build_pass_list(
    level: OptimizationLevel,
    no_cse_llm: bool,
    target: OptimizationTarget,
) -> Vec<String> {
    build_pipeline_plan(level, no_cse_llm, target).mlir_pass_names()
}

/// Like [`build_pass_list`] but appends `unconsumed-value-warning` when requested.
pub fn build_pass_list_with_warn(
    level: OptimizationLevel,
    no_cse_llm: bool,
    target: OptimizationTarget,
    warn: bool,
) -> Vec<String> {
    build_pipeline_plan_with_warn(level, no_cse_llm, target, warn).mlir_pass_names()
}

/// Materialize the executable plan for a [`PipelineConfig`].
///
/// Explicit pass lists replace only configurable MLIR stages. Required lowering
/// and artifact stages remain mandatory, and artifact stages never enter MLIR
/// dispatch. Disabling a mandatory stage is ignored so ablation flags cannot
/// bypass executable lowering or artifact validation/finalization.
pub fn resolve_pipeline_plan(config: &PipelineConfig) -> PipelinePlan {
    let mut plan = if let Some(override_list) = config.pass_list_override.as_ref() {
        plan_from_override(override_list)
    } else {
        build_pipeline_plan_with_warn(
            config.opt_level,
            config.no_cse_llm,
            config.target,
            config.warn_unconsumed,
        )
    };

    let mut disabled: std::collections::HashSet<&str> =
        config.disable_passes.iter().map(String::as_str).collect();
    if config.no_cse_llm {
        disabled.insert(CSE);
    }
    plan.retain_unless_disabled(&disabled);
    plan
}

/// Return the MLIR stage list for callers retaining the legacy list API.
pub fn resolve_pass_list(config: &PipelineConfig) -> Vec<String> {
    resolve_pipeline_plan(config).mlir_pass_names()
}

fn append_required_lowering(plan: &mut PipelinePlan) {
    append_stages(
        plan,
        [
            required_lowering(NORMALIZE),
            required_lowering(BUILD_PROMPT),
        ],
    );
}

fn append_o2_target_stages(plan: &mut PipelinePlan, target: OptimizationTarget) {
    match target {
        OptimizationTarget::Tokens | OptimizationTarget::Cost | OptimizationTarget::Balanced => {
            plan.push_stage(rewrite(CANONICALIZER));
        }
        OptimizationTarget::Latency | OptimizationTarget::Parallelism => {
            append_stages(plan, [rewrite(SCHEDULING), rewrite(CANONICALIZER)]);
        }
    }

    if !plan.contains_stage(SCHEDULING) {
        plan.push_stage(analysis(SCHEDULING));
    }
}

fn o3_cleanup_stages(target: OptimizationTarget) -> Vec<PipelineStage> {
    let mut stages = match target {
        OptimizationTarget::Latency | OptimizationTarget::Parallelism => vec![
            analysis(SCHEDULING),
            rewrite(TEMPLATE_SPECIALIZATION),
            rewrite(DEAD_CONTEXT_ELIMINATION),
            rewrite(CANONICALIZER),
        ],
        OptimizationTarget::Tokens | OptimizationTarget::Cost | OptimizationTarget::Balanced => {
            vec![
                rewrite(TEMPLATE_SPECIALIZATION),
                rewrite(DEAD_CONTEXT_ELIMINATION),
                analysis(SCHEDULING),
                rewrite(CANONICALIZER),
            ]
        }
    };
    // This transform erases only a closed pure allow-list, so it must run
    // before symbol cleanup sees the reduced operation graph.
    stages.push(rewrite(PURE_DEAD_NODE_ELIMINATION));
    stages.push(rewrite(SYMBOL_DCE));
    stages
}

fn append_artifact_stages(plan: &mut PipelinePlan) {
    for stage in mandatory_artifact_stages() {
        plan.push_stage(stage);
    }
}

/// Mandatory artifact-owned stages, in the only order that may finalize a
/// compiled execution DAG. Artifact emission consumes this same sequence so
/// diagnostics describe the stages that actually ran.
pub(crate) fn mandatory_artifact_stages() -> [PipelineStage; 4] {
    [
        artifact_validation(TEMPLATE_PLACEHOLDER_VALIDATION),
        artifact_finalization(TOKEN_ESTIMATE_REFINEMENT),
        artifact_validation(CAPABILITY_BINDING),
        artifact_finalization(BIND_CAPABILITY_HANDLERS),
    ]
}

fn plan_from_override(override_list: &[String]) -> PipelinePlan {
    let mut plan = PipelinePlan::new();
    append_required_lowering(&mut plan);
    for name in override_list {
        if matches!(
            name.as_str(),
            NORMALIZE
                | BUILD_PROMPT
                | TEMPLATE_PLACEHOLDER_VALIDATION
                | TOKEN_ESTIMATE_REFINEMENT
                | CAPABILITY_BINDING
                | BIND_CAPABILITY_HANDLERS
        ) {
            continue;
        }
        plan.push_stage(explicit_stage(name));
    }
    append_artifact_stages(&mut plan);
    plan
}

fn insert_before_artifact_stages(plan: &mut PipelinePlan, stage: PipelineStage) {
    let artifact_index = plan
        .steps
        .iter()
        .position(|step| match step {
            super::plan::PipelinePlanStep::Stage(stage) => !stage.kind.executes_in_mlir(),
            super::plan::PipelinePlanStep::Convergence(_) => false,
        })
        .unwrap_or(plan.steps.len());
    plan.steps
        .insert(artifact_index, super::plan::PipelinePlanStep::Stage(stage));
}

fn append_stages<const N: usize>(plan: &mut PipelinePlan, stages: [PipelineStage; N]) {
    for stage in stages {
        plan.push_stage(stage);
    }
}

fn required_lowering(name: &str) -> PipelineStage {
    stage(name, PipelineStageKind::RequiredLowering, true)
}

fn rewrite(name: &str) -> PipelineStage {
    stage(name, PipelineStageKind::MlirRewrite, false)
}

fn analysis(name: &str) -> PipelineStage {
    stage(name, PipelineStageKind::MlirAnalysis, false)
}

fn diagnostic(name: &str) -> PipelineStage {
    stage(name, PipelineStageKind::Diagnostic, false)
}

fn artifact_validation(name: &str) -> PipelineStage {
    stage(name, PipelineStageKind::ArtifactValidation, true)
}

fn artifact_finalization(name: &str) -> PipelineStage {
    stage(name, PipelineStageKind::ArtifactFinalization, true)
}

fn explicit_stage(name: &str) -> PipelineStage {
    stage(name, stage_kind_for_name(name), false)
}

fn stage(name: &str, kind: PipelineStageKind, mandatory: bool) -> PipelineStage {
    let (required, preserved, invalidated): (
        &[CompilerAnalysisKind],
        &[CompilerAnalysisKind],
        &[CompilerAnalysisKind],
    ) = match name {
        NORMALIZE | CANONICALIZER => (&[][..], &[][..], ALL_ANALYSES),
        BUILD_PROMPT | TEMPLATE_SPECIALIZATION => {
            (&[][..], EFFECT_DAG_AND_PROFILE, PROMPT_AND_COST)
        }
        DEAD_CONTEXT_ELIMINATION => (
            &[
                CompilerAnalysisKind::PromptContract,
                CompilerAnalysisKind::DagUse,
            ],
            &[],
            ALL_ANALYSES,
        ),
        PURE_DEAD_NODE_ELIMINATION | SYMBOL_DCE => (
            &[
                CompilerAnalysisKind::DagUse,
                CompilerAnalysisKind::EffectAuthority,
            ],
            &[],
            ALL_ANALYSES,
        ),
        SHARED_PREFIX_ANALYSIS => (
            &[
                CompilerAnalysisKind::PromptContract,
                CompilerAnalysisKind::TokenCost,
            ],
            EFFECT_DAG_TOKEN_AND_PROFILE,
            &[
                CompilerAnalysisKind::PromptContract,
                CompilerAnalysisKind::BackendLegality,
            ],
        ),
        ASSIGN_PRIORITY => (
            &[
                CompilerAnalysisKind::DagUse,
                CompilerAnalysisKind::EffectAuthority,
                CompilerAnalysisKind::TokenCost,
                CompilerAnalysisKind::ProfileCost,
            ],
            ALL_PRESERVED,
            &[],
        ),
        SCHEDULING => (
            &[
                CompilerAnalysisKind::DagUse,
                CompilerAnalysisKind::EffectAuthority,
                CompilerAnalysisKind::TokenCost,
                CompilerAnalysisKind::ProfileCost,
            ],
            &[],
            ALL_ANALYSES,
        ),
        UNCONSUMED_VALUE_WARNING => (&[CompilerAnalysisKind::DagUse], ALL_PRESERVED, &[]),
        TEMPLATE_PLACEHOLDER_VALIDATION => {
            (&[CompilerAnalysisKind::PromptContract], ALL_PRESERVED, &[])
        }
        TOKEN_ESTIMATE_REFINEMENT => (
            &[
                CompilerAnalysisKind::PromptContract,
                CompilerAnalysisKind::TokenCost,
            ],
            ALL_PRESERVED,
            &[],
        ),
        CAPABILITY_BINDING | BIND_CAPABILITY_HANDLERS => (
            &[
                CompilerAnalysisKind::EffectAuthority,
                CompilerAnalysisKind::DagUse,
                CompilerAnalysisKind::BackendLegality,
            ],
            ALL_PRESERVED,
            &[],
        ),
        _ => (&[][..], &[][..], ALL_ANALYSES),
    };
    PipelineStage::new(name, kind, mandatory).with_analysis_contract(
        required,
        preserved,
        invalidated,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPLICIT_ONLY_PASSES: &[&str] = &[
        "fuse-ask-ops",
        "condense-ops",
        "schema-narrowing",
        "prompt-canonicalization",
        CSE,
    ];

    #[test]
    fn default_o_levels_exclude_experiment_only_passes() {
        for level in [
            OptimizationLevel::O0,
            OptimizationLevel::O1,
            OptimizationLevel::O2,
            OptimizationLevel::O3,
        ] {
            let passes = build_pass_list(level, false, OptimizationTarget::Balanced);
            for explicit_only in EXPLICIT_ONLY_PASSES {
                assert!(
                    !passes.iter().any(|pass| pass == explicit_only),
                    "{level:?} unexpectedly includes {explicit_only}"
                );
            }
        }
    }

    #[test]
    fn o3_uses_one_bounded_convergence_group() {
        let plan = build_pipeline_plan(OptimizationLevel::O3, false, OptimizationTarget::Balanced);
        let groups: Vec<_> = plan
            .steps
            .iter()
            .filter_map(|step| match step {
                super::super::plan::PipelinePlanStep::Convergence(group) => Some(group),
                super::super::plan::PipelinePlanStep::Stage(_) => None,
            })
            .collect();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, O3_CLEANUP_GROUP);
        assert_eq!(groups[0].max_iterations, O3_MAX_CLEANUP_ITERATIONS);
        assert_eq!(
            build_pass_list(OptimizationLevel::O3, false, OptimizationTarget::Balanced)
                .iter()
                .filter(|pass| pass.as_str() == SYMBOL_DCE)
                .count(),
            1
        );
    }

    #[test]
    fn artifact_stages_are_mandatory_and_never_reach_mlir_dispatch() {
        let config = PipelineConfig {
            disable_passes: vec![
                CAPABILITY_BINDING.to_string(),
                BIND_CAPABILITY_HANDLERS.to_string(),
            ],
            ..PipelineConfig::default()
        };
        let plan = resolve_pipeline_plan(&config);
        let artifact_stages = plan.artifact_stages();

        assert!(artifact_stages.iter().all(|stage| stage.mandatory));
        assert_eq!(
            artifact_stages
                .iter()
                .filter(|stage| stage.name == CAPABILITY_BINDING)
                .count(),
            1
        );
        assert_eq!(
            artifact_stages
                .iter()
                .filter(|stage| stage.name == BIND_CAPABILITY_HANDLERS)
                .count(),
            1
        );
        assert!(plan.contains_stage(CAPABILITY_BINDING));
        assert!(plan.contains_stage(BIND_CAPABILITY_HANDLERS));
        assert!(
            !plan
                .mlir_pass_names()
                .contains(&CAPABILITY_BINDING.to_string())
        );
        assert!(
            !plan
                .mlir_pass_names()
                .contains(&BIND_CAPABILITY_HANDLERS.to_string())
        );
    }

    #[test]
    fn explicit_pass_lists_retain_required_lowering_and_artifact_work() {
        let config = PipelineConfig {
            pass_list_override: Some(vec![CANONICALIZER.to_string()]),
            disable_passes: vec![NORMALIZE.to_string(), BUILD_PROMPT.to_string()],
            ..PipelineConfig::default()
        };
        let plan = resolve_pipeline_plan(&config);

        assert!(plan.contains_stage(NORMALIZE));
        assert!(plan.contains_stage(BUILD_PROMPT));
        assert!(plan.contains_stage(TEMPLATE_PLACEHOLDER_VALIDATION));
        assert!(plan.contains_stage(BIND_CAPABILITY_HANDLERS));
        assert_eq!(
            plan.mlir_pass_names(),
            vec![
                NORMALIZE.to_string(),
                BUILD_PROMPT.to_string(),
                CANONICALIZER.to_string(),
            ]
        );
    }

    #[test]
    fn stages_declare_non_overlapping_analysis_invalidation() {
        for level in [
            OptimizationLevel::O0,
            OptimizationLevel::O1,
            OptimizationLevel::O2,
            OptimizationLevel::O3,
        ] {
            for stage in build_pipeline_plan(level, false, OptimizationTarget::Balanced).stages() {
                assert!(
                    stage
                        .preserved_analyses
                        .iter()
                        .all(|analysis| !stage.invalidated_analyses.contains(analysis)),
                    "{} both preserves and invalidates {stage:?}",
                    stage.name
                );
                assert_eq!(
                    stage.preserved_analyses.len() + stage.invalidated_analyses.len(),
                    ALL_ANALYSES.len(),
                    "{} leaves analysis freshness unspecified",
                    stage.name
                );
            }
        }

        let plan = build_pipeline_plan(OptimizationLevel::O2, false, OptimizationTarget::Balanced);
        let dead_context = plan
            .stages()
            .into_iter()
            .find(|stage| stage.name == DEAD_CONTEXT_ELIMINATION)
            .expect("dead-context stage");
        assert!(
            dead_context
                .required_analyses
                .contains(&CompilerAnalysisKind::PromptContract)
        );
        assert!(
            dead_context
                .invalidated_analyses
                .contains(&CompilerAnalysisKind::DagUse)
        );
    }

    #[test]
    fn unknown_explicit_stages_invalidate_all_cached_analyses() {
        let stage = explicit_stage("third-party-rewrite");
        assert!(stage.required_analyses.is_empty());
        assert!(stage.preserved_analyses.is_empty());
        assert_eq!(stage.invalidated_analyses, ALL_ANALYSES);
    }
}
