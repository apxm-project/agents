//! This file is responsible for managing passes in the compiler.

use super::metrics::{PassMetrics, PipelineDiagnostics};
use crate::api::{Context, Module, module::invalid_input_error};
use crate::ffi;
use apxm_core::error::compiler::Result;
use apxm_core::types::OptimizationLevel;
use std::ffi::CString;
use std::time::Instant;

pub struct PassManager<'ctx> {
    raw: *mut ffi::ApxmPassManager,
    context: &'ctx Context,
}

impl<'ctx> PassManager<'ctx> {
    pub fn new(context: &'ctx Context) -> Result<Self> {
        let raw = ffi::handle_null_result(
            unsafe { ffi::apxm_pass_manager_create(context.as_ptr()) },
            "pass manager creation",
        )?;

        Ok(Self { raw, context })
    }

    pub fn from_opt_level(context: &'ctx Context, level: OptimizationLevel) -> Result<Self> {
        let mut pm = Self::new(context)?;
        super::pipeline::build_pipeline(&mut pm, level)?;
        Ok(pm)
    }

    pub fn from_config(
        context: &'ctx Context,
        config: &apxm_core::types::PipelineConfig,
    ) -> Result<Self> {
        let mut pm = Self::new(context)?;
        super::pipeline::build_pipeline_with_config(
            &mut pm,
            config.opt_level,
            config.no_cse_llm,
            config.target,
        )?;
        Ok(pm)
    }

    pub fn add_pass(&mut self, name: &str) -> Result<&mut Self> {
        let c_name = CString::new(name)
            .map_err(|e| invalid_input_error(format!("Invalid pass name: {}", e)))?;

        ffi::handle_bool_result(
            unsafe { ffi::apxm_pass_manager_add_pass_by_name(self.raw, c_name.as_ptr()) },
            &format!("adding pass '{}'", name),
        )?;

        Ok(self)
    }

    pub fn normalize(&mut self) -> Result<&mut Self> {
        self.add_pass("normalize")
    }

    pub fn build_prompt(&mut self) -> Result<&mut Self> {
        self.add_pass("build-prompt")
    }

    pub fn dspy_optimize(&mut self) -> Result<&mut Self> {
        self.add_pass("dspy-optimize")
    }

    pub fn scheduling(&mut self) -> Result<&mut Self> {
        self.add_pass("scheduling")
    }

    pub fn fuse_ask_ops(&mut self) -> Result<&mut Self> {
        self.add_pass("fuse-ask-ops")
    }

    pub fn condense_ops(&mut self) -> Result<&mut Self> {
        self.add_pass("condense-ops")
    }

    pub fn canonicalizer(&mut self) -> Result<&mut Self> {
        self.add_pass("canonicalizer")
    }

    pub fn cse(&mut self) -> Result<&mut Self> {
        self.add_pass("cse")
    }

    pub fn symbol_dce(&mut self) -> Result<&mut Self> {
        self.add_pass("symbol-dce")
    }

    pub fn unconsumed_value_warning(&mut self) -> Result<&mut Self> {
        self.add_pass("unconsumed-value-warning")
    }

    pub fn template_specialization(&mut self) -> Result<&mut Self> {
        self.add_pass("template-specialization")
    }

    pub fn dead_context_elimination(&mut self) -> Result<&mut Self> {
        self.add_pass("dead-context-elimination")
    }

    pub fn schema_narrowing(&mut self) -> Result<&mut Self> {
        self.add_pass("schema-narrowing")
    }

    pub fn prompt_canonicalization(&mut self) -> Result<&mut Self> {
        self.add_pass("prompt-canonicalization")
    }

    pub fn run(&self, module: &Module) -> Result<()> {
        ffi::handle_bool_result(
            unsafe { ffi::apxm_pass_manager_run(self.raw, module.as_ptr()) },
            "pass manager execution",
        )
    }

    /// Run a sequence of passes individually, collecting per-pass metrics.
    ///
    /// Each pass is added to a fresh pass manager, executed, and then cleared
    /// so that timing and op-count deltas are isolated per pass.
    pub fn run_with_metrics(
        &self,
        module: &Module,
        pass_names: &[String],
    ) -> Result<PipelineDiagnostics> {
        let mut diag = PipelineDiagnostics::new();
        let pipeline_start = Instant::now();

        let initial_ops = count_module_ops(module)?;
        diag.initial_ops = initial_ops;

        let mut current_ops = initial_ops;

        // Create a temporary pass manager for individual pass execution.
        let mut tmp_pm = PassManager::new(self.context)?;

        for name in pass_names {
            tmp_pm.add_pass(name)?;

            let start = Instant::now();
            tmp_pm.run(module)?;
            let elapsed = start.elapsed();

            let ops_after = count_module_ops(module)?;

            diag.passes.push(PassMetrics {
                pass_name: name.clone(),
                duration_ms: elapsed.as_secs_f64() * 1000.0,
                ops_before: current_ops,
                ops_after,
                ops_delta: ops_after as isize - current_ops as isize,
            });

            current_ops = ops_after;
            tmp_pm.clear();
        }

        diag.final_ops = current_ops;
        diag.total_duration_ms = pipeline_start.elapsed().as_secs_f64() * 1000.0;

        Ok(diag)
    }

    pub fn clear(&mut self) {
        unsafe {
            ffi::apxm_pass_manager_clear(self.raw);
        }
    }
}

/// Count the number of MLIR operations in a module by scanning its textual
/// representation for lines that contain an operation mnemonic (lines with `=`
/// or that start with a known dialect prefix like `ais.`).
///
/// This is an approximation: it counts non-empty, non-brace, non-comment
/// lines within function bodies, which closely tracks the actual op count
/// for the AIS dialect.
fn count_module_ops(module: &Module) -> Result<usize> {
    let text = module.to_string()?;
    let count = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with("//")
                && !l.starts_with("module")
                && !l.starts_with("func.")
                && *l != "}"
                && *l != "{"
                && *l != "})"
                && *l != "}) {"
                && !l.starts_with("#")
        })
        .filter(|l| {
            // Count lines that look like MLIR operations
            l.contains("ais.")
                || l.contains("arith.")
                || l.contains("cf.")
                || l.contains("scf.")
                || l.contains("return")
        })
        .count();
    Ok(count)
}

impl Drop for PassManager<'_> {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                ffi::apxm_pass_manager_destroy(self.raw);
            }
        }
    }
}
