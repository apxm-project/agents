//! This file is responsible for managing passes in the compiler.

use super::metrics::{PassMetrics, PipelineDiagnostics};
use crate::api::{Context, Module, module::invalid_input_error};
use crate::ffi;
// Reads and erases per-pass metrics after each pass, so later diagnostics only
// see the pass that just ran.
use crate::ffi::{apxm_module_drain_pass_stats, apxm_module_total_template_tokens};
use apxm_core::error::compiler::Result;
use apxm_core::types::OptimizationLevel;
use apxm_core::types::compiler::metadata::{
    BUILD_PROMPT, CANONICALIZER, CONDENSE_OPS, CSE, DEAD_CONTEXT_ELIMINATION, DSPY_OPTIMIZE,
    FUSE_ASK_OPS, NORMALIZE, PROMPT_CANONICALIZATION, SCHEDULING, SCHEMA_NARROWING, SYMBOL_DCE,
    TEMPLATE_SPECIALIZATION, UNCONSUMED_VALUE_WARNING,
};
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
        // resolve_pass_list applies pass_list_override and disable_passes on
        // top of the level/target/no_cse_llm/warn_unconsumed defaults.
        for name in super::pipeline::resolve_pass_list(config) {
            if super::pipeline::is_mlir_pass(&name) {
                pm.add_pass(&name)?;
            }
        }
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
        self.add_pass(NORMALIZE.name)
    }

    pub fn build_prompt(&mut self) -> Result<&mut Self> {
        self.add_pass(BUILD_PROMPT.name)
    }

    pub fn dspy_optimize(&mut self) -> Result<&mut Self> {
        self.add_pass(DSPY_OPTIMIZE.name)
    }

    pub fn scheduling(&mut self) -> Result<&mut Self> {
        self.add_pass(SCHEDULING.name)
    }

    pub fn fuse_ask_ops(&mut self) -> Result<&mut Self> {
        self.add_pass(FUSE_ASK_OPS.name)
    }

    pub fn condense_ops(&mut self) -> Result<&mut Self> {
        self.add_pass(CONDENSE_OPS.name)
    }

    pub fn canonicalizer(&mut self) -> Result<&mut Self> {
        self.add_pass(CANONICALIZER.name)
    }

    pub fn cse(&mut self) -> Result<&mut Self> {
        self.add_pass(CSE.name)
    }

    pub fn symbol_dce(&mut self) -> Result<&mut Self> {
        self.add_pass(SYMBOL_DCE.name)
    }

    pub fn unconsumed_value_warning(&mut self) -> Result<&mut Self> {
        self.add_pass(UNCONSUMED_VALUE_WARNING.name)
    }

    pub fn template_specialization(&mut self) -> Result<&mut Self> {
        self.add_pass(TEMPLATE_SPECIALIZATION.name)
    }

    pub fn dead_context_elimination(&mut self) -> Result<&mut Self> {
        self.add_pass(DEAD_CONTEXT_ELIMINATION.name)
    }

    pub fn schema_narrowing(&mut self) -> Result<&mut Self> {
        self.add_pass(SCHEMA_NARROWING.name)
    }

    pub fn prompt_canonicalization(&mut self) -> Result<&mut Self> {
        self.add_pass(PROMPT_CANONICALIZATION.name)
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

        // Create a scratch pass manager for individual pass execution.
        let mut tmp_pm = PassManager::new(self.context)?;

        for name in pass_names {
            tmp_pm.add_pass(name)?;

            let tokens_before = total_template_tokens(module);
            let start = Instant::now();
            tmp_pm.run(module)?;
            let elapsed = start.elapsed();
            let tokens_after = total_template_tokens(module);

            let ops_after = count_module_ops(module)?;
            let (fired_count, ir_size_delta) = drain_pass_stats(module, name);
            let tokens_saved = if tokens_before > tokens_after {
                Some(tokens_before - tokens_after)
            } else {
                None
            };

            diag.passes.push(PassMetrics {
                pass_name: name.clone(),
                duration_ms: elapsed.as_secs_f64() * 1000.0,
                ops_before: current_ops,
                ops_after,
                ops_delta: ops_after.cast_signed() - current_ops.cast_signed(),
                fired_count,
                ir_size_delta,
                tokens_saved,
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

/// Read + erase the `<pass_name>_fired_count` and `<pass_name>_ir_size_delta`
/// IntegerAttr attributes that a transform pass writes onto the module op.
///
/// Returns `(fired_count, ir_size_delta)`; either component is 0 if the pass
/// did not surface stats.
fn drain_pass_stats(module: &Module, pass_name: &str) -> (usize, isize) {
    let Ok(c_name) = CString::new(pass_name) else {
        return (0, 0);
    };
    let mut fired: i64 = 0;
    let mut ir_delta: i64 = 0;
    // SAFETY: `module.as_ptr()` is a valid `*mut ApxmModule` for the lifetime
    // of the &Module borrow. The C side only reads + erases the two named
    // string-keyed attrs and writes to the two i64 out-params.
    unsafe {
        apxm_module_drain_pass_stats(module.as_ptr(), c_name.as_ptr(), &raw mut fired, &raw mut ir_delta);
    }
    (fired.max(0) as usize, ir_delta as isize)
}

/// Sum every op's `ais.est_template_tokens` IntegerAttr across the module.
///
/// Returns 0 when no op carries the attr (e.g. before BuildPrompt has run).
/// Computes `PassMetrics::tokens_saved` as the pre/post delta around each pass.
fn total_template_tokens(module: &Module) -> usize {
    let mut total: u64 = 0;
    // SAFETY: `module.as_ptr()` is a valid `*mut ApxmModule` for the lifetime
    // of the &Module borrow. The C side only walks ops and reads the
    // `ais.est_template_tokens` IntegerAttr; no IR mutation.
    let rc = unsafe { apxm_module_total_template_tokens(module.as_ptr(), &raw mut total) };
    if rc != 0 {
        return 0;
    }
    // usize is 64-bit on the supported targets; saturate just in case.
    usize::try_from(total).unwrap_or(usize::MAX)
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
                && !l.starts_with('#')
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
