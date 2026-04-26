//! High-level linker that orchestrates compiler and runtime execution.

use std::path::Path;
use std::sync::Arc;

use apxm_artifact::{Artifact, ArtifactSection};
use apxm_core::constants::env as apxm_env;
use apxm_core::error::runtime::RuntimeError;
use apxm_core::log_info;
use apxm_core::paths::ApxmPaths;
use apxm_core::types::OptimizationTarget;
use apxm_core::types::{OptimizationLevel, PipelineConfig};
use apxm_runtime::{ExecutionEventEmitter, RuntimeConfig, RuntimeExecutionResult};

use crate::{
    cache, compiler::Compiler, config::ApXmConfig, error::DriverError, runtime::RuntimeExecutor,
};

fn state_err(msg: impl Into<String>) -> DriverError {
    DriverError::Runtime(RuntimeError::State(msg.into()))
}

fn compiler_project_config_exists() -> bool {
    ApxmPaths::discover()
        .ok()
        .map(|paths| paths.compiler_config_path().is_file())
        .unwrap_or(false)
}

/// Linker configuration that drives compiler and runtime orchestration.
#[derive(Debug, Clone)]
pub struct LinkerConfig {
    /// APxM configuration (providers, tools, policies).
    pub apxm_config: ApXmConfig,

    /// Optional runtime configuration overrides.
    pub runtime_config: RuntimeConfig,

    /// Optimization level for compilation.
    pub opt_level: OptimizationLevel,

    /// Full compiler pipeline configuration.
    pub pipeline_config: PipelineConfig,

    /// When `true`, skip the artifact cache entirely.
    pub no_cache: bool,
}

impl LinkerConfig {
    /// Create a configuration from an `ApXmConfig` instance.
    pub fn from_apxm_config(apxm_config: ApXmConfig) -> Self {
        Self {
            apxm_config,
            runtime_config: RuntimeConfig::default(),
            opt_level: OptimizationLevel::O2,
            pipeline_config: PipelineConfig {
                opt_level: OptimizationLevel::O2,
                ..Default::default()
            },
            no_cache: false,
        }
    }

    /// Set the optimization level.
    pub fn with_opt_level(mut self, opt_level: OptimizationLevel) -> Self {
        self.opt_level = opt_level;
        self.pipeline_config.opt_level = opt_level;
        self
    }

    /// Set the full compiler pipeline configuration.
    pub fn with_pipeline_config(mut self, pipeline_config: PipelineConfig) -> Self {
        self.opt_level = pipeline_config.opt_level;
        self.pipeline_config = pipeline_config;
        self
    }
}

/// Result returned after linking compilation/runtime steps.
pub struct LinkResult {
    /// Compiled artifact that can be reused.
    pub artifact: Artifact,
    /// Runtime execution report for this artifact.
    pub execution: RuntimeExecutionResult,
    /// Metrics about compile/runtime overhead.
    #[cfg(feature = "metrics")]
    pub metrics: LinkMetrics,
    /// Compiler diagnostics (in-memory only, not persisted in .apxmobj).
    pub compiler_diagnostics: Option<serde_json::Value>,
}

/// Timing breakdown for a link+execute run.
#[cfg(feature = "metrics")]
#[derive(Debug, Clone)]
pub struct LinkMetrics {
    pub compile_time: std::time::Duration,
    pub runtime_time: std::time::Duration,
}

/// High-level linker that orchestrates compiler and runtime execution.
pub struct Linker {
    /// MLIR compiler — None when MLIR toolchain is not available (graph-direct mode).
    compiler: Option<Compiler>,
    runtime: RuntimeExecutor,
    pipeline_config: PipelineConfig,
    no_cache: bool,
}

impl Linker {
    /// Create a new linker instance with the provided configuration.
    ///
    /// The MLIR compiler is optional — if the toolchain is unavailable, the linker
    /// falls back to graph-direct execution (JSON → ExecutionDag, bypassing MLIR).
    pub async fn new(config: LinkerConfig) -> Result<Self, DriverError> {
        let compiler = match Compiler::with_opt_level(config.opt_level) {
            Ok(c) => {
                log_info!("driver", "MLIR compiler initialized");
                Some(c)
            }
            Err(e) => {
                log_info!(
                    "driver",
                    "MLIR unavailable ({}); using graph-direct mode",
                    e
                );
                None
            }
        };
        let runtime = RuntimeExecutor::new(&config).await?;
        let pipeline_config = config.pipeline_config;
        let no_cache = config.no_cache;

        Ok(Self {
            compiler,
            runtime,
            pipeline_config,
            no_cache,
        })
    }

    fn artifact_cache_enabled(&self) -> bool {
        !self.no_cache
            && std::env::var_os(apxm_env::APXM_CONFIG).is_none()
            && !compiler_project_config_exists()
            && self.pipeline_config.opt_level == OptimizationLevel::O2
            && self.pipeline_config.target == OptimizationTarget::Balanced
            && self.pipeline_config.verify
            && !self.pipeline_config.no_cse_llm
            && self.pipeline_config.profile_path.is_none()
            && self.pipeline_config.token_budget.is_none()
            && self.pipeline_config.compiler_config_path.is_none()
            && !self.pipeline_config.warn_unconsumed
            && self.pipeline_config.disable_passes.is_empty()
            && self.pipeline_config.pass_list_override.is_none()
    }

    /// Compile graph input into an executable artifact.
    ///
    /// For .air inputs, parses as MLIR text directly (bypasses graph loading + lowering).
    /// For JSON graph inputs, loads as `AirModule`, emits AIR text, and compiles via MLIR.
    ///
    /// When caching is enabled (the default), the graph JSON is hashed and
    /// looked up in `~/.cache/apxm/artifacts/`.  On a cache hit the
    /// compilation step is skipped entirely.
    pub fn compile_graph(&self, input: &Path) -> Result<Artifact, DriverError> {
        self.compile_graph_inner(input, None)
            .map(|(artifact, _)| artifact)
    }

    /// Compile graph input and return both artifact and compiler diagnostics.
    ///
    /// Same as `compile_graph` but also returns the per-pass diagnostics
    /// (serialized as JSON) when available. Cache hits return `None` diagnostics.
    pub fn compile_graph_with_diagnostics(
        &self,
        input: &Path,
    ) -> Result<(Artifact, Option<serde_json::Value>), DriverError> {
        self.compile_graph_inner(input, None)
    }

    fn compile_graph_inner(
        &self,
        input: &Path,
        python_tools_sidecar: Option<&[u8]>,
    ) -> Result<(Artifact, Option<serde_json::Value>), DriverError> {
        let Some(ref compiler) = self.compiler else {
            return Err(DriverError::Driver(
                "MLIR compiler required but is not available. Rebuild APXM with MLIR support."
                    .to_string(),
            ));
        };

        // For .air files, parse the MLIR text directly and run the optimization
        // pipeline with diagnostics so the unified --emit-metrics report includes
        // the compiler section even when execute() runs against a .py source
        // (which is lowered to a temp .air file before reaching the linker).
        let ext = input.extension().and_then(|ext| ext.to_str());
        if matches!(ext, Some("air")) {
            let air_text = std::fs::read_to_string(input)
                .map_err(|e| state_err(format!("Failed to read {}: {}", input.display(), e)))?;
            let config = self.pipeline_config.clone();
            let (module, diagnostics) =
                compiler.compile_air_with_config_and_diagnostics(&air_text, config)?;
            let diagnostics_json = Some(diagnostics.to_json());
            let manifest = python_tools_manifest(python_tools_sidecar)?;
            let mut artifact = module.generate_artifact_with_manifest(None, manifest.as_deref())?;
            add_python_tools_section(&mut artifact, python_tools_sidecar);

            let dag = artifact
                .entry_dag()
                .ok_or_else(|| state_err("Artifact contains no entry DAG"))?;
            if let Err(err) = dag.validate() {
                return Err(state_err(format!(
                    "Artifact DAG validation failed: {}",
                    err
                )));
            }
            return Ok((artifact, diagnostics_json));
        }

        let air_module = compiler.load_graph(input)?;

        // Try the artifact cache first.
        let hash = cache::graph_hash(&air_module).ok();

        let artifact_cache_enabled = self.artifact_cache_enabled();
        if artifact_cache_enabled
            && let Some(ref h) = hash
            && let Some(cached_bytes) = cache::load_cached(h)?
        {
            log_info!("driver", "cache hit for graph hash {}", h);
            let artifact =
                Artifact::from_bytes(&cached_bytes).map_err(|e| state_err(e.to_string()))?;
            return Ok((artifact, None));
        }

        let (module, diagnostics) = compiler
            .compile_graph_with_config_and_diagnostics(&air_module, self.pipeline_config.clone())?;
        let diagnostics_json = Some(diagnostics.to_json());
        let artifact_bytes = module.generate_artifact_bytes()?;

        // Store in cache for next time.
        if artifact_cache_enabled && let Some(ref h) = hash {
            let _ = cache::store_cached(h, &artifact_bytes);
        }

        let artifact =
            Artifact::from_bytes(&artifact_bytes).map_err(|e| state_err(e.to_string()))?;

        let dag = artifact
            .entry_dag()
            .ok_or_else(|| state_err("Artifact contains no entry DAG"))?;
        if let Err(err) = dag.validate() {
            return Err(state_err(format!(
                "Artifact DAG validation failed: {}",
                err
            )));
        }
        Ok((artifact, diagnostics_json))
    }

    /// Compile graph input and execute with entry arguments.
    pub async fn run_graph(
        &self,
        input: &Path,
        args: Vec<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<&Path>,
    ) -> Result<LinkResult, DriverError> {
        self.run_graph_with_python_tools_sidecar(input, args, event_emitter, session_dir, None)
            .await
    }

    /// Compile graph input and execute with an optional Python tools sidecar
    /// extracted by the CLI's Python frontend bridge.
    pub async fn run_graph_with_python_tools_sidecar(
        &self,
        input: &Path,
        args: Vec<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<&Path>,
        python_tools_sidecar: Option<Vec<u8>>,
    ) -> Result<LinkResult, DriverError> {
        log_info!("driver", "Compiling graph {}", input.display());
        #[cfg(feature = "metrics")]
        let compile_start = std::time::Instant::now();
        let (artifact, compiler_diagnostics) =
            self.compile_graph_inner(input, python_tools_sidecar.as_deref())?;
        #[cfg(feature = "metrics")]
        let compile_time = compile_start.elapsed();

        #[cfg(feature = "metrics")]
        let runtime_start = std::time::Instant::now();
        let execution = self
            .runtime
            .execute_artifact_with_emitter(
                artifact.clone(),
                args,
                event_emitter,
                session_dir.map(|dir| dir.to_string_lossy().to_string()),
            )
            .await?;
        #[cfg(feature = "metrics")]
        let runtime_time = runtime_start.elapsed();

        Ok(LinkResult {
            artifact,
            execution,
            #[cfg(feature = "metrics")]
            metrics: LinkMetrics {
                compile_time,
                runtime_time,
            },
            compiler_diagnostics,
        })
    }

    /// Get a reference to the runtime executor
    pub fn runtime_executor(&self) -> &RuntimeExecutor {
        &self.runtime
    }

    /// Gracefully shut down the runtime, closing all agent processes.
    ///
    /// This method should be called when execution completes to ensure
    /// all ACP sessions are properly terminated. Without this, agent
    /// processes may leak.
    pub fn shutdown(&self) {
        self.runtime.shutdown();
    }
}

fn python_tools_manifest(
    python_tools_sidecar: Option<&[u8]>,
) -> Result<Option<Vec<apxm_compiler::passes::PythonToolManifestEntry>>, DriverError> {
    python_tools_sidecar
        .map(|data| serde_json::from_slice(data).map_err(|e| state_err(e.to_string())))
        .transpose()
}

fn add_python_tools_section(artifact: &mut Artifact, python_tools_sidecar: Option<&[u8]>) {
    if let Some(data) = python_tools_sidecar {
        artifact.add_section(ArtifactSection {
            kind: apxm_runtime::python_tools::CAPABILITY_NAME.into(),
            data: data.to_vec(),
        });
    }
}
