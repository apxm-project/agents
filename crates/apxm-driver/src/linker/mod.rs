//! High-level linker that orchestrates compiler and runtime execution.

use std::path::Path;
use std::sync::Arc;

use apxm_artifact::Artifact;
use apxm_core::error::runtime::RuntimeError;
use apxm_core::log_info;
use apxm_core::types::OptimizationLevel;
use apxm_runtime::{ExecutionEventEmitter, RuntimeConfig, RuntimeExecutionResult};

use apxm_artifact::ArtifactMetadata;
use apxm_graph::ApxmGraph;

use crate::{
    cache, compiler::Compiler, config::ApXmConfig, error::DriverError, runtime::RuntimeExecutor,
};

fn state_err(msg: impl Into<String>) -> DriverError {
    DriverError::Runtime(RuntimeError::State(msg.into()))
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

    /// When `true`, skip the artifact cache entirely.
    pub no_cache: bool,
}

impl LinkerConfig {
    /// Create a configuration from an `ApXmConfig` instance.
    pub fn from_apxm_config(apxm_config: ApXmConfig) -> Self {
        Self {
            apxm_config,
            runtime_config: RuntimeConfig::default(),
            opt_level: OptimizationLevel::O1,
            no_cache: false,
        }
    }

    /// Set the optimization level.
    pub fn with_opt_level(mut self, opt_level: OptimizationLevel) -> Self {
        self.opt_level = opt_level;
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
        let no_cache = config.no_cache;

        Ok(Self {
            compiler,
            runtime,
            no_cache,
        })
    }

    /// Compile a JSON graph directly to an Artifact without going through MLIR.
    ///
    /// Compile an in-memory `ApxmGraph` directly to an executable artifact.
    ///
    /// This is the primary integration point for AgentMate and any other frontend
    /// that constructs graphs programmatically (Rust `WorkflowBuilder`, Python
    /// `FlowModule`, etc.). No file I/O required — the graph is compiled entirely
    /// in memory via the pure-Rust `to_execution_dag()` path.
    ///
    pub fn compile_from_graph(
        &self,
        graph: ApxmGraph,
        name: Option<String>,
    ) -> Result<Artifact, DriverError> {
        let hash = cache::graph_hash(&graph).ok();
        if !self.no_cache
            && let Some(ref h) = hash
            && let Some(cached_bytes) = cache::load_cached(h)?
        {
            log_info!("driver", "cache hit (graph) for graph hash {}", h);
            return Artifact::from_bytes(&cached_bytes).map_err(|e| state_err(e.to_string()));
        }

        let dag = graph
            .to_execution_dag()
            .map_err(|e| DriverError::Driver(format!("Graph lowering error: {e}")))?;

        let metadata = ArtifactMetadata::new(
            name.or_else(|| Some(graph.name.clone())),
            env!("CARGO_PKG_VERSION"),
        );
        let artifact = Artifact::new(metadata, vec![dag]);

        if !self.no_cache
            && let Some(ref h) = hash
        {
            if let Ok(artifact_bytes) = artifact.to_bytes() {
                let _ = cache::store_cached(h, &artifact_bytes);
            }
        }

        log_info!("driver", "graph-direct compilation complete");
        Ok(artifact)
    }

    /// Compile graph input into an executable artifact.
    ///
    /// For .air inputs, parses as MLIR text directly (bypasses graph loading + lowering).
    /// For JSON graph inputs, uses the graph-direct path (pure Rust, no MLIR) when
    /// the MLIR compiler is unavailable. Falls back to full MLIR compilation otherwise.
    ///
    /// When caching is enabled (the default), the graph JSON is hashed and
    /// looked up in `~/.cache/apxm/artifacts/`.  On a cache hit the
    /// compilation step is skipped entirely.
    pub fn compile_graph(&self, input: &Path) -> Result<Artifact, DriverError> {
        let Some(ref compiler) = self.compiler else {
            return Err(DriverError::Driver(
                "MLIR compiler required for non-JSON inputs but is not available. Run `dekk apxm build` to rebuild with MLIR support.".to_string(),
            ));
        };

        // For .air files, use compiler.compile() which parses .air → ApxmGraph → MLIR
        let ext = input.extension().and_then(|ext| ext.to_str());
        if matches!(ext, Some("air")) {
            let module = compiler.compile(input)?;
            let artifact_bytes = module.generate_artifact_bytes()?;
            let artifact =
                Artifact::from_bytes(&artifact_bytes).map_err(|e| state_err(e.to_string()))?;

            let dag = artifact
                .dag()
                .ok_or_else(|| state_err("Artifact contains no DAGs"))?;
            if let Err(err) = dag.validate() {
                return Err(state_err(format!(
                    "Artifact DAG validation failed: {}",
                    err
                )));
            }
            return Ok(artifact);
        }

        let graph = compiler.load_graph(input)?;

        // Try the artifact cache first.
        let hash = cache::graph_hash(&graph).ok();

        if !self.no_cache
            && let Some(ref h) = hash
            && let Some(cached_bytes) = cache::load_cached(h)?
        {
            log_info!("driver", "cache hit for graph hash {}", h);
            let artifact =
                Artifact::from_bytes(&cached_bytes).map_err(|e| state_err(e.to_string()))?;
            return Ok(artifact);
        }

        let module = compiler.compile_graph(&graph)?;
        let artifact_bytes = module.generate_artifact_bytes()?;

        // Store in cache for next time.
        if !self.no_cache
            && let Some(ref h) = hash
        {
            let _ = cache::store_cached(h, &artifact_bytes);
        }

        let artifact =
            Artifact::from_bytes(&artifact_bytes).map_err(|e| state_err(e.to_string()))?;

        let dag = artifact
            .dag()
            .ok_or_else(|| state_err("Artifact contains no DAGs"))?;
        if let Err(err) = dag.validate() {
            return Err(state_err(format!(
                "Artifact DAG validation failed: {}",
                err
            )));
        }
        Ok(artifact)
    }

    /// Execute an in-memory `ApxmGraph` directly.
    ///
    /// Primary entry point for AgentMate and other programmatic frontends.
    /// Combines `compile_from_graph` + runtime execution in one call.
    /// No files written — fully in-memory pipeline.
    ///
    /// ```rust,ignore
    /// // AgentMate usage:
    /// let graph = WorkflowBuilder::new("research").ask("Research {0}").build();
    /// let result = linker.run_from_graph(graph, vec!["quantum".into()], None, None).await?;
    /// ```
    pub async fn run_from_graph(
        &self,
        graph: ApxmGraph,
        args: Vec<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<&Path>,
    ) -> Result<LinkResult, DriverError> {
        let name = Some(graph.name.clone());
        #[cfg(feature = "metrics")]
        let compile_start = std::time::Instant::now();
        let artifact = self.compile_from_graph(graph, name)?;
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
        })
    }

    /// Compile graph input and execute with entry arguments.
    pub async fn run_graph(
        &self,
        input: &Path,
        args: Vec<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<&Path>,
    ) -> Result<LinkResult, DriverError> {
        log_info!("driver", "Compiling graph {}", input.display());
        #[cfg(feature = "metrics")]
        let compile_start = std::time::Instant::now();
        let artifact = self.compile_graph(input)?;
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
        })
    }
}
