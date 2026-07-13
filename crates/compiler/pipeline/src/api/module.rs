//! Module API for the Apxm compiler.
//!
//! Modules are the building blocks of Apxm programs.
//! They contain a set of functions, variables, and types available to other modules.
//!
//! Modules can be imported using the `import` keyword, which allows the functions, variables,
//! and types defined in the imported module to be used in the current module.

use crate::analysis;
use crate::api::Context;
use crate::codegen::artifact::parse_wire_dags;
use crate::ffi;
use crate::passes::{PassMetrics, PipelineStage, PipelineStageStatus, mandatory_artifact_stages};
use apxm_artifact::{Artifact, ArtifactMetadata};
use apxm_core::error::Error;
use apxm_core::error::codes::ErrorCode;
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::types::compiler::OPTIMIZATION_SUMMARY_ARTIFACT_SECTION;
use std::ffi::CString;
use std::marker::PhantomData;
use std::os::raw::c_char;
use std::ptr;
use std::time::Instant;

pub(crate) fn invalid_input_error(message: impl Into<String>) -> CompilerError {
    CompilerError::InvalidInput(Box::new(Error::new_generic(
        ErrorCode::InternalError,
        message,
    )))
}

pub struct Module {
    raw: *mut ffi::ApxmModule,
    _context: PhantomData<Context>,
}

impl Module {
    /// # Safety
    ///
    /// The caller must ensure that `raw` is a valid pointer to an `ApxmModule`
    /// and that ownership is properly transferred to the returned `Module`.
    pub unsafe fn from_raw(raw: *mut ffi::ApxmModule) -> Self {
        Self {
            raw,
            _context: PhantomData,
        }
    }

    /// Parses the given source string into a module.
    pub fn parse(context: &Context, source: &str) -> Result<Self> {
        let c_source = CString::new(source)
            .map_err(|e| invalid_input_error(format!("Invalid source string: {}", e)))?;

        let raw = ffi::handle_null_result(
            unsafe { ffi::apxm_module_parse(context.as_ptr(), c_source.as_ptr()) },
            "module parsing",
        )?;

        Ok(unsafe { Self::from_raw(raw) })
    }

    /// Parses the given source file into a module.
    pub fn parse_file(context: &Context, path: &std::path::Path) -> Result<Self> {
        let source = std::fs::read_to_string(path).map_err(CompilerError::Io)?;
        Self::parse(context, &source)
    }

    /// Verifies the module's structure and syntax.
    pub fn verify(&self) -> Result<()> {
        ffi::handle_bool_result(
            unsafe { ffi::apxm_module_verify(self.raw) },
            "module verification",
        )
    }

    pub fn to_string(&self) -> Result<String> {
        let c_str = ffi::handle_null_result(
            unsafe { ffi::apxm_module_to_string(self.raw) },
            "module serialization",
        )?;

        let result = unsafe {
            std::ffi::CStr::from_ptr(c_str)
                .to_string_lossy()
                .into_owned()
        };

        unsafe { ffi::apxm_string_free(c_str.cast()) };

        Ok(result)
    }

    pub fn as_ptr(&self) -> *mut ffi::ApxmModule {
        self.raw
    }

    pub fn generate_artifact(&self) -> Result<Artifact> {
        self.generate_artifact_with_name(None)
    }

    pub fn generate_artifact_with_name(&self, module_name: Option<&str>) -> Result<Artifact> {
        self.generate_artifact_with_manifest(module_name, None)
    }

    pub fn generate_artifact_with_manifest(
        &self,
        module_name: Option<&str>,
        manifest: Option<&apxm_core::types::HandlerManifest>,
    ) -> Result<Artifact> {
        self.generate_artifact_with_manifest_and_caps(
            module_name,
            manifest,
            &std::collections::HashSet::new(),
        )
    }

    /// Like [`generate_artifact_with_manifest`], but `known_caps` declares
    /// capabilities the host already has registered at runtime (provider/pack
    /// blocks). They satisfy the capability-binding check (E712) alongside builtins and
    /// in-graph `REGISTER_CAPABILITY` nodes — a standalone compile passes an empty
    /// set and stays strict.
    pub fn generate_artifact_with_manifest_and_caps(
        &self,
        module_name: Option<&str>,
        manifest: Option<&apxm_core::types::HandlerManifest>,
        known_caps: &std::collections::HashSet<String>,
    ) -> Result<Artifact> {
        self.generate_artifact_with_manifest_and_caps_with_diagnostics(
            module_name,
            manifest,
            known_caps,
        )
        .map(|(artifact, _)| artifact)
    }

    /// Generate an artifact and return metrics for the mandatory artifact
    /// stages that executed after MLIR lowering.
    pub fn generate_artifact_with_manifest_and_caps_with_diagnostics(
        &self,
        module_name: Option<&str>,
        manifest: Option<&apxm_core::types::HandlerManifest>,
        known_caps: &std::collections::HashSet<String>,
    ) -> Result<(Artifact, Vec<PassMetrics>)> {
        let payload = self.emit_artifact_payload(module_name)?;
        let mut dags = parse_wire_dags(&payload)?;

        let stages = mandatory_artifact_stages();
        let dag_ops = dag_node_count(&dags);
        let mut metrics = Vec::with_capacity(stages.len());

        let start = Instant::now();
        crate::artifact_validation::validate_template_placeholders(&dags)
            .map_err(invalid_input_error)?;

        metrics.push(artifact_stage_metric(
            &stages[0],
            start.elapsed(),
            dag_ops,
            0,
        ));

        let start = Instant::now();
        crate::token_estimate::refine_token_estimates(&mut dags);

        metrics.push(artifact_stage_metric(
            &stages[1],
            start.elapsed(),
            dag_ops,
            0,
        ));

        let start = Instant::now();
        let mut warning_count = 0;
        for dag in &mut dags {
            let warnings = crate::passes::capability_binding_check_dag(dag, manifest, known_caps)?;
            warning_count += warnings.len();
            for w in &warnings {
                eprintln!("warning[{}]: {}", w.code, w.message);
            }
        }
        metrics.push(artifact_stage_metric(
            &stages[2],
            start.elapsed(),
            dag_ops,
            warning_count,
        ));

        let start = Instant::now();
        for dag in &mut dags {
            crate::passes::bind_python_handlers_to_dag(dag)?;
        }
        metrics.push(artifact_stage_metric(
            &stages[3],
            start.elapsed(),
            dag_ops,
            0,
        ));

        // Find entry DAG for metadata naming
        let entry_dag = dags.iter().find(|d| d.metadata.is_entry);
        let name = entry_dag
            .and_then(|d| d.metadata.name.clone())
            .or_else(|| dags.first().and_then(|d| d.metadata.name.clone()));

        let metadata = ArtifactMetadata::new(name, crate::VERSION);
        let summary = analysis::summarize(&dags);
        let summary = serde_json::to_vec(&summary).map_err(|error| {
            invalid_input_error(format!(
                "optimization summary serialization failed: {error}"
            ))
        })?;
        let mut artifact = Artifact::new(metadata, dags);
        artifact.replace_section(OPTIMIZATION_SUMMARY_ARTIFACT_SECTION, summary);
        Ok((artifact, metrics))
    }

    pub fn generate_artifact_bytes(&self) -> Result<Vec<u8>> {
        self.generate_artifact_bytes_with_name(None)
    }

    /// Artifact bytes with host-declared `known_caps` for the capability-binding check.
    pub fn generate_artifact_bytes_with_known_caps(
        &self,
        known_caps: &std::collections::HashSet<String>,
    ) -> Result<Vec<u8>> {
        self.generate_artifact_with_manifest_and_caps(None, None, known_caps)?
            .to_bytes()
            .map_err(|err| invalid_input_error(err.to_string()))
    }

    pub fn generate_artifact_bytes_with_name(&self, module_name: Option<&str>) -> Result<Vec<u8>> {
        self.generate_artifact_with_name(module_name)?
            .to_bytes()
            .map_err(|err| invalid_input_error(err.to_string()))
    }

    pub fn generate_artifact_to_path<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let bytes = self.generate_artifact_bytes()?;
        std::fs::write(path, &bytes).map_err(CompilerError::Io)
    }

    fn emit_artifact_payload(&self, module_name: Option<&str>) -> Result<Vec<u8>> {
        let module_name_cstr = module_name
            .map(|name| {
                CString::new(name)
                    .map_err(|e| invalid_input_error(format!("Invalid module name: {e}")))
            })
            .transpose()?;

        let c_options = ffi::ApxmArtifactOptions {
            module_name: module_name_cstr
                .as_ref()
                .map_or(ptr::null(), |c| c.as_ptr()),
            emit_debug_json: false,
            target_version: ptr::null(),
        };

        let raw = ffi::handle_null_result(
            unsafe { ffi::apxm_codegen_emit_artifact(self.raw, &raw const c_options) },
            "artifact generation",
        )?;

        unsafe { copy_artifact_buffer(raw) }
    }

    /// Emit the current execution DAG snapshot for compiler-private analysis.
    ///
    /// This deliberately stops before artifact validation and finalization so
    /// the pass manager can materialize required evidence at an MLIR boundary
    /// without treating a provisional artifact as executable output.
    pub(crate) fn analysis_dags(&self) -> Result<Vec<apxm_core::types::execution::ExecutionDag>> {
        let payload = self.emit_artifact_payload(None)?;
        parse_wire_dags(&payload)
    }
}

fn dag_node_count(dags: &[apxm_core::types::execution::ExecutionDag]) -> usize {
    dags.iter().map(|dag| dag.nodes.len()).sum()
}

fn artifact_stage_metric(
    stage: &PipelineStage,
    elapsed: std::time::Duration,
    dag_ops: usize,
    fired_count: usize,
) -> PassMetrics {
    PassMetrics {
        pass_name: stage.name.clone(),
        stage_kind: stage.kind,
        iteration: None,
        status: PipelineStageStatus::Executed,
        mandatory: stage.mandatory,
        duration_ms: elapsed.as_secs_f64() * 1000.0,
        ops_before: dag_ops,
        ops_after: dag_ops,
        ops_delta: 0,
        fired_count,
        ir_size_delta: 0,
        tokens_saved: None,
    }
}

unsafe fn copy_artifact_buffer(ptr: *mut c_char) -> Result<Vec<u8>> {
    struct BufferGuard(*mut c_char);

    impl Drop for BufferGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { ffi::apxm_codegen_free(self.0) };
            }
        }
    }

    let guard = BufferGuard(ptr);
    if guard.0.is_null() {
        return Err(invalid_input_error("Artifact emitter returned null buffer"));
    }

    let mut len_buf = [0u8; 8];
    unsafe {
        len_buf.copy_from_slice(std::slice::from_raw_parts(guard.0 as *const u8, 8));
    }
    let len = u64::from_le_bytes(len_buf) as usize;

    let data = unsafe {
        let data_ptr = guard.0.add(std::mem::size_of::<u64>()) as *const u8;
        std::slice::from_raw_parts(data_ptr, len)
    };
    Ok(data.to_vec())
}

impl Drop for Module {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                ffi::apxm_module_destroy(self.raw);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Pipeline;
    use crate::passes::{PipelineStageKind, PipelineStageStatus};
    use apxm_core::types::OptimizationLevel;
    use apxm_core::types::compiler::{
        OPTIMIZATION_SUMMARY_ARTIFACT_SECTION, OptimizationSummaryV1,
    };

    const SIMPLE_AIR: &str = r#"
module {
  func.func @artifact_finalizers() -> !ais.token attributes {ais.entry} {
    %answer = ais.ask "Answer concisely." : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

    #[test]
    fn artifact_finalizers_report_the_stages_that_executed() {
        let context = Context::new().expect("compiler context");
        let module = Pipeline::with_opt_level(&context, OptimizationLevel::O2)
            .compile(SIMPLE_AIR)
            .expect("compile module");

        let (_artifact, metrics) = module
            .generate_artifact_with_manifest_and_caps_with_diagnostics(
                None,
                None,
                &std::collections::HashSet::new(),
            )
            .expect("generate artifact");

        assert_eq!(metrics.len(), 4);
        assert!(
            metrics.iter().all(|metric| {
                metric.status == PipelineStageStatus::Executed && metric.mandatory
            })
        );
        assert_eq!(
            metrics
                .iter()
                .map(|metric| metric.stage_kind)
                .collect::<Vec<_>>(),
            vec![
                PipelineStageKind::ArtifactValidation,
                PipelineStageKind::ArtifactFinalization,
                PipelineStageKind::ArtifactValidation,
                PipelineStageKind::ArtifactFinalization,
            ]
        );
    }

    #[test]
    fn artifact_contains_a_versioned_optimization_summary() {
        let context = Context::new().expect("compiler context");
        let module = Pipeline::with_opt_level(&context, OptimizationLevel::O2)
            .compile(SIMPLE_AIR)
            .expect("compile module");
        let artifact = module.generate_artifact().expect("generate artifact");

        let round_tripped = Artifact::from_bytes(
            &artifact
                .to_bytes()
                .expect("serialize artifact with summary"),
        )
        .expect("deserialize artifact with summary");
        let bytes = round_tripped
            .section_data(OPTIMIZATION_SUMMARY_ARTIFACT_SECTION)
            .expect("optimization summary section");
        let summary: OptimizationSummaryV1 =
            serde_json::from_slice(bytes).expect("deserialize optimization summary");
        assert_eq!(summary.schema_version, 1);
        assert_eq!(summary.dags.len(), 1);
        assert!(
            summary.dags[0]
                .nodes
                .iter()
                .any(|node| node.operation == "ask")
        );
    }
}
