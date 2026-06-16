//! Pipeline API for compiling and optimizing modules.

use crate::air_builder::AirModule;
use crate::api::{Context, Module};
use crate::optimization::CompilerOptimizationContext;
use crate::passes::{PassManager, PipelineDiagnostics, resolve_pass_list};
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::{Error, codes::ErrorCode};
use apxm_core::types::compiler::metadata::{BUILD_PROMPT, DEAD_CONTEXT_ELIMINATION, DSPY_OPTIMIZE};
use apxm_core::types::{OptimizationLevel, PipelineConfig};
use std::ffi::CString;

/// Pipeline API for compiling and optimizing modules.
pub struct Pipeline<'ctx> {
    context: &'ctx Context,
    config: PipelineConfig,
    optimization_context: CompilerOptimizationContext,
}

impl<'ctx> Pipeline<'ctx> {
    /// Creates a new pipeline with default configuration.
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            config: PipelineConfig::default(),
            optimization_context: CompilerOptimizationContext::default(),
        }
    }

    /// Creates a new pipeline with custom configuration.
    pub fn with_config(context: &'ctx Context, config: PipelineConfig) -> Self {
        let optimization_context = CompilerOptimizationContext::from_pipeline_config(&config);
        Self {
            context,
            config,
            optimization_context,
        }
    }

    pub fn with_opt_level(context: &'ctx Context, level: OptimizationLevel) -> Self {
        let config = PipelineConfig {
            opt_level: level,
            ..Default::default()
        };
        Self::with_config(context, config)
    }

    /// Creates a pipeline with a caller-provided compiler optimization context.
    pub fn with_config_and_optimization_context(
        context: &'ctx Context,
        config: PipelineConfig,
        optimization_context: CompilerOptimizationContext,
    ) -> Self {
        Self {
            context,
            config,
            optimization_context,
        }
    }

    pub fn compile(&self, source: &str) -> Result<Module> {
        let module = Module::parse(self.context, source)?;
        self.process_module(module)
    }

    /// Compile pre-built AIR text and collect per-pass diagnostics.
    ///
    /// Mirror of [`compile`] for callers that need the diagnostics array,
    /// e.g. the CLI's `--emit-diagnostics` flag and the ablation harness.
    pub fn compile_with_diagnostics(&self, source: &str) -> Result<(Module, PipelineDiagnostics)> {
        let module = Module::parse(self.context, source)?;
        self.process_module_with_diagnostics(module)
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

        let air_text = module.to_air().map_err(|e| {
            CompilerError::Unsupported(Box::new(Error::new_generic(
                ErrorCode::InternalError,
                format!("AIR emission failed: {e}"),
            )))
        })?;
        Module::parse(self.context, &air_text)
    }

    fn process_module(&self, module: Module) -> Result<Module> {
        let pass_names = self.resolved_pass_names()?;
        self.apply_transient_module_config(&module, &pass_names)?;

        if self.config.verify {
            module.verify()?;
        }

        let pm = self.pass_manager_from_names(&pass_names)?;
        pm.run(&module)?;

        // Strip per-pass stat attributes (`ais.<pass>_fired_count`,
        // `ais.<pass>_ir_size_delta`) before verification + serialization so
        // they don't bake into the artifact and break golden-roundtrip /
        // idempotency checks. The diagnostics path drains them per-pass via
        // `drain_pass_stats` instead.
        // SAFETY: `module.as_ptr()` is a valid `*mut ApxmModule` for the
        // lifetime of this borrow; the C side only mutates module attrs.
        unsafe {
            crate::ffi::apxm_module_strip_all_pass_stats(module.as_ptr());
        }
        self.strip_transient_module_config(&module)?;

        if self.config.verify {
            module.verify()?;
        }

        Ok(module)
    }

    fn process_module_with_diagnostics(
        &self,
        module: Module,
    ) -> Result<(Module, PipelineDiagnostics)> {
        let pass_names = self.resolved_pass_names()?;
        self.apply_transient_module_config(&module, &pass_names)?;

        if self.config.verify {
            module.verify()?;
        }

        let pm = PassManager::new(self.context)?;
        // resolve_pass_list applies pass_list_override and disable_passes on
        // top of the level/target/no_cse_llm/warn_unconsumed defaults so the
        // diagnostics path agrees with PassManager::from_config. Rust-only
        // passes (tool-binding-check, bind-tool-handlers) are dispatched outside
        // the MLIR pass manager and must not reach it here.
        let pass_names: Vec<String> = pass_names
            .into_iter()
            .filter(|n| crate::passes::is_mlir_pass(n))
            .collect();
        let diagnostics = pm.run_with_metrics(&module, &pass_names)?;
        self.strip_transient_module_config(&module)?;

        if self.config.verify {
            module.verify()?;
        }

        Ok((module, diagnostics))
    }

    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    fn resolved_pass_names(&self) -> Result<Vec<String>> {
        let mut pass_names = resolve_pass_list(&self.config);
        let dspy_name = DSPY_OPTIMIZE.name;

        if self.config.pass_list_override.is_none()
            && self.config.opt_level != OptimizationLevel::O0
            && !pass_names.iter().any(|name| name == dspy_name)
            && self.optimization_context.prompt_optimization_configured()?
        {
            // Insert dspy-optimize AFTER dead-context-elimination so DCE has
            // already pruned unconsumed Asks; otherwise DSPy spends API budget
            // rewriting prompts that DCE is about to remove. Fall back to
            // (build-prompt + 1) only if DCE is not in the pass list, such as
            // O0 with a custom override.
            let insert_at = pass_names
                .iter()
                .position(|name| name == DEAD_CONTEXT_ELIMINATION.name)
                .or_else(|| pass_names.iter().position(|name| name == BUILD_PROMPT.name))
                .map(|index| index + 1)
                .unwrap_or(pass_names.len());
            pass_names.insert(insert_at, dspy_name.to_string());
        }

        Ok(pass_names)
    }

    fn pass_manager_from_names(&self, pass_names: &[String]) -> Result<PassManager<'ctx>> {
        let mut pm = PassManager::new(self.context)?;
        for name in pass_names {
            if crate::passes::is_mlir_pass(name) {
                pm.add_pass(name)?;
            }
        }
        Ok(pm)
    }

    fn apply_transient_module_config(&self, module: &Module, pass_names: &[String]) -> Result<()> {
        use apxm_core::constants::dspy;

        if !pass_names.iter().any(|name| name == DSPY_OPTIMIZE.name) {
            return Ok(());
        }

        let Some(prompt_optimization) = self.optimization_context.prompt_optimization()? else {
            return Ok(());
        };

        set_module_string_attr(
            module,
            dspy::ATTR_TRAINING_DATA_PATH,
            &prompt_optimization.training_data_path.to_string_lossy(),
        )?;
        set_module_string_attr(
            module,
            dspy::ATTR_OPTIMIZER,
            prompt_optimization.optimizer.as_str(),
        )?;
        set_module_string_attr(module, dspy::ATTR_AUTO, prompt_optimization.budget.as_str())?;
        set_module_string_attr(
            module,
            dspy::ATTR_METRIC,
            prompt_optimization.metric.as_str(),
        )?;
        set_module_string_attr(
            module,
            dspy::ATTR_BACKEND_JSON,
            &prompt_optimization.backend_json,
        )?;
        set_module_string_attr(
            module,
            dspy::ATTR_CACHE_DIR,
            &prompt_optimization.cache_dir.to_string_lossy(),
        )?;
        if prompt_optimization.no_cache {
            set_module_bool_attr(module, dspy::ATTR_NO_CACHE, true)?;
        }
        Ok(())
    }

    fn strip_transient_module_config(&self, module: &Module) -> Result<()> {
        use apxm_core::constants::dspy;

        for attr in [
            dspy::ATTR_TRAINING_DATA_PATH,
            dspy::ATTR_BACKEND_JSON,
            dspy::ATTR_CACHE_DIR,
            dspy::ATTR_OPTIMIZER,
            dspy::ATTR_AUTO,
            dspy::ATTR_METRIC,
            dspy::ATTR_NO_CACHE,
        ] {
            remove_module_attr(module, attr)?;
        }
        Ok(())
    }
}

fn ffi_attr_error(action: &str, name: &str) -> CompilerError {
    CompilerError::Unsupported(Box::new(Error::new_generic(
        ErrorCode::InternalError,
        format!("Failed to {action} transient module attribute {name}"),
    )))
}

fn set_module_string_attr(module: &Module, name: &str, value: &str) -> Result<()> {
    let c_name = CString::new(name).map_err(|_| ffi_attr_error("set", name))?;
    let c_value = CString::new(value).map_err(|_| ffi_attr_error("set", name))?;
    let ok = unsafe {
        crate::ffi::apxm_module_set_string_attr(module.as_ptr(), c_name.as_ptr(), c_value.as_ptr())
    };
    if ok {
        Ok(())
    } else {
        Err(ffi_attr_error("set", name))
    }
}

fn set_module_bool_attr(module: &Module, name: &str, value: bool) -> Result<()> {
    let c_name = CString::new(name).map_err(|_| ffi_attr_error("set", name))?;
    let ok =
        unsafe { crate::ffi::apxm_module_set_bool_attr(module.as_ptr(), c_name.as_ptr(), value) };
    if ok {
        Ok(())
    } else {
        Err(ffi_attr_error("set", name))
    }
}

fn remove_module_attr(module: &Module, name: &str) -> Result<()> {
    let c_name = CString::new(name).map_err(|_| ffi_attr_error("remove", name))?;
    let ok = unsafe { crate::ffi::apxm_module_remove_attr(module.as_ptr(), c_name.as_ptr()) };
    if ok {
        Ok(())
    } else {
        Err(ffi_attr_error("remove", name))
    }
}
