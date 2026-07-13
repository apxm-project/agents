//! Pipeline API for compiling and optimizing modules.

use crate::air_builder::AirModule;
use crate::api::{Context, Module};
use crate::optimization::CompilerOptimizationContext;
use crate::passes::{PassManager, PipelineDiagnostics, PipelinePlan, resolve_pipeline_plan};
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::{Error, codes::ErrorCode};
use apxm_core::types::compiler::metadata::DSPY_OPTIMIZE;
use apxm_core::types::{OptimizationLevel, PipelineConfig};

/// Pipeline API for compiling and optimizing modules.
pub struct Pipeline<'ctx> {
    context: &'ctx Context,
    config: PipelineConfig,
}

impl<'ctx> Pipeline<'ctx> {
    /// Creates a new pipeline with default configuration.
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            config: PipelineConfig::default(),
        }
    }

    /// Creates a new pipeline with custom configuration.
    pub fn with_config(context: &'ctx Context, config: PipelineConfig) -> Self {
        Self { context, config }
    }

    /// Creates a pipeline with one optimization level.
    pub fn with_opt_level(context: &'ctx Context, level: OptimizationLevel) -> Self {
        let config = PipelineConfig {
            opt_level: level,
            ..Default::default()
        };
        Self::with_config(context, config)
    }

    /// Creates a pipeline with a caller-provided optimization context.
    ///
    /// Prompt optimization is intentionally unavailable during production
    /// compilation, so the context remains accepted for source compatibility
    /// but does not alter the executable pipeline plan.
    pub fn with_config_and_optimization_context(
        context: &'ctx Context,
        config: PipelineConfig,
        _optimization_context: CompilerOptimizationContext,
    ) -> Self {
        Self { context, config }
    }

    /// Compile AIR text without collecting diagnostics.
    pub fn compile(&self, source: &str) -> Result<Module> {
        let module = Module::parse(self.context, source)?;
        self.process_module(module)
    }

    /// Compile AIR text and collect per-stage diagnostics.
    pub fn compile_with_diagnostics(&self, source: &str) -> Result<(Module, PipelineDiagnostics)> {
        let module = Module::parse(self.context, source)?;
        self.process_module_with_diagnostics(module)
    }

    /// Compile a frontend graph without collecting diagnostics.
    pub fn compile_graph(&self, module: &AirModule) -> Result<Module> {
        let ir_module = self.lower_graph(module)?;
        self.process_module(ir_module)
    }

    /// Compile a frontend graph and collect per-stage diagnostics.
    pub fn compile_graph_with_diagnostics(
        &self,
        module: &AirModule,
    ) -> Result<(Module, PipelineDiagnostics)> {
        let ir_module = self.lower_graph(module)?;
        self.process_module_with_diagnostics(ir_module)
    }

    fn lower_graph(&self, module: &AirModule) -> Result<Module> {
        let mut module = module.clone();

        if let Some(ref profile_path) = self.config.profile_path {
            let profile = crate::passes::profile::ExecutionProfile::load_from_file(profile_path)
                .map_err(|error| {
                    CompilerError::Unsupported(Box::new(Error::new_generic(
                        ErrorCode::InternalError,
                        format!("Failed to load profile: {error}"),
                    )))
                })?;
            profile.apply_to_module(&mut module, self.config.token_budget);
        }

        crate::token_estimate::annotate_token_estimates(&mut module);
        let air_text = module.to_air().map_err(|error| {
            CompilerError::Unsupported(Box::new(Error::new_generic(
                ErrorCode::InternalError,
                format!("AIR emission failed: {error}"),
            )))
        })?;
        Module::parse(self.context, &air_text)
    }

    fn process_module(&self, module: Module) -> Result<Module> {
        let plan = self.resolved_plan()?;
        if self.config.verify {
            module.verify()?;
        }

        PassManager::from_plan(self.context, plan)?.run(&module)?;

        // Strip MLIR pass statistics before verification and serialization so
        // they do not become artifact data.
        // SAFETY: `module.as_ptr()` remains valid for this borrow and the FFI
        // only removes transient compiler statistics from module attributes.
        unsafe {
            crate::ffi::apxm_module_strip_all_pass_stats(module.as_ptr());
        }

        if self.config.verify {
            module.verify()?;
        }
        Ok(module)
    }

    fn process_module_with_diagnostics(
        &self,
        module: Module,
    ) -> Result<(Module, PipelineDiagnostics)> {
        let plan = self.resolved_plan()?;
        if self.config.verify {
            module.verify()?;
        }

        let pm = PassManager::from_plan(self.context, plan.clone())?;
        let diagnostics = pm.run_plan_with_metrics(&module, &plan)?;

        if self.config.verify {
            module.verify()?;
        }
        Ok((module, diagnostics))
    }

    /// Return the immutable configuration used to derive this pipeline's plan.
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    fn resolved_plan(&self) -> Result<PipelinePlan> {
        let plan = resolve_pipeline_plan(&self.config);
        reject_unavailable_stages(&plan)?;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passes::{
        ConvergenceStatus, O3_MAX_CLEANUP_ITERATIONS, PipelineStageKind, PipelineStageStatus,
    };

    const SIMPLE_AIR: &str = r#"
module {
  func.func @pipeline_diagnostics() -> !ais.token attributes {ais.entry} {
    %answer = ais.ask "Answer concisely." : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

    #[test]
    fn diagnostics_distinguish_mlir_execution_from_deferred_artifact_stages() {
        let context = Context::new().expect("compiler context");
        let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O3);

        let (_, diagnostics) = pipeline
            .compile_with_diagnostics(SIMPLE_AIR)
            .expect("O3 pipeline compiles");

        let convergence = diagnostics
            .convergence
            .iter()
            .find(|entry| entry.group_name == "o3-cleanup")
            .expect("O3 convergence diagnostics");
        assert!(convergence.iterations > 0);
        assert!(convergence.iterations <= O3_MAX_CLEANUP_ITERATIONS);
        assert_eq!(convergence.status, ConvergenceStatus::Converged);

        let artifact_stages: Vec<_> = diagnostics
            .passes
            .iter()
            .filter(|entry| {
                matches!(
                    entry.stage_kind,
                    PipelineStageKind::ArtifactValidation | PipelineStageKind::ArtifactFinalization
                )
            })
            .collect();
        assert_eq!(artifact_stages.len(), 4);
        assert!(
            artifact_stages
                .iter()
                .all(|entry| { entry.mandatory && entry.status == PipelineStageStatus::Deferred })
        );
        assert!(
            diagnostics
                .passes
                .iter()
                .filter(|entry| entry.status == PipelineStageStatus::Deferred)
                .all(|entry| !matches!(
                    entry.stage_kind,
                    PipelineStageKind::RequiredLowering
                        | PipelineStageKind::MlirRewrite
                        | PipelineStageKind::MlirAnalysis
                        | PipelineStageKind::Diagnostic
                ))
        );
    }

    #[test]
    fn default_plan_excludes_dspy_and_explicit_dspy_fails_before_execution() {
        let context = Context::new().expect("compiler context");
        let default_pipeline = Pipeline::new(&context);
        assert!(
            !default_pipeline
                .resolved_plan()
                .expect("default plan")
                .contains_stage(DSPY_OPTIMIZE.name)
        );

        let pipeline = Pipeline::with_config(
            &context,
            PipelineConfig {
                pass_list_override: Some(vec![DSPY_OPTIMIZE.name.to_string()]),
                ..PipelineConfig::default()
            },
        );
        let error = match pipeline.compile(SIMPLE_AIR) {
            Ok(_) => panic!("DSPy must be unavailable in production compilation"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("offline evaluation workflow"));
    }
}

fn reject_unavailable_stages(plan: &PipelinePlan) -> Result<()> {
    if plan.contains_stage(DSPY_OPTIMIZE.name) {
        return Err(CompilerError::Unsupported(Box::new(Error::new_generic(
            ErrorCode::InternalError,
            "dspy-optimize is unavailable in production compilation; run prompt optimization through the offline evaluation workflow".to_string(),
        ))));
    }
    Ok(())
}
