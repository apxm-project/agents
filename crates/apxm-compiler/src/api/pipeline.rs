//! Pipeline API for compiling and optimizing modules.

use crate::api::{Context, Module};
use crate::passes::{PassManager, PipelineDiagnostics, build_pass_list};
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::{builder::ErrorBuilder, codes::ErrorCode};
use apxm_core::types::{OptimizationLevel, PipelineConfig};
use apxm_graph::ApxmGraph;

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

    pub fn with_opt_level(context: &'ctx Context, level: OptimizationLevel) -> Self {
        let config = PipelineConfig {
            opt_level: level,
            ..Default::default()
        };
        Self { context, config }
    }

    pub fn compile(&self, source: &str) -> Result<Module> {
        let module = Module::parse(self.context, source)?;
        self.process_module(module)
    }

    pub fn compile_dsl(&self, source: &str, filename: &str) -> Result<Module> {
        // Canonical frontend path: DSL AST -> ApxmGraph -> AIS MLIR.
        let module = Module::parse_dsl(self.context, source, filename)?;
        self.process_module(module)
    }

    /// Compile a graph input by lowering to textual MLIR first.
    ///
    /// At O2 and above, graph-level optimization passes (prompt caching and
    /// memoization hints) are applied *before* MLIR lowering so that the
    /// hint attributes are visible in the generated IR.
    ///
    /// If a profile path is configured, the profile is loaded and applied
    /// to the graph before MLIR lowering (at any optimization level).
    pub fn compile_graph(&self, graph: &ApxmGraph) -> Result<Module> {
        let mut graph = graph.clone();

        // Apply profile-guided annotations if a profile is configured.
        if let Some(ref profile_path) = self.config.profile_path {
            let profile =
                crate::passes::profile::ExecutionProfile::load_from_file(profile_path)
                    .map_err(|e| {
                        CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                            ErrorCode::InternalError,
                            format!("Failed to load profile: {e}"),
                        )))
                    })?;
            profile.apply_to_graph(&mut graph, self.config.token_budget);
        }

        // Run graph-level optimization passes at O2+.
        if matches!(
            self.config.opt_level,
            OptimizationLevel::O2 | OptimizationLevel::O3
        ) {
            graph.prompt_caching().map_err(|e| {
                CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                    ErrorCode::InternalError,
                    format!("Prompt caching pass failed: {e}"),
                )))
            })?;
            graph.memoization_hints().map_err(|e| {
                CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                    ErrorCode::InternalError,
                    format!("Memoization hints pass failed: {e}"),
                )))
            })?;
        }

        let mlir_text = graph.to_mlir().map_err(|e| {
            CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                ErrorCode::InternalError,
                format!("Graph lowering failed: {e}"),
            )))
        })?;
        let module = Module::parse(self.context, &mlir_text)?;
        self.process_module(module)
    }

    /// Compile a graph and collect per-pass diagnostics.
    ///
    /// Each pass is executed individually so that timing and op-count deltas
    /// are recorded in the returned [`PipelineDiagnostics`].
    pub fn compile_graph_with_diagnostics(
        &self,
        graph: &ApxmGraph,
    ) -> Result<(Module, PipelineDiagnostics)> {
        let mut graph = graph.clone();

        // Apply profile-guided annotations if a profile is configured.
        if let Some(ref profile_path) = self.config.profile_path {
            let profile =
                crate::passes::profile::ExecutionProfile::load_from_file(profile_path)
                    .map_err(|e| {
                        CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                            ErrorCode::InternalError,
                            format!("Failed to load profile: {e}"),
                        )))
                    })?;
            profile.apply_to_graph(&mut graph, self.config.token_budget);
        }

        // Run graph-level optimization passes at O2+.
        if matches!(
            self.config.opt_level,
            OptimizationLevel::O2 | OptimizationLevel::O3
        ) {
            graph.prompt_caching().map_err(|e| {
                CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                    ErrorCode::InternalError,
                    format!("Prompt caching pass failed: {e}"),
                )))
            })?;
            graph.memoization_hints().map_err(|e| {
                CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                    ErrorCode::InternalError,
                    format!("Memoization hints pass failed: {e}"),
                )))
            })?;
        }

        let mlir_text = graph.to_mlir().map_err(|e| {
            CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                ErrorCode::InternalError,
                format!("Graph lowering failed: {e}"),
            )))
        })?;
        let module = Module::parse(self.context, &mlir_text)?;
        self.process_module_with_diagnostics(module)
    }

    fn process_module(&self, module: Module) -> Result<Module> {
        if self.config.verify {
            module.verify()?;
        }

        let pm = PassManager::from_config(self.context, &self.config)?;
        pm.run(&module)?;

        if self.config.verify {
            module.verify()?;
        }

        Ok(module)
    }

    fn process_module_with_diagnostics(
        &self,
        module: Module,
    ) -> Result<(Module, PipelineDiagnostics)> {
        if self.config.verify {
            module.verify()?;
        }

        let pm = PassManager::new(self.context)?;
        let pass_names = build_pass_list(self.config.opt_level, self.config.no_cse_llm);
        let diagnostics = pm.run_with_metrics(&module, &pass_names)?;

        if self.config.verify {
            module.verify()?;
        }

        Ok((module, diagnostics))
    }

    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }
}
