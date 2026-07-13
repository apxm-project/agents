//! MLIR pass-manager execution for typed compiler plans.

use super::metrics::{
    ConvergenceDiagnostics, ConvergenceStatus, PassMetrics, PipelineDiagnostics,
    PipelineStageStatus,
};
use super::plan::{PipelinePlan, PipelinePlanStep, PipelineStage, PipelineStageKind};
use crate::analysis::AnalysisStore;
use crate::api::{Context, Module, module::invalid_input_error};
use crate::ffi;
use crate::ffi::{apxm_module_drain_pass_stats, apxm_module_total_template_tokens};
use apxm_core::error::compiler::Result;
use apxm_core::types::OptimizationLevel;
use apxm_core::types::compiler::CompilerAnalysisKind;
use apxm_core::types::compiler::metadata::{
    BUILD_PROMPT, CANONICALIZER, CONDENSE_OPS, CSE, DEAD_CONTEXT_ELIMINATION, DSPY_OPTIMIZE,
    FUSE_ASK_OPS, NORMALIZE, PROMPT_CANONICALIZATION, PURE_DEAD_NODE_ELIMINATION, SCHEDULING,
    SCHEMA_NARROWING, SYMBOL_DCE, TEMPLATE_SPECIALIZATION, UNCONSUMED_VALUE_WARNING,
};
use std::ffi::CString;
use std::time::Instant;

/// A bounded convergence result returned by the execution helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConvergenceResult {
    iterations: usize,
    status: ConvergenceStatus,
}

/// Cache reusable compiler evidence across typed stages of one module run.
///
/// A stage may keep only facts it explicitly preserves. All other cached
/// values are discarded, and the next required analysis is derived from a
/// fresh execution-DAG snapshot of the rewritten module.
#[derive(Default)]
struct PipelineAnalysisCache {
    store: Option<AnalysisStore>,
    snapshot_is_current: bool,
}

impl PipelineAnalysisCache {
    /// Materialize the evidence required by one stage before it executes.
    fn materialize_required(
        &mut self,
        module: &Module,
        required: &[CompilerAnalysisKind],
    ) -> Result<()> {
        if required.is_empty() {
            return Ok(());
        }

        if self.store.is_none() {
            self.store = Some(AnalysisStore::new(module.analysis_dags()?));
            self.snapshot_is_current = true;
        } else if !self.snapshot_is_current {
            let store = self.store.as_mut().expect("analysis store exists");
            store.rebase(module.analysis_dags()?);
            self.snapshot_is_current = true;
        }

        self.store
            .as_mut()
            .expect("analysis store exists")
            .materialize(required);
        Ok(())
    }

    /// Retain only evidence that the completed stage explicitly preserves.
    fn complete_stage(&mut self, stage: &PipelineStage) {
        let Some(store) = self.store.as_mut() else {
            return;
        };
        store.retain_only(&stage.preserved_analyses);
        self.snapshot_is_current = false;
    }

    #[cfg(test)]
    fn is_materialized(&self, kind: CompilerAnalysisKind) -> bool {
        self.store
            .as_ref()
            .is_some_and(|store| store.is_materialized(kind))
    }
}

/// Executes MLIR stages and, when configured, a typed [`PipelinePlan`].
pub struct PassManager<'ctx> {
    raw: *mut ffi::ApxmPassManager,
    context: &'ctx Context,
    plan: Option<PipelinePlan>,
}

impl<'ctx> PassManager<'ctx> {
    /// Create an unconfigured pass manager for direct MLIR pass registration.
    pub fn new(context: &'ctx Context) -> Result<Self> {
        let raw = ffi::handle_null_result(
            unsafe { ffi::apxm_pass_manager_create(context.as_ptr()) },
            "pass manager creation",
        )?;

        Ok(Self {
            raw,
            context,
            plan: None,
        })
    }

    /// Create a manager configured to execute a typed plan.
    pub fn from_plan(context: &'ctx Context, plan: PipelinePlan) -> Result<Self> {
        let mut pm = Self::new(context)?;
        pm.set_plan(plan);
        Ok(pm)
    }

    /// Configure a manager to execute a typed plan rather than its raw queue.
    pub fn set_plan(&mut self, plan: PipelinePlan) {
        self.clear();
        self.plan = Some(plan);
    }

    /// Configure a manager with the default typed plan for an optimization level.
    pub fn from_opt_level(context: &'ctx Context, level: OptimizationLevel) -> Result<Self> {
        Self::from_plan(
            context,
            super::pipeline::build_pipeline_plan(
                level,
                false,
                apxm_core::types::OptimizationTarget::Balanced,
            ),
        )
    }

    /// Configure a manager from the typed plan resolved from pipeline configuration.
    pub fn from_config(
        context: &'ctx Context,
        config: &apxm_core::types::PipelineConfig,
    ) -> Result<Self> {
        Self::from_plan(context, super::pipeline::resolve_pipeline_plan(config))
    }

    /// Add a direct MLIR pass to the raw manager queue.
    pub fn add_pass(&mut self, name: &str) -> Result<&mut Self> {
        self.plan = None;
        let c_name = CString::new(name)
            .map_err(|e| invalid_input_error(format!("Invalid pass name: {}", e)))?;

        ffi::handle_bool_result(
            unsafe { ffi::apxm_pass_manager_add_pass_by_name(self.raw, c_name.as_ptr()) },
            &format!("adding pass '{name}'"),
        )?;

        Ok(self)
    }

    /// Register normalize on the raw manager queue.
    pub fn normalize(&mut self) -> Result<&mut Self> {
        self.add_pass(NORMALIZE.name)
    }

    /// Register build-prompt on the raw manager queue.
    pub fn build_prompt(&mut self) -> Result<&mut Self> {
        self.add_pass(BUILD_PROMPT.name)
    }

    /// Register dspy-optimize on the raw manager queue.
    pub fn dspy_optimize(&mut self) -> Result<&mut Self> {
        self.add_pass(DSPY_OPTIMIZE.name)
    }

    /// Register fuse-ask-ops on the raw manager queue.
    pub fn fuse_ask_ops(&mut self) -> Result<&mut Self> {
        self.add_pass(FUSE_ASK_OPS.name)
    }

    /// Register cse on the raw manager queue.
    pub fn cse(&mut self) -> Result<&mut Self> {
        self.add_pass(CSE.name)
    }

    /// Register condense-ops on the raw manager queue.
    pub fn condense_ops(&mut self) -> Result<&mut Self> {
        self.add_pass(CONDENSE_OPS.name)
    }

    /// Register scheduling on the raw manager queue.
    pub fn scheduling(&mut self) -> Result<&mut Self> {
        self.add_pass(SCHEDULING.name)
    }

    /// Register canonicalizer on the raw manager queue.
    pub fn canonicalize(&mut self) -> Result<&mut Self> {
        self.add_pass(CANONICALIZER.name)
    }

    /// Register symbol-dce on the raw manager queue.
    pub fn symbol_dce(&mut self) -> Result<&mut Self> {
        self.add_pass(SYMBOL_DCE.name)
    }

    /// Register unconsumed-value-warning on the raw manager queue.
    pub fn unconsumed_value_warning(&mut self) -> Result<&mut Self> {
        self.add_pass(UNCONSUMED_VALUE_WARNING.name)
    }

    /// Register template-specialization on the raw manager queue.
    pub fn template_specialization(&mut self) -> Result<&mut Self> {
        self.add_pass(TEMPLATE_SPECIALIZATION.name)
    }

    /// Register dead-context-elimination on the raw manager queue.
    pub fn dead_context_elimination(&mut self) -> Result<&mut Self> {
        self.add_pass(DEAD_CONTEXT_ELIMINATION.name)
    }

    /// Register pure-dead-node-elimination on the raw manager queue.
    pub fn pure_dead_node_elimination(&mut self) -> Result<&mut Self> {
        self.add_pass(PURE_DEAD_NODE_ELIMINATION.name)
    }

    /// Register schema-narrowing on the raw manager queue.
    pub fn schema_narrowing(&mut self) -> Result<&mut Self> {
        self.add_pass(SCHEMA_NARROWING.name)
    }

    /// Register prompt-canonicalization on the raw manager queue.
    pub fn prompt_canonicalization(&mut self) -> Result<&mut Self> {
        self.add_pass(PROMPT_CANONICALIZATION.name)
    }

    /// Execute the configured typed plan or directly registered MLIR passes.
    pub fn run(&self, module: &Module) -> Result<()> {
        match self.plan.as_ref() {
            Some(plan) => self.run_plan(module, plan),
            None => self.run_raw(module),
        }
    }

    /// Run a legacy MLIR-only pass list and collect per-stage metrics.
    pub fn run_with_metrics(
        &self,
        module: &Module,
        pass_names: &[String],
    ) -> Result<PipelineDiagnostics> {
        let mut plan = PipelinePlan::new();
        for name in pass_names {
            plan.push_stage(PipelineStage::new(
                name,
                PipelineStageKind::MlirRewrite,
                false,
            ));
        }
        self.run_plan_with_metrics(module, &plan)
    }

    /// Execute a typed plan and collect stage-boundary diagnostics.
    pub fn run_plan_with_metrics(
        &self,
        module: &Module,
        plan: &PipelinePlan,
    ) -> Result<PipelineDiagnostics> {
        let mut diagnostics = PipelineDiagnostics::new();
        let pipeline_start = Instant::now();
        diagnostics.initial_ops = count_module_ops(module)?;
        let mut current_ops = diagnostics.initial_ops;
        let mut analyses = PipelineAnalysisCache::default();

        for step in &plan.steps {
            match step {
                PipelinePlanStep::Stage(stage) => {
                    self.execute_stage_with_metrics(
                        module,
                        stage,
                        None,
                        &mut current_ops,
                        &mut diagnostics,
                        &mut analyses,
                    )?;
                }
                PipelinePlanStep::Convergence(group) => {
                    let result = run_bounded_convergence(group.max_iterations, |iteration| {
                        let before = module.to_string()?;
                        for stage in &group.stages {
                            self.execute_stage_with_metrics(
                                module,
                                stage,
                                Some(iteration),
                                &mut current_ops,
                                &mut diagnostics,
                                &mut analyses,
                            )?;
                        }
                        Ok(before != module.to_string()?)
                    })?;
                    diagnostics.convergence.push(ConvergenceDiagnostics {
                        group_name: group.name.clone(),
                        iterations: result.iterations,
                        max_iterations: group.max_iterations,
                        status: result.status,
                    });
                }
            }
        }

        diagnostics.final_ops = current_ops;
        diagnostics.total_duration_ms = pipeline_start.elapsed().as_secs_f64() * 1000.0;
        Ok(diagnostics)
    }

    /// Clear directly registered MLIR passes and any configured typed plan.
    pub fn clear(&mut self) {
        unsafe {
            ffi::apxm_pass_manager_clear(self.raw);
        }
        self.plan = None;
    }

    fn run_plan(&self, module: &Module, plan: &PipelinePlan) -> Result<()> {
        let mut analyses = PipelineAnalysisCache::default();
        for step in &plan.steps {
            match step {
                PipelinePlanStep::Stage(stage) if stage.kind.executes_in_mlir() => {
                    self.execute_stage(module, stage, &mut analyses)?;
                }
                PipelinePlanStep::Stage(_) => {}
                PipelinePlanStep::Convergence(group) => {
                    run_bounded_convergence(group.max_iterations, |_| {
                        let before = module.to_string()?;
                        for stage in &group.stages {
                            self.execute_stage(module, stage, &mut analyses)?;
                        }
                        Ok(before != module.to_string()?)
                    })?;
                }
            }
        }
        Ok(())
    }

    fn run_mlir_stages(&self, module: &Module, stages: &[PipelineStage]) -> Result<()> {
        let mut stage_manager = PassManager::new(self.context)?;
        let mut has_mlir_stage = false;
        for stage in stages {
            if stage.kind.executes_in_mlir() {
                stage_manager.add_pass(&stage.name)?;
                has_mlir_stage = true;
            }
        }
        if has_mlir_stage {
            stage_manager.run_raw(module)?;
        }
        Ok(())
    }

    fn execute_stage_with_metrics(
        &self,
        module: &Module,
        stage: &PipelineStage,
        iteration: Option<usize>,
        current_ops: &mut usize,
        diagnostics: &mut PipelineDiagnostics,
        analyses: &mut PipelineAnalysisCache,
    ) -> Result<()> {
        if !stage.kind.executes_in_mlir() {
            diagnostics.passes.push(PassMetrics {
                pass_name: stage.name.clone(),
                stage_kind: stage.kind,
                iteration,
                status: PipelineStageStatus::Deferred,
                mandatory: stage.mandatory,
                duration_ms: 0.0,
                ops_before: *current_ops,
                ops_after: *current_ops,
                ops_delta: 0,
                fired_count: 0,
                ir_size_delta: 0,
                tokens_saved: None,
            });
            return Ok(());
        }

        let tokens_before = total_template_tokens(module);
        let start = Instant::now();
        self.execute_stage(module, stage, analyses)?;
        let elapsed = start.elapsed();
        let tokens_after = total_template_tokens(module);
        let ops_after = count_module_ops(module)?;
        let (fired_count, ir_size_delta) = drain_pass_stats(module, &stage.name);
        let tokens_saved = (tokens_before > tokens_after).then_some(tokens_before - tokens_after);

        diagnostics.passes.push(PassMetrics {
            pass_name: stage.name.clone(),
            stage_kind: stage.kind,
            iteration,
            status: PipelineStageStatus::Executed,
            mandatory: stage.mandatory,
            duration_ms: elapsed.as_secs_f64() * 1000.0,
            ops_before: *current_ops,
            ops_after,
            ops_delta: ops_after.cast_signed() - current_ops.cast_signed(),
            fired_count,
            ir_size_delta,
            tokens_saved,
        });
        *current_ops = ops_after;
        Ok(())
    }

    /// Execute one MLIR stage with its analysis boundary enforced.
    fn execute_stage(
        &self,
        module: &Module,
        stage: &PipelineStage,
        analyses: &mut PipelineAnalysisCache,
    ) -> Result<()> {
        analyses.materialize_required(module, &stage.required_analyses)?;
        self.run_mlir_stages(module, std::slice::from_ref(stage))?;
        analyses.complete_stage(stage);
        Ok(())
    }

    fn run_raw(&self, module: &Module) -> Result<()> {
        ffi::handle_bool_result(
            unsafe { ffi::apxm_pass_manager_run(self.raw, module.as_ptr()) },
            "pass manager execution",
        )
    }
}

/// Run an iteration body until it reports no change or consumes its bound.
fn run_bounded_convergence<F>(
    max_iterations: usize,
    mut run_iteration: F,
) -> Result<ConvergenceResult>
where
    F: FnMut(usize) -> Result<bool>,
{
    debug_assert!(
        max_iterations > 0,
        "convergence groups require a positive bound"
    );
    for iteration in 1..=max_iterations {
        if !run_iteration(iteration)? {
            return Ok(ConvergenceResult {
                iterations: iteration,
                status: ConvergenceStatus::Converged,
            });
        }
    }
    Ok(ConvergenceResult {
        iterations: max_iterations,
        status: ConvergenceStatus::IterationLimitReached,
    })
}

/// Read + erase pass statistics written by one MLIR transform.
fn drain_pass_stats(module: &Module, pass_name: &str) -> (usize, isize) {
    let Ok(c_name) = CString::new(pass_name) else {
        return (0, 0);
    };
    let mut fired: i64 = 0;
    let mut ir_delta: i64 = 0;
    // SAFETY: `module.as_ptr()` is valid for this borrow; the FFI only drains
    // per-pass statistics attached to the module.
    unsafe {
        apxm_module_drain_pass_stats(
            module.as_ptr(),
            c_name.as_ptr(),
            &raw mut fired,
            &raw mut ir_delta,
        );
    }
    (fired.max(0) as usize, ir_delta as isize)
}

/// Sum `ais.est_template_tokens` across the module.
fn total_template_tokens(module: &Module) -> usize {
    let mut total: u64 = 0;
    // SAFETY: `module.as_ptr()` is valid for this borrow; the FFI only reads
    // token-estimate attributes without mutating the IR.
    let rc = unsafe { apxm_module_total_template_tokens(module.as_ptr(), &raw mut total) };
    if rc != 0 {
        return 0;
    }
    usize::try_from(total).unwrap_or(usize::MAX)
}

/// Approximate the number of operations in the textual MLIR representation.
fn count_module_ops(module: &Module) -> Result<usize> {
    let text = module.to_string()?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("//")
                && !line.starts_with("module")
                && !line.starts_with("func.")
                && *line != "}"
                && *line != "{"
                && *line != "})"
                && *line != "}) {"
                && !line.starts_with('#')
        })
        .filter(|line| {
            line.contains("ais.")
                || line.contains("arith.")
                || line.contains("cf.")
                || line.contains("scf.")
                || line.contains("return")
        })
        .count())
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

#[cfg(test)]
mod analysis_cache_tests {
    use super::*;
    use apxm_core::types::compiler::CompilerAnalysisKind;
    use apxm_core::types::execution::{ExecutionDag, Node};
    use apxm_core::types::AISOperationType;

    #[test]
    fn cache_keeps_only_the_stage_contracts_explicitly_preserve() {
        let mut dag = ExecutionDag::new();
        dag.nodes.push(Node::new(1, AISOperationType::Nop));
        let mut cache = PipelineAnalysisCache {
            store: Some(AnalysisStore::new(vec![dag])),
            snapshot_is_current: true,
        };
        cache
            .store
            .as_mut()
            .expect("analysis store")
            .materialize(&[
                CompilerAnalysisKind::DagUse,
                CompilerAnalysisKind::EffectAuthority,
            ]);

        let stage = PipelineStage::new("rewrite", PipelineStageKind::MlirRewrite, false)
            .with_analysis_contract(
                &[],
                &[CompilerAnalysisKind::EffectAuthority],
                &[CompilerAnalysisKind::DagUse],
            );
        cache.complete_stage(&stage);

        assert!(!cache.is_materialized(CompilerAnalysisKind::DagUse));
        assert!(cache.is_materialized(CompilerAnalysisKind::EffectAuthority));
        assert!(!cache.snapshot_is_current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convergence_stops_at_the_first_fixed_point() {
        let mut outcomes = [true, false].into_iter();
        let result = run_bounded_convergence(10, |_| Ok(outcomes.next().unwrap()))
            .expect("convergence helper should succeed");

        assert_eq!(result.iterations, 2);
        assert_eq!(result.status, ConvergenceStatus::Converged);
    }

    #[test]
    fn convergence_reports_iteration_bound_when_changes_continue() {
        let result =
            run_bounded_convergence(3, |_| Ok(true)).expect("convergence helper should succeed");

        assert_eq!(result.iterations, 3);
        assert_eq!(result.status, ConvergenceStatus::IterationLimitReached);
    }
}
