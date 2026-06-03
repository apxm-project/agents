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
