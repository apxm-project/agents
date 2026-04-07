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

    pub fn compile_graph(&self, graph: &ApxmGraph) -> Result<Module> {
        let module = self.lower_graph(graph)?;
        self.process_module(module)
    }

    /// Compile a graph and collect per-pass diagnostics.
    pub fn compile_graph_with_diagnostics(
        &self,
        graph: &ApxmGraph,
    ) -> Result<(Module, PipelineDiagnostics)> {
        let module = self.lower_graph(graph)?;
        self.process_module_with_diagnostics(module)
    }

    /// Apply profile annotations and graph-level optimization passes, then
    /// lower to MLIR and parse into a Module.
    ///
    /// Pass ordering at O2+:
    /// 1. constant_folding -- fold CONST_STR into THINK/ASK templates
    /// 2. prompt_caching -- detect shared system prompts
    /// 3. memoization_hints -- mark duplicate pure operations
    /// 4. parallelism_analysis -- compute max parallelism & critical path
    fn lower_graph(&self, graph: &ApxmGraph) -> Result<Module> {
        let mut graph = graph.clone();

        if let Some(ref profile_path) = self.config.profile_path {
            let profile = crate::passes::profile::ExecutionProfile::load_from_file(profile_path)
                .map_err(|e| {
                    CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                        ErrorCode::InternalError,
                        format!("Failed to load profile: {e}"),
                    )))
                })?;
            profile.apply_to_graph(&mut graph, self.config.token_budget);
        }

        if matches!(
            self.config.opt_level,
            OptimizationLevel::O2 | OptimizationLevel::O3
        ) {
            Self::run_graph_passes(&mut graph, &self.config)?;
        }

        let mlir_text = graph.to_mlir().map_err(|e| {
            CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                ErrorCode::InternalError,
                format!("Graph lowering failed: {e}"),
            )))
        })?;
        Module::parse(self.context, &mlir_text)
    }

    fn run_graph_passes(graph: &mut ApxmGraph, config: &PipelineConfig) -> Result<()> {
        macro_rules! run_pass {
            ($graph:expr, $pass:ident, $label:literal) => {
                $graph.$pass().map_err(|e| {
                    CompilerError::Unsupported(Box::new(ErrorBuilder::generic(
                        ErrorCode::InternalError,
                        format!(concat!($label, " pass failed: {}"), e),
                    )))
                })?
            };
        }

        // Set optimization target in graph metadata for passes to use
        graph.metadata.insert(
            "optimization.target".to_string(),
            apxm_core::types::Value::String(config.target.to_string().into()),
        );

        run_pass!(graph, constant_folding, "Constant folding");
        run_pass!(graph, prompt_caching, "Prompt caching");
        run_pass!(graph, memoization_hints, "Memoization hints");
        run_pass!(graph, parallelism_analysis, "Parallelism analysis");
        run_pass!(graph, vllm_priority_hints, "vLLM priority hints");
        Ok(())
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
        let pass_names = build_pass_list(
            self.config.opt_level,
            self.config.no_cse_llm,
            self.config.target,
        );
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
