//! Compiler wrapper used by the driver.

use apxm_compiler::AirModule;
use apxm_compiler::{Context, Module, Pipeline, PipelineDiagnostics};
use apxm_core::types::{OptimizationLevel, PipelineConfig};
use apxm_core::utils::build::MlirEnvReport;
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
                ".mlir is a low-level format emitted by the compiler. Use .air for compilation."
                    .to_string(),
            ));
        }

        // .air files: parse as MLIR text and run optimization pipeline
        if matches!(ext, Some("air")) {
            let air_text = fs::read_to_string(path)?;
            let pipeline = Pipeline::with_opt_level(&self.context, self.opt_level);
            let module = pipeline.compile(&air_text).map_err(DriverError::Compiler)?;
            return Ok(module);
        }

        let module = self.load_graph(path)?;
        self.compile_graph(&module)
    }

    /// Validate model allowlist for an AIR module.
    ///
    /// This is a static validation that doesn't require a compiler context.
    /// If `allowlist` is None, validation passes silently (backward compatible).
    pub fn validate_model_allowlist(
        module: &AirModule,
        allowlist: Option<&Vec<String>>,
    ) -> Result<(), DriverError> {
        use apxm_core::error::compiler::CompilerError;
        use apxm_core::error::span::Span;
        use apxm_core::error::{Error, ErrorCode};
        use std::collections::{HashMap, HashSet};

        let Some(allowed_models) = allowlist else {
            return Ok(());
        };

        let allowed_set: HashSet<&str> = allowed_models.iter().map(String::as_str).collect();
        let mut seen_models: HashMap<String, Vec<String>> = HashMap::new();

        for node in &module.nodes {
            if let Some(model_value) = node.attributes.get("model") {
                let Some(model) = model_value.as_str() else {
                    continue;
                };
                if model.is_empty() || model == "null" {
                    continue;
                }
                if !allowed_set.contains(model) {
                    seen_models
                        .entry(model.to_string())
                        .or_default()
                        .push(node.name.clone());
                }
            }
        }

        if seen_models.is_empty() {
            return Ok(());
        }

        let mut violations: Vec<String> = Vec::new();
        for (model, nodes) in seen_models {
            let node_list = if nodes.len() <= 3 {
                nodes.join(", ")
            } else {
                format!("{}, {} and {} more", nodes[0], nodes[1], nodes.len() - 2)
            };
            violations.push(format!(
                "Model '{}' not in allowlist (used by node{})",
                model,
                if nodes.len() == 1 {
                    format!(": {}", nodes[0])
                } else {
                    format!("s: {}", node_list)
                }
            ));
        }

        let error_msg = format!(
            "Model allowlist validation failed:\n\n{}\n\nAllowed models: {}",
            violations.join("\n"),
            if allowed_models.is_empty() {
                "(empty allowlist)".to_string()
            } else if allowed_models.len() <= 5 {
                allowed_models.join(", ")
            } else {
                format!(
                    "{}, {} and {} more",
                    allowed_models[0],
                    allowed_models[1],
                    allowed_models.len() - 2
                )
            }
        );

        Err(DriverError::Compiler(CompilerError::Verification(
            Box::new(Error::new(
                ErrorCode::InvalidConfiguration,
                error_msg,
                Span::new("<graph>".to_string(), 1, 1, 0),
            )),
        )))
    }

    /// Compile an in-memory AIR module by emitting .air text and running optimizer passes.
    pub fn compile_graph(&self, module: &AirModule) -> Result<Module, DriverError> {
        let air_text = module
            .to_air()
            .map_err(|e| DriverError::Driver(format!("AIR emission failed: {}", e)))?;
        let pipeline = Pipeline::with_opt_level(&self.context, self.opt_level);
        pipeline.compile(&air_text).map_err(DriverError::Compiler)
    }

    /// Compile an in-memory AIR module with a custom pipeline configuration.
    pub fn compile_graph_with_config(
        &self,
        module: &AirModule,
        config: PipelineConfig,
    ) -> Result<Module, DriverError> {
        let air_text = module
            .to_air()
            .map_err(|e| DriverError::Driver(format!("AIR emission failed: {}", e)))?;
        let pipeline = Pipeline::with_config(&self.context, config);
        pipeline.compile(&air_text).map_err(DriverError::Compiler)
    }

    /// Compile an in-memory AIR module and collect per-pass diagnostics.
    pub fn compile_graph_with_diagnostics(
        &self,
        module: &AirModule,
    ) -> Result<(Module, PipelineDiagnostics), DriverError> {
        let pipeline = Pipeline::with_opt_level(&self.context, self.opt_level);
        pipeline
            .compile_graph_with_diagnostics(module)
            .map_err(DriverError::Compiler)
    }

    /// Compile an in-memory AIR module with a custom config and collect per-pass diagnostics.
    pub fn compile_graph_with_config_and_diagnostics(
        &self,
        module: &AirModule,
        config: PipelineConfig,
    ) -> Result<(Module, PipelineDiagnostics), DriverError> {
        let pipeline = Pipeline::with_config(&self.context, config);
        pipeline
            .compile_graph_with_diagnostics(module)
            .map_err(DriverError::Compiler)
    }

    /// Load a JSON graph file from disk into an `AirModule`.
    ///
    /// For `.air` (MLIR text) inputs use [`compile()`](Self::compile) instead — those
    /// are parsed directly as MLIR without round-tripping through `AirModule`.
    pub fn load_graph(&self, path: &Path) -> Result<AirModule, DriverError> {
        if matches!(path.extension().and_then(|ext| ext.to_str()), Some("air")) {
            return Err(DriverError::Driver(
                ".air files should be compiled directly via compile(), not loaded as AirModule."
                    .to_string(),
            ));
        }

        let text = fs::read_to_string(path)?;
        serde_json::from_str::<AirModule>(&text)
            .map_err(|e| DriverError::Driver(format!("Graph parse error: {e}")))
    }

    /// Emit canonical .air text IR for a module.
    ///
    /// This produces MLIR text (the AIS dialect) that can be re-compiled.
    /// Analogous to LLVM .ll — human-readable, diffable, debuggable, and round-trippable.
    pub fn emit_air(&self, module: &AirModule) -> Result<String, DriverError> {
        module
            .to_air()
            .map_err(|e| DriverError::Driver(format!("Failed to generate .air (MLIR) text: {}", e)))
    }
}
