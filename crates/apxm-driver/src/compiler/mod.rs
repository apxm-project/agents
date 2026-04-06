//! Compiler wrapper used by the driver.

use apxm_compiler::{Context, Module, Pipeline, PipelineDiagnostics};
use apxm_core::types::{OptimizationLevel, PipelineConfig};
use apxm_core::utils::build::MlirEnvReport;
use apxm_graph::semantic::{SemanticContext, validate_semantic};
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

        if matches!(ext, Some("mlir" | "air")) {
            return Err(DriverError::Driver(
                ".air is an inspection/debug format (like LLVM .ll dumps).                  It is not round-trip compilable. AgentMate should emit .ais source files.                  Use 'dekk apxm execute <file.ais>' instead.".to_string(),
            ));
        }

        let graph = self.load_graph(path)?;
        // Semantic validation runs inside load_graph for .ais files.
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

    /// Load graph input (JSON or bincode) from disk.
    pub fn load_graph(&self, path: &Path) -> Result<ApxmGraph, DriverError> {
        let bytes = fs::read(path)?;

        match path.extension().and_then(|ext| ext.to_str()) {
            Some("ais") => {
                let source = std::str::from_utf8(&bytes)
                    .map_err(|e| DriverError::Driver(format!("AIS file is not UTF-8: {e}")))?;
                let path_str = path.to_str().unwrap_or("<unknown>");
                let graph = Module::parse_dsl_graph(&self.context, source, path_str)
                    .map_err(|e| DriverError::Driver(format!("AIS parse error: {e}")))?;

                // Run semantic validation immediately after AIS parsing.
                // This surfaces incompatibilities (e.g. E511 spawn_agent in a
                // parameterized flow) as clear diagnostics before MLIR lowering
                // produces cryptic E900 internal errors.
                let sem_errors = validate_semantic(&graph, &SemanticContext::default());
                let hard_errors: Vec<_> = sem_errors
                    .iter()
                    .filter(|e| !e.code.is_warning())
                    .collect();
                if !hard_errors.is_empty() {
                    let messages: Vec<String> = hard_errors
                        .iter()
                        .map(|e| {
                            let mut msg = format!("[{}] {}", e.code.as_str(), e.short_message());
                            if let Some(help) = &e.help {
                                msg.push_str(&format!("\n  help: {}", help));
                            }
                            msg
                        })
                        .collect();
                    return Err(DriverError::Driver(format!(
                        "Compilation error: {}",
                        messages.join("\n")
                    )));
                }

                Ok(graph)
            }
            Some("air") => {
                // .air is the inspection/debug format (like LLVM .ll).
                // It is not a round-trip input format — use .ais source files instead.
                // AgentMate should emit .ais, not .air.
                Err(DriverError::Driver(
                    ".air is an inspection format, not a compile input.                      Write .ais source files instead. AgentMate emits .ais.".to_string()
                ))
            }

            _ => ApxmGraph::from_bytes(&bytes)
                .map_err(|e| DriverError::Driver(format!("Graph parse error: {e}"))),
        }
    }

    /// Emit canonical .air text IR for a graph.
    /// Analogous to LLVM .ll — human-readable, diffable, debuggable.
    pub fn emit_air(&self, graph: &ApxmGraph) -> String {
        let mut out = String::new();
        out.push_str("; Agent IR (.air) — canonical intermediate representation\n");
        out.push_str(&format!("; graph: {}\n", graph.name));
        for (k, v) in &graph.metadata {
            out.push_str(&format!("; {}: {}\n", k, v));
        }
        out.push('\n');
        if !graph.parameters.is_empty() {
            for p in &graph.parameters {
                out.push_str(&format!("; param %{}: {}\n", p.name, p.type_name));
            }
            out.push('\n');
        }
        for node in &graph.nodes {
            let op = node.op.to_string().to_lowercase();
            let mut attrs = vec![];
            for (k, v) in &node.attributes {
                if !k.starts_with('_') {
                    attrs.push(format!("{} = {}", k, v));
                }
            }
            let attr_str = if attrs.is_empty() {
                String::new()
            } else {
                format!(" {{{}}}", attrs.join(", "))
            };
            out.push_str(&format!("  %{} = ais.{}{}\n", node.name, op, attr_str));
        }
        if !graph.edges.is_empty() {
            out.push_str("\n  ; edges:\n");
            for edge in &graph.edges {
                let from = graph
                    .nodes
                    .iter()
                    .find(|n| n.id == edge.from)
                    .map(|n| n.name.as_str())
                    .unwrap_or("?");
                let to = graph
                    .nodes
                    .iter()
                    .find(|n| n.id == edge.to)
                    .map(|n| n.name.as_str())
                    .unwrap_or("?");
                out.push_str(&format!(
                    "  ; %{} -> %{} ({:?})\n",
                    from, to, edge.dependency
                ));
            }
        }
        out
    }
}
