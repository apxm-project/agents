//! High-level linker that coordinates compiler and runtime execution.

use std::path::Path;
use std::sync::Arc;

use apxm_artifact::{Artifact, ArtifactSection};
use apxm_core::constants::extensions;
use apxm_core::error::runtime::RuntimeError;
use apxm_core::log_info;
use apxm_core::types::{OptimizationLevel, PipelineConfig};
use apxm_runtime::{ExecutionEventEmitter, RuntimeConfig, RuntimeExecutionResult};

use crate::{compiler::Compiler, config::ApXmConfig, error::DriverError, runtime::RuntimeExecutor};

fn state_err(msg: impl Into<String>) -> DriverError {
    DriverError::Runtime(RuntimeError::State(msg.into()))
}

/// Linker configuration that drives compiler and runtime coordination.
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
        self.runtime_config.optimization_target = pipeline_config.target;
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

#[derive(Debug, Clone, Copy, Default)]
struct FrontendHandlerSidecars<'a> {
    python_tools: Option<&'a [u8]>,
    typescript_tools: Option<&'a [u8]>,
}

impl<'a> FrontendHandlerSidecars<'a> {
    const fn new(python_tools: Option<&'a [u8]>, typescript_tools: Option<&'a [u8]>) -> Self {
        Self {
            python_tools,
            typescript_tools,
        }
    }

    fn python_manifest(
        self,
    ) -> Result<Option<Vec<apxm_compiler::passes::PythonCapabilityManifestEntry>>, DriverError>
    {
        self.python_tools
            .map(|data| serde_json::from_slice(data).map_err(|e| state_err(e.to_string())))
            .transpose()
    }

    /// Re-attach the captured Python/TypeScript tool sidecars to the
    /// compiled artifact as `python_tools`/`typescript_tools` sections, ONLY
    /// when the operator has explicitly trusted script artifacts
    /// (`apxm_runtime::script_admission::script_artifacts_trusted`) — the
    /// same trust+sandbox gate the Server enforces
    /// (`python_artifacts_trusted`) and the Runtime enforces at dispatch
    /// (`execute_artifact_inner`). Before this gate the driver attached both
    /// sidecars unconditionally regardless of any env var; now the CLI's
    /// source-compile path (`apxm execute`) fails closed identically to the
    /// Server's raw execute route. No-op (both sidecars dropped) when
    /// untrusted — the compiled artifact then carries no script section and
    /// the Runtime's own admission guard also rejects, so a caller cannot
    /// bypass this by skipping the driver.
    fn append_to_artifact(self, artifact: &mut Artifact) {
        if !apxm_runtime::script_admission::script_artifacts_trusted() {
            return;
        }
        let sidecars = [
            HandlerSidecar::new(
                apxm_runtime::python_tools::CAPABILITY_NAME,
                self.python_tools,
            ),
            HandlerSidecar::new(
                apxm_runtime::typescript_tools::CAPABILITY_NAME,
                self.typescript_tools,
            ),
        ];
        add_artifact_sidecars(
            artifact,
            sidecars
                .iter()
                .map(|sidecar| sidecar as &dyn ArtifactSidecar),
        );
    }
}

trait ArtifactSidecar {
    fn section_kind(&self) -> &'static str;
    fn data(&self) -> Option<&[u8]>;

    fn append_to(&self, artifact: &mut Artifact) {
        if let Some(data) = self.data() {
            artifact.add_section(ArtifactSection {
                kind: self.section_kind().into(),
                data: data.to_vec(),
            });
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct HandlerSidecar<'a> {
    section_kind: &'static str,
    data: Option<&'a [u8]>,
}

impl<'a> HandlerSidecar<'a> {
    const fn new(section_kind: &'static str, data: Option<&'a [u8]>) -> Self {
        Self { section_kind, data }
    }
}

impl ArtifactSidecar for HandlerSidecar<'_> {
    fn section_kind(&self) -> &'static str {
        self.section_kind
    }

    fn data(&self) -> Option<&[u8]> {
        self.data
    }
}

/// High-level linker that coordinates compiler and runtime execution.
pub struct Linker {
    /// MLIR compiler required for AIR-to-artifact compilation.
    compiler: Option<Compiler>,
    runtime: RuntimeExecutor,
    pipeline_config: PipelineConfig,
}

impl Linker {
    /// Create a new linker instance with the provided configuration.
    ///
    /// The MLIR compiler is required. The linker does not execute source graphs
    /// directly; graph source must compile to `.apxmobj` before runtime execution.
    pub async fn new(config: LinkerConfig) -> Result<Self, DriverError> {
        let compiler = match Compiler::with_opt_level(config.opt_level) {
            Ok(c) => {
                log_info!("driver", "MLIR compiler initialized");
                Some(c)
            }
            Err(e) => {
                log_info!("driver", "MLIR unavailable ({})", e);
                None
            }
        };
        let runtime = RuntimeExecutor::new(&config).await?;
        let pipeline_config = config.pipeline_config;

        Ok(Self {
            compiler,
            runtime,
            pipeline_config,
        })
    }

    /// Compile canonical AIR graph source into an executable artifact.
    pub fn compile_graph(&self, input: &Path) -> Result<Artifact, DriverError> {
        self.compile_graph_inner(input, FrontendHandlerSidecars::default())
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
        self.compile_graph_inner(input, FrontendHandlerSidecars::default())
    }

    fn compile_graph_inner(
        &self,
        input: &Path,
        handler_sidecars: FrontendHandlerSidecars<'_>,
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
        if matches!(ext, Some(extensions::AIR)) {
            let air_text = std::fs::read_to_string(input)
                .map_err(|e| state_err(format!("Failed to read {}: {}", input.display(), e)))?;
            let config = self.pipeline_config.clone();
            let (module, diagnostics) =
                compiler.compile_air_with_config_and_diagnostics(&air_text, config)?;
            let diagnostics_json = Some(diagnostics.to_json());
            let manifest = handler_sidecars.python_manifest()?;
            let mut artifact = module.generate_artifact_with_manifest(None, manifest.as_deref())?;
            handler_sidecars.append_to_artifact(&mut artifact);

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
        Err(DriverError::Driver(format!(
            "Unsupported graph source '{}'. Runtime execution requires canonical .air source or a precompiled .apxmobj artifact.",
            input.display()
        )))
    }

    /// Compile graph input and execute with entry arguments.
    pub async fn run_graph(
        &self,
        input: &Path,
        args: Vec<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<&Path>,
    ) -> Result<LinkResult, DriverError> {
        self.run_graph_with_python_tools_sidecar(
            input,
            args,
            event_emitter,
            session_dir,
            None,
            None,
        )
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
        typescript_tools_sidecar: Option<Vec<u8>>,
    ) -> Result<LinkResult, DriverError> {
        log_info!("driver", "Compiling graph {}", input.display());
        #[cfg(feature = "metrics")]
        let compile_start = std::time::Instant::now();
        let handler_sidecars = FrontendHandlerSidecars::new(
            python_tools_sidecar.as_deref(),
            typescript_tools_sidecar.as_deref(),
        );
        let (artifact, compiler_diagnostics) = self.compile_graph_inner(input, handler_sidecars)?;
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

fn add_artifact_sidecars<'a>(
    artifact: &mut Artifact,
    sidecars: impl IntoIterator<Item = &'a dyn ArtifactSidecar>,
) {
    for sidecar in sidecars {
        sidecar.append_to(artifact);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_artifact::ArtifactMetadata;
    use std::sync::Mutex;

    // Env vars are process-global; serialize tests that touch them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn set_vars(trust: bool, sandbox: bool) {
        #[allow(unsafe_code)]
        unsafe {
            if trust {
                std::env::set_var("APXM_TRUST_PYTHON_ARTIFACTS", "1");
            } else {
                std::env::remove_var("APXM_TRUST_PYTHON_ARTIFACTS");
            }
            if sandbox {
                std::env::set_var("APXM_SANDBOX_PYTHON", "1");
            } else {
                std::env::remove_var("APXM_SANDBOX_PYTHON");
            }
        }
    }

    struct EnvGuard;

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            set_vars(false, false);
        }
    }

    fn empty_artifact() -> Artifact {
        Artifact::new(
            ArtifactMetadata::new(Some("test".to_string()), "test"),
            Vec::new(),
        )
    }

    fn has_section(artifact: &Artifact, kind: &str) -> bool {
        artifact.sections().iter().any(|s| s.kind == kind)
    }

    /// Environment matrix for script-artifact admission: the
    /// driver's `append_to_artifact` must not attach EITHER sidecar unless
    /// the operator has asserted BOTH trust and sandbox — before this gate
    /// the driver attached both sidecars unconditionally regardless of any
    /// env var, so the CLI admitted and ran both languages with no policy at
    /// all. Trust-only and sandbox-only must fail closed identically to no
    /// vars.
    #[test]
    fn append_to_artifact_env_matrix_admits_only_when_fully_trusted() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;

        for (trust, sandbox, expect_attached) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, true),
        ] {
            set_vars(trust, sandbox);
            let sidecars =
                FrontendHandlerSidecars::new(Some(b"[]".as_slice()), Some(b"[]".as_slice()));
            let mut artifact = empty_artifact();
            sidecars.append_to_artifact(&mut artifact);

            assert_eq!(
                has_section(&artifact, apxm_runtime::python_tools::CAPABILITY_NAME),
                expect_attached,
                "trust={trust} sandbox={sandbox}: python_tools section attach mismatch"
            );
            assert_eq!(
                has_section(&artifact, apxm_runtime::typescript_tools::CAPABILITY_NAME),
                expect_attached,
                "trust={trust} sandbox={sandbox}: typescript_tools section attach mismatch"
            );
        }
    }

    /// Recovery: attaching under no vars is not sticky — the identical
    /// sidecar pair attaches once both vars are set, proving the driver's
    /// gate is a pure function of env state.
    #[test]
    fn append_to_artifact_recovers_after_trust_and_sandbox_are_set() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;

        set_vars(false, false);
        let sidecars = FrontendHandlerSidecars::new(Some(b"[]".as_slice()), Some(b"[]".as_slice()));

        let mut untrusted_artifact = empty_artifact();
        sidecars.append_to_artifact(&mut untrusted_artifact);
        assert!(
            untrusted_artifact.sections().is_empty(),
            "no sidecar should attach without trust+sandbox"
        );

        set_vars(true, true);
        let mut trusted_artifact = empty_artifact();
        sidecars.append_to_artifact(&mut trusted_artifact);
        assert_eq!(
            trusted_artifact.sections().len(),
            2,
            "both sidecars should attach for the identical pair once trusted+sandboxed"
        );
    }
}
