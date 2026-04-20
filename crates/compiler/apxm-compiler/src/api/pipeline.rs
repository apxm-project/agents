//! Pipeline API for compiling and optimizing modules.

use crate::air_builder::AirModule;
use crate::api::{Context, Module};
use crate::passes::{PassManager, PipelineDiagnostics, build_pass_list};
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::{Error, codes::ErrorCode};
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

    pub fn compile_graph(&self, module: &AirModule) -> Result<Module> {
        let ir_module = self.lower_graph(module)?;
        self.process_module(ir_module)
    }

    /// Compile a graph and collect per-pass diagnostics.
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
                .map_err(|e| {
                    CompilerError::Unsupported(Box::new(Error::new_generic(
                        ErrorCode::InternalError,
                        format!("Failed to load profile: {e}"),
                    )))
                })?;
            profile.apply_to_module(&mut module, self.config.token_budget);
        }

        crate::token_estimate::annotate_token_estimates(&mut module);

        // Rust-side vllm_hints pass: stamps `_vllm_*` attrs on LLM nodes so
        // the runtime can splice them into LLMRequest.extra_body.apxm.* for
        // GraphAware vLLM backends. Listed in `build_pass_list()` after
        // ASSIGN_PRIORITY but dispatched here on the AirModule, since the
        // MLIR PassManager only knows about MLIR-side passes.
        if !matches!(self.config.opt_level, OptimizationLevel::O0) {
            crate::passes::vllm_hints(&mut module);
        }

        let air_text = module.to_air().map_err(|e| {
            CompilerError::Unsupported(Box::new(Error::new_generic(
                ErrorCode::InternalError,
                format!("AIR emission failed: {e}"),
            )))
        })?;
        Module::parse(self.context, &air_text)
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
