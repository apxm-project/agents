//! Typed compilation-stage planning.
//!
//! A [`PipelinePlan`] is the compiler's executable source of truth. It keeps
//! MLIR work separate from artifact work that runs after artifact emission and
//! makes O3 cleanup repetition a bounded convergence group instead of a
//! materialized sequence of duplicate pass names.

use apxm_core::types::compiler::CompilerAnalysisKind;
use serde::{Deserialize, Serialize};

/// The maximum number of O3 cleanup iterations before compilation reports a
/// bounded-convergence result.
pub const O3_MAX_CLEANUP_ITERATIONS: usize = 10;

/// The execution boundary for a pipeline stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStageKind {
    /// Required MLIR normalization or lowering needed for executable output.
    RequiredLowering,
    /// An MLIR transformation that can rewrite the module.
    #[default]
    MlirRewrite,
    /// An MLIR analysis that derives metadata without selecting artifact work.
    MlirAnalysis,
    /// Validation performed after MLIR emits the artifact representation.
    ArtifactValidation,
    /// Artifact mutation performed after validation succeeds.
    ArtifactFinalization,
    /// A non-mutating compiler diagnostic stage.
    Diagnostic,
}

impl PipelineStageKind {
    /// Whether this stage is dispatched through the MLIR pass manager.
    pub const fn executes_in_mlir(self) -> bool {
        matches!(
            self,
            Self::RequiredLowering | Self::MlirRewrite | Self::MlirAnalysis | Self::Diagnostic
        )
    }
}

/// A named compiler stage with its execution boundary and removal policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineStage {
    /// Stable stage identifier.
    pub name: String,
    /// The boundary that owns the stage's execution.
    pub kind: PipelineStageKind,
    /// Whether pass-list overrides and pass ablations must retain the stage.
    pub mandatory: bool,
    /// Analyses that must be current before this stage may execute.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_analyses: Vec<CompilerAnalysisKind>,
    /// Analyses that remain valid after this stage completes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preserved_analyses: Vec<CompilerAnalysisKind>,
    /// Analyses invalidated by this stage's mutation or artifact finalization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invalidated_analyses: Vec<CompilerAnalysisKind>,
}

impl PipelineStage {
    /// Construct a stage from a canonical pass or artifact-stage name.
    pub fn new(name: impl Into<String>, kind: PipelineStageKind, mandatory: bool) -> Self {
        Self {
            name: name.into(),
            kind,
            mandatory,
            required_analyses: Vec::new(),
            preserved_analyses: Vec::new(),
            invalidated_analyses: Vec::new(),
        }
    }

    /// Attach the stage's explicit reusable-analysis contract.
    pub fn with_analysis_contract(
        mut self,
        required_analyses: &[CompilerAnalysisKind],
        preserved_analyses: &[CompilerAnalysisKind],
        invalidated_analyses: &[CompilerAnalysisKind],
    ) -> Self {
        self.required_analyses = required_analyses.to_vec();
        self.preserved_analyses = preserved_analyses.to_vec();
        self.invalidated_analyses = invalidated_analyses.to_vec();
        self
    }

    /// Whether this stage is dispatched through the MLIR pass manager.
    pub const fn executes_in_mlir(&self) -> bool {
        self.kind.executes_in_mlir()
    }
}

/// A repeated MLIR stage group that terminates when the module reaches a fixed point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineConvergenceGroup {
    /// Stable group identifier used in diagnostics.
    pub name: String,
    /// The ordered stages run during one convergence iteration.
    pub stages: Vec<PipelineStage>,
    /// The maximum number of iterations permitted for this group.
    pub max_iterations: usize,
}

impl PipelineConvergenceGroup {
    /// Construct a bounded convergence group.
    pub fn new(name: impl Into<String>, stages: Vec<PipelineStage>, max_iterations: usize) -> Self {
        Self {
            name: name.into(),
            stages,
            max_iterations,
        }
    }
}

/// One ordered unit of pipeline execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PipelinePlanStep {
    /// A stage that executes once.
    Stage(PipelineStage),
    /// A bounded convergence group.
    Convergence(PipelineConvergenceGroup),
}

/// A typed compilation plan derived from a pipeline configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelinePlan {
    /// Ordered execution units.
    pub steps: Vec<PipelinePlanStep>,
}

impl PipelinePlan {
    /// Construct an empty plan.
    pub const fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Append a stage that executes once.
    pub fn push_stage(&mut self, stage: PipelineStage) {
        self.steps.push(PipelinePlanStep::Stage(stage));
    }

    /// Append a bounded convergence group.
    pub fn push_convergence_group(&mut self, group: PipelineConvergenceGroup) {
        self.steps.push(PipelinePlanStep::Convergence(group));
    }

    /// Return every stage in execution order, with one representative copy of
    /// each convergence-group stage.
    pub fn stages(&self) -> Vec<&PipelineStage> {
        self.steps
            .iter()
            .flat_map(|step| match step {
                PipelinePlanStep::Stage(stage) => vec![stage],
                PipelinePlanStep::Convergence(group) => group.stages.iter().collect(),
            })
            .collect()
    }

    /// Return the MLIR stage names in execution order, without materializing
    /// repeated convergence iterations.
    pub fn mlir_pass_names(&self) -> Vec<String> {
        self.stages()
            .into_iter()
            .filter(|stage| stage.kind.executes_in_mlir())
            .map(|stage| stage.name.clone())
            .collect()
    }

    /// Return artifact-owned stages in their artifact execution order.
    pub fn artifact_stages(&self) -> Vec<&PipelineStage> {
        self.stages()
            .into_iter()
            .filter(|stage| !stage.kind.executes_in_mlir())
            .collect()
    }

    /// Whether the plan contains a named stage at any execution boundary.
    pub fn contains_stage(&self, name: &str) -> bool {
        self.stages().into_iter().any(|stage| stage.name == name)
    }

    /// Retain only allowed non-mandatory stages while preserving all mandatory stages.
    pub fn retain_unless_disabled(&mut self, disabled: &std::collections::HashSet<&str>) {
        self.steps = std::mem::take(&mut self.steps)
            .into_iter()
            .filter_map(|step| match step {
                PipelinePlanStep::Stage(stage) => (stage.mandatory
                    || !disabled.contains(stage.name.as_str()))
                .then_some(PipelinePlanStep::Stage(stage)),
                PipelinePlanStep::Convergence(mut group) => {
                    group
                        .stages
                        .retain(|stage| stage.mandatory || !disabled.contains(stage.name.as_str()));
                    (!group.stages.is_empty()).then_some(PipelinePlanStep::Convergence(group))
                }
            })
            .collect();
    }
}
