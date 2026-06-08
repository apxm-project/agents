//! Runtime executor used by the driver to run compiled DAGs.

use std::sync::Arc;
use std::time::Duration;

use apxm_artifact::Artifact;
use apxm_core::types::execution::ExecutionDag;
use apxm_runtime::{
    ExecutionEventEmitter, LoopGuardMiddleware, OperationMiddleware, Runtime,
    RuntimeExecutionResult, TimeoutMiddleware,
};

use crate::{config::MiddlewareConfig, error::DriverError, hooks, linker::LinkerConfig};

mod llm;
use llm::configure_llm_registry;
mod capabilities;
use capabilities::configure_capability_registry;
pub mod sandbox;
use sandbox::configure_sandbox_registry;
mod inner_plan;
use apxm_core::utils::build::MlirEnvReport;
use apxm_runtime::NoOpLinker;
use inner_plan::CompilerInnerPlanLinker;
pub mod agents;
use agents::configure_agent_registry;
mod workflow_spawn;
use workflow_spawn::DriverWorkflowSpawner;

/// Runtime executor used by the driver to run compiled DAGs.
pub struct RuntimeExecutor {
    runtime: Arc<Runtime>,
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
}

impl RuntimeExecutor {
    pub async fn new(config: &LinkerConfig) -> Result<Self, DriverError> {
        let mut runtime = Runtime::new(config.runtime_config.clone())
            .await
            .map_err(DriverError::Runtime)?;

        configure_llm_registry(runtime.llm_registry(), &config.apxm_config).await?;
        configure_capability_registry(runtime.capability_system_arc(), &config.apxm_config)?;

        // Initialize ModelRouter after LLM backends are registered.
        // Populate operation_policies from config.chat.routing.operation_routes so
        // the ModelRouter correctly routes ASK/THINK/etc. to their configured backends.
        let router_operation_policies = config
            .apxm_config
            .chat
            .routing
            .operation_routes
            .iter()
            .filter_map(|(op_str, route)| {
                let operation = op_str.parse::<apxm_core::types::AISOperationType>().ok()?;
                Some(apxm_runtime::OperationPolicy {
                    operation,
                    model: route.model.clone(),
                    backend: route.backend.clone(),
                    target: apxm_runtime::RoutingTarget::Balanced,
                })
            })
            .collect::<Vec<_>>();

        let mut router_config = apxm_runtime::ModelRouterConfig::default();
        router_config.operation_policies = router_operation_policies;

        runtime
            .init_model_router(router_config)
            .map_err(DriverError::Runtime)?;

        let sandbox_registry = configure_sandbox_registry();
        runtime.set_sandbox_registry(std::sync::Arc::clone(&sandbox_registry));

        configure_agent_registry(
            runtime.process_table(),
            runtime.capability_system_arc(),
            sandbox_registry,
        )
        .await?;

        runtime.set_instruction_config(config.apxm_config.instruction.clone());
        runtime.set_middlewares(build_middlewares(&config.apxm_config.middlewares));

        // Use CompilerInnerPlanLinker when MLIR is available, otherwise fall back to
        // NoOpLinker (graph-direct mode). Mirrors how Linker handles MLIR unavailability.
        let report = MlirEnvReport::detect();
        report.apply_env();
        if report.is_ready() {
            match CompilerInnerPlanLinker::new(config.pipeline_config.clone()) {
                Ok(linker) => {
                    apxm_core::log_info!("driver", "MLIR inner-plan linker initialized");
                    runtime.set_inner_plan_linker(Arc::new(linker));
                }
                Err(e) => {
                    apxm_core::log_info!(
                        "driver",
                        "MLIR inner-plan linker init failed ({}); using NoOp",
                        e
                    );
                    runtime.set_inner_plan_linker(Arc::new(NoOpLinker));
                }
            }
        } else {
            apxm_core::log_info!(
                "driver",
                "MLIR not available; using NoOp inner-plan linker (graph-direct mode)"
            );
            runtime.set_inner_plan_linker(Arc::new(NoOpLinker));
        }

        let configured_emitter = hooks::emitter_from_config(&config.apxm_config.hooks);
        let workflow_spawner = Arc::new(DriverWorkflowSpawner::new(
            configured_emitter.as_ref().map(Arc::clone),
        ));
        runtime.set_workflow_spawner(workflow_spawner.clone());
        let runtime = Arc::new(runtime);
        workflow_spawner.attach_runtime(Arc::downgrade(&runtime));

        Ok(Self {
            runtime,
            configured_emitter,
        })
    }

    pub async fn execute(&self, dag: ExecutionDag) -> Result<RuntimeExecutionResult, DriverError> {
        self.runtime
            .execute_with_event_emitter(dag, self.compose_emitter(None))
            .await
            .map_err(DriverError::Runtime)
    }

    /// Execute an artifact with @entry flow requirement.
    ///
    /// This method will return an error if the artifact doesn't have an @entry flow.
    pub async fn execute_artifact_auto(
        &self,
        artifact: Artifact,
    ) -> Result<RuntimeExecutionResult, DriverError> {
        self.runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                vec![],
                None,
                self.compose_emitter(None),
                None,
            )
            .await
            .map_err(DriverError::Runtime)
    }

    /// Execute an artifact with provided arguments for entry flow parameters.
    ///
    /// Validates that the number of arguments matches the entry flow's parameter count.
    pub async fn execute_artifact_with_args(
        &self,
        artifact: Artifact,
        args: Vec<String>,
    ) -> Result<RuntimeExecutionResult, DriverError> {
        self.runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                args,
                None,
                self.compose_emitter(None),
                None,
            )
            .await
            .map_err(DriverError::Runtime)
    }

    /// Execute an artifact with arguments and an optional event emitter.
    pub async fn execute_artifact_with_emitter(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
    ) -> Result<RuntimeExecutionResult, DriverError> {
        self.runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                args,
                None,
                self.compose_emitter(emitter),
                session_dir,
            )
            .await
            .map_err(DriverError::Runtime)
    }

    fn compose_emitter(
        &self,
        emitter: Option<Arc<dyn ExecutionEventEmitter>>,
    ) -> Option<Arc<dyn ExecutionEventEmitter>> {
        hooks::compose_emitters(emitter, self.configured_emitter.as_ref().map(Arc::clone))
    }

    /// Get the LLM registry from the runtime
    pub fn llm_registry(&self) -> Arc<apxm_backends::LLMRegistry> {
        self.runtime.llm_registry_arc()
    }

    /// Get list of available capability names
    ///
    /// Returns the names of all capabilities registered in the runtime.
    /// Useful for validation, UI display, and constraining LLM generation.
    pub fn capability_names(&self) -> Vec<String> {
        self.runtime.capability_system().list_capability_names()
    }

    /// Get capability system reference (for advanced usage)
    ///
    /// Provides direct access to the capability system for
    /// advanced operations like capability inspection or registration.
    pub fn capability_system(&self) -> &apxm_runtime::capability::CapabilitySystem {
        self.runtime.capability_system()
    }

    /// Get memory system reference
    ///
    /// Provides access to the memory system for integration with session output
    pub fn memory_system(&self) -> Arc<apxm_runtime::memory::MemorySystem> {
        self.runtime.memory_system_arc()
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

/// Install the driver workflow-spawn bridge on a server-owned runtime.
///
/// APXM server constructs `Runtime` directly instead of going through
/// [`RuntimeExecutor`], but server/MCP graph execution still needs the same
/// `WORKFLOW_SPAWN` host bridge as the CLI. The bridge keeps only a weak handle
/// back to the runtime to avoid a reference cycle.
pub fn install_workflow_spawner(
    runtime: &mut Arc<Runtime>,
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
) -> Result<(), apxm_runtime::RuntimeError> {
    let installed = {
        let runtime = Arc::get_mut(runtime).ok_or_else(|| {
            apxm_runtime::RuntimeError::State(
                "cannot install workflow spawner after runtime has been shared".to_string(),
            )
        })?;
        install_workflow_spawner_unattached(runtime, configured_emitter)
    };
    installed.attach_runtime(runtime);
    Ok(())
}

pub struct InstalledWorkflowSpawner {
    spawner: Arc<DriverWorkflowSpawner>,
}

impl InstalledWorkflowSpawner {
    pub fn attach_runtime(&self, runtime: &Arc<Runtime>) {
        self.spawner.attach_runtime(Arc::downgrade(runtime));
    }
}

pub fn install_workflow_spawner_unattached(
    runtime: &mut Runtime,
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
) -> InstalledWorkflowSpawner {
    let workflow_spawner = Arc::new(DriverWorkflowSpawner::new(configured_emitter));
    runtime.set_workflow_spawner(workflow_spawner.clone());
    InstalledWorkflowSpawner {
        spawner: workflow_spawner,
    }
}

fn build_middlewares(configs: &[MiddlewareConfig]) -> Vec<Arc<dyn OperationMiddleware>> {
    configs
        .iter()
        .map(|config| match config {
            MiddlewareConfig::Timeout { default_timeout_ms } => Arc::new(TimeoutMiddleware::new(
                default_timeout_ms.map(Duration::from_millis),
            ))
                as Arc<dyn OperationMiddleware>,
            MiddlewareConfig::LoopGuard { max_repeats } => {
                Arc::new(LoopGuardMiddleware::new(*max_repeats)) as Arc<dyn OperationMiddleware>
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_artifact::Artifact;
    use apxm_compiler::{Context, Pipeline};
    use apxm_core::error::RuntimeError;
    use apxm_core::types::Value;
    use apxm_runtime::capability::executor::CapabilityExecutor;
    use apxm_runtime::capability::metadata::CapabilityMetadata;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    const CONST_GRAPH: &str = r#"module {
  func.func @const_graph() -> !ais.token attributes {ais.entry} {
    %value = ais.const_str "ok" : !ais.token
    func.return %value : !ais.token
  }
}
"#;

    #[tokio::test]
    async fn installed_workflow_spawner_executes_workflow_spawn_graph() {
        let temp = tempfile::tempdir().expect("tempdir");
        let graph_path = temp.path().join("child.air");
        let workflow_path = temp.path().join("child.apxmw");
        std::fs::write(&graph_path, CONST_GRAPH).expect("write graph");
        std::fs::write(
            &workflow_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "name": "child",
                "graphs": [
                    {"id": "const_step", "path": "child.air"}
                ],
                "output": "{{const_step.output}}"
            }))
            .expect("serialize workflow"),
        )
        .expect("write workflow");

        let context = Context::new().expect("compiler context");
        let pipeline = Pipeline::new(&context);
        let workflow_target = air_string(workflow_path.to_string_lossy().as_ref());
        let air = format!(
            r#"module {{
  func.func @server_workflow_spawn() -> !ais.token attributes {{ais.entry}} {{
    %child = ais.workflow_spawn "workflow_path" "{workflow_target}" {{await_result = true}} : !ais.token
    func.return %child : !ais.token
  }}
}}
"#
        );
        let module = pipeline.compile(&air).expect("compile workflow spawn air");
        let artifact = Artifact::from_bytes(
            &module
                .generate_artifact_bytes()
                .expect("generate artifact bytes"),
        )
        .expect("decode artifact");

        let runtime = Runtime::new(apxm_runtime::RuntimeConfig::in_memory())
            .await
            .expect("runtime");
        let mut runtime = Arc::new(runtime);
        install_workflow_spawner(&mut runtime, None).expect("install workflow spawner");

        let parent_session_dir = temp.path().join("sessions").join("parent");
        std::fs::create_dir_all(&parent_session_dir).expect("create parent session");
        let execution = runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                Vec::new(),
                Some("parent".to_string()),
                None,
                Some(parent_session_dir.to_string_lossy().to_string()),
            )
            .await
            .expect("execute workflow spawn");
        let output = execution
            .results
            .values()
            .last()
            .expect("workflow spawn result")
            .to_json()
            .expect("result json");

        assert_eq!(output["result"], "ok");
        let child_session_dir = output["session_dir"].as_str().expect("session_dir");
        assert!(
            child_session_dir.contains("workflow-child-"),
            "child session dir: {child_session_dir}"
        );
        assert!(
            std::path::Path::new(child_session_dir)
                .join("results.json")
                .is_file()
        );
    }

    #[tokio::test]
    async fn installed_workflow_spawner_runs_independent_workflow_phase_steps_in_parallel() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("left.air"),
            tool_graph("fixture_phase_left"),
        )
        .expect("write left graph");
        std::fs::write(
            temp.path().join("right.air"),
            tool_graph("fixture_phase_right"),
        )
        .expect("write right graph");
        let workflow_path = temp.path().join("parallel.apxmw");
        std::fs::write(
            &workflow_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "name": "parallel_phase",
                "graphs": [
                    {"id": "left", "path": "left.air"},
                    {"id": "right", "path": "right.air"}
                ],
                "output": "{{left.output}}+{{right.output}}"
            }))
            .expect("serialize workflow"),
        )
        .expect("write workflow");

        let context = Context::new().expect("compiler context");
        let pipeline = Pipeline::new(&context);
        let workflow_target = air_string(workflow_path.to_string_lossy().as_ref());
        let air = format!(
            r#"module {{
  func.func @parallel_workflow_spawn() -> !ais.token attributes {{ais.entry}} {{
    %child = ais.workflow_spawn "workflow_path" "{workflow_target}" {{await_result = true}} : !ais.token
    func.return %child : !ais.token
  }}
}}
"#
        );
        let module = pipeline.compile(&air).expect("compile workflow spawn air");
        let artifact = Artifact::from_bytes(
            &module
                .generate_artifact_bytes()
                .expect("generate artifact bytes"),
        )
        .expect("decode artifact");

        let runtime = Runtime::new(apxm_runtime::RuntimeConfig::in_memory())
            .await
            .expect("runtime");
        let mut runtime = Arc::new(runtime);
        let probe = Arc::new(ParallelProbe::new(2));
        runtime
            .capability_system()
            .register(Arc::new(BarrierCapability::new(
                "fixture_phase_left",
                "left",
                Arc::clone(&probe),
            )))
            .expect("register left capability");
        runtime
            .capability_system()
            .register(Arc::new(BarrierCapability::new(
                "fixture_phase_right",
                "right",
                Arc::clone(&probe),
            )))
            .expect("register right capability");
        install_workflow_spawner(&mut runtime, None).expect("install workflow spawner");

        let parent_session_dir = temp.path().join("sessions").join("parent");
        std::fs::create_dir_all(&parent_session_dir).expect("create parent session");
        let execution = runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                Vec::new(),
                Some("parent".to_string()),
                None,
                Some(parent_session_dir.to_string_lossy().to_string()),
            )
            .await
            .expect("execute workflow spawn");
        let output = execution
            .results
            .values()
            .last()
            .expect("workflow spawn result")
            .to_json()
            .expect("result json");

        assert_eq!(output["result"], "left+right");
        assert!(
            probe.max_seen.load(Ordering::SeqCst) >= 2,
            "independent workflow steps should overlap in one execution phase"
        );
    }

    fn air_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    fn tool_graph(capability: &str) -> String {
        format!(
            r#"module {{
  func.func @tool_graph() -> !ais.token attributes {{ais.entry}} {{
    %reg = ais.register_capability "{capability}" {{description = "fixture phase probe"}} : !ais.token
    %tool = ais.inv_tool "{capability}" ("{{}}") [%reg : !ais.token] : !ais.token
    func.return %tool : !ais.token
  }}
}}
"#
        )
    }

    struct ParallelProbe {
        expected: usize,
        current: AtomicUsize,
        max_seen: AtomicUsize,
        notify: Notify,
    }

    impl ParallelProbe {
        fn new(expected: usize) -> Self {
            Self {
                expected,
                current: AtomicUsize::new(0),
                max_seen: AtomicUsize::new(0),
                notify: Notify::new(),
            }
        }

        async fn enter(&self) {
            let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_seen.fetch_max(current, Ordering::SeqCst);
            if current >= self.expected {
                self.notify.notify_waiters();
            }
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(250),
                self.notify.notified(),
            )
            .await;
        }

        fn exit(&self) {
            self.current.fetch_sub(1, Ordering::SeqCst);
        }
    }

    struct BarrierCapability {
        metadata: CapabilityMetadata,
        label: String,
        probe: Arc<ParallelProbe>,
    }

    impl BarrierCapability {
        fn new(name: &str, label: &str, probe: Arc<ParallelProbe>) -> Self {
            Self {
                metadata: CapabilityMetadata::new(
                    name,
                    "Fixture capability that records concurrent workflow phase execution",
                    serde_json::json!({ "type": "object", "properties": {} }),
                )
                .with_returns("string")
                .with_read_only(),
                label: label.to_string(),
                probe,
            }
        }
    }

    #[async_trait]
    impl CapabilityExecutor for BarrierCapability {
        async fn execute(&self, _args: HashMap<String, Value>) -> Result<Value, RuntimeError> {
            self.probe.enter().await;
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            self.probe.exit();
            Ok(Value::String(self.label.clone()))
        }

        fn metadata(&self) -> &CapabilityMetadata {
            &self.metadata
        }
    }
}
