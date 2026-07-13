//! Compilation pipeline types module.
//!
//! Contains types for compilation stages, optimization levels, and code generation.

mod codegen;
pub mod metadata;
mod optimization;
mod optimization_summary;
mod passes;
mod stages;

pub use codegen::CodegenOptions;
pub use metadata::{PassMetadata, find_pass_metadata, list_pass_metadata};
pub use optimization::{OptimizationLevel, OptimizationTarget, PipelineConfig};
pub use optimization_summary::{
    BackendLegalityRequirements, CompilerAnalysisKind, CostProvenance, CostSummary,
    DagOptimizationSummaryV1, Determinism, EffectAuthoritySummary,
    OPTIMIZATION_SUMMARY_ARTIFACT_SECTION, OPTIMIZATION_SUMMARY_VERSION,
    OperationOptimizationSummaryV1, OptimizationDecisionV1, OptimizationDisposition,
    OptimizationSummaryV1, OptimizationTransformKind, PromptContractSummary, ReplaySafety,
    TransformationLegality,
};
pub use passes::{PassCategory, PassInfo};
pub use stages::{CompilationStage, EmitFormat, stage_rank};
