//! Compiler wrapper used by the driver.

use apxm_compiler::{Context, Module, Pipeline, PipelineDiagnostics};
use apxm_core::types::{OptimizationLevel, PipelineConfig};
use apxm_core::utils::build::MlirEnvReport;
use apxm_graph::ApxmGraph;
use std::fs;
use std::path::Path;

use crate::error::DriverError;

/// Compiler wrapper used by the linker.
pub struct Compiler {
    context: Context,
    opt_level: OptimizationLevel,
}

impl Compiler {
    /// Initialize a new compiler context for linking.
    pub fn new() -> Result<Self, DriverError> {
        Self::with_opt_level(OptimizationLevel::O1)
    }

    /// Initialize a new compiler context with a specific optimization level.
    pub fn with_opt_level(opt_level: OptimizationLevel) -> Result<Self, DriverError> {
        let report = MlirEnvReport::detect();
        report.apply_env();
        if !report.is_ready() {
            return Err(DriverError::Driver(format!(
                "MLIR toolchain not detected.\n{}\nSet MLIR_DIR/MLIR_PREFIX/LLVM_PREFIX/CONDA_PREFIX or ensure mlir-tblgen is on PATH.",
                report.summary()
            )));
        }

        let context = Context::new().map_err(DriverError::Compiler)?;
        Ok(Self { context, opt_level })
    }

    /// Get the current optimization level.
    pub fn opt_level(&self) -> OptimizationLevel {
        self.opt_level
    }

    /// Compile a graph file into a compiler module.
    pub fn compile(&self, path: &Path) -> Result<Module, DriverError> {
        let ext = path.extension().and_then(|ext| ext.to_str());

        if matches!(ext, Some("mlir")) {
            return Err(DriverError::Driver(
                ".mlir is a low-level format emitted by the compiler. Use .apxm or .air for compilation.".to_string(),
            ));
        }

        // .air files: try direct MLIR parse first (new format), fallback to custom parser (old format)
        if matches!(ext, Some("air")) {
            let air_text = fs::read_to_string(path)?;

            // Detect format: new .air files start with "module {", old ones with "; Agent IR"
            if air_text.trim_start().starts_with("module") || air_text.trim_start().starts_with("func.func") {
                // New format: valid MLIR — parse directly
                let module = Module::parse(&self.context, &air_text)
                    .map_err(|e| DriverError::Driver(format!("MLIR parse error in .air file: {e}")))?;
                return Ok(module);
            } else {
                // Old format: custom .air text — parse to ApxmGraph, then compile
                let graph = ApxmGraph::from_air(&air_text)
                    .map_err(|e| DriverError::Driver(format!(".air parse error: {e}")))?;
                return self.compile_graph(&graph);
            }
        }

        let graph = self.load_graph(path)?;
        self.compile_graph(&graph)
    }

    /// Validate model allowlist for a graph.
    ///
    /// This is a static validation that doesn't require a compiler context.
    /// If `allowlist` is None, validation passes silently (backward compatible).
    pub fn validate_model_allowlist(
        graph: &ApxmGraph,
        allowlist: Option<&Vec<String>>,
    ) -> Result<(), DriverError> {
        apxm_compiler::passes::validate_model_allowlist(graph, allowlist)
            .map_err(DriverError::Compiler)
    }

    /// Compile an in-memory graph by lowering to MLIR and running optimizer passes.
    pub fn compile_graph(&self, graph: &ApxmGraph) -> Result<Module, DriverError> {
        let pipeline = Pipeline::with_opt_level(&self.context, self.opt_level);
        pipeline.compile_graph(graph).map_err(DriverError::Compiler)
    }

    /// Compile an in-memory graph with a custom pipeline configuration.
    pub fn compile_graph_with_config(
        &self,
        graph: &ApxmGraph,
        config: PipelineConfig,
    ) -> Result<Module, DriverError> {
        let pipeline = Pipeline::with_config(&self.context, config);
        pipeline.compile_graph(graph).map_err(DriverError::Compiler)
    }

    /// Compile an in-memory graph and collect per-pass diagnostics.
    pub fn compile_graph_with_diagnostics(
        &self,
        graph: &ApxmGraph,
    ) -> Result<(Module, PipelineDiagnostics), DriverError> {
        let pipeline = Pipeline::with_opt_level(&self.context, self.opt_level);
        pipeline
            .compile_graph_with_diagnostics(graph)
            .map_err(DriverError::Compiler)
    }

    /// Compile an in-memory graph with a custom config and collect per-pass diagnostics.
    pub fn compile_graph_with_config_and_diagnostics(
        &self,
        graph: &ApxmGraph,
        config: PipelineConfig,
    ) -> Result<(Module, PipelineDiagnostics), DriverError> {
        let pipeline = Pipeline::with_config(&self.context, config);
        pipeline
            .compile_graph_with_diagnostics(graph)
            .map_err(DriverError::Compiler)
    }

    /// Load graph input (JSON or .air) from disk.
    ///
    /// NOTE: New .air files (valid MLIR) should use compile() directly, which parses
    /// them as MLIR without converting to ApxmGraph. This method only handles old-format
    /// .air files and JSON files that need to be converted to ApxmGraph.
    pub fn load_graph(&self, path: &Path) -> Result<ApxmGraph, DriverError> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("air") => {
                let text = fs::read_to_string(path)?;

                // New .air files (valid MLIR) cannot be converted back to ApxmGraph
                // — they should go through compile() instead
                if text.trim_start().starts_with("module") || text.trim_start().starts_with("func.func") {
                    return Err(DriverError::Driver(
                        "This .air file uses the new MLIR format and cannot be loaded as ApxmGraph. \
                         Use compile() directly instead of load_graph().".to_string()
                    ));
                }

                ApxmGraph::from_air(&text)
                    .map_err(|e| DriverError::Driver(format!(".air parse error: {e}")))
            }

            _ => {
                let bytes = fs::read(path)?;
                ApxmGraph::from_bytes(&bytes)
                    .map_err(|e| DriverError::Driver(format!("Graph parse error: {e}")))
            }
        }
    }

    /// Emit canonical .air text IR for a graph.
    ///
    /// This produces MLIR text (the AIS dialect) that can be re-compiled.
    /// Analogous to LLVM .ll — human-readable, diffable, debuggable, and round-trippable.
    pub fn emit_air(&self, graph: &ApxmGraph) -> Result<String, DriverError> {
        // Lower the graph to MLIR text format
        graph.to_mlir().map_err(|e| {
            DriverError::Driver(format!("Failed to generate .air (MLIR) text: {}", e))
        })
    }
}
